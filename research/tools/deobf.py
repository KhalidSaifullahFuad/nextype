#!/usr/bin/env python3
# Scratch analysis helper: emulate Cotypist's compile-time string-obfuscation decoders statically.
# Interprets llvm-objdump text of each decoder function; helper calls are modelled in Python.
# Read-only on the target binary; writes decoded strings to an output TSV.
import re, struct, subprocess, sys, pickle, bisect

B = '/Applications/Cotypist.app/Contents/MacOS/Cotypist'
OBJ = '/Library/Developer/CommandLineTools/usr/bin/llvm-objdump'
data = open(B, 'rb').read()
WORK = sys.argv[1]
D = pickle.load(open(WORK + '/scan.pkl', 'rb'))
fset = D['fstarts']; stubs = D['stubs']
decoders = pickle.load(open(WORK + '/decoders.pkl', 'rb'))

# segments for initial memory image
segs = []
ncmds = struct.unpack_from('<I', data, 16)[0]; o = 32
for _ in range(ncmds):
    cmd, size = struct.unpack_from('<II', data, o)
    if cmd == 0x19:
        vm, vs, fo, fs = struct.unpack_from('<QQQQ', data, o + 24)
        segs.append((vm, vs, fo, fs))
    o += size

M64 = (1 << 64) - 1
class Mem:
    def __init__(self):
        self.pages = {}
    def page(self, a):
        p = a >> 12
        pg = self.pages.get(p)
        if pg is None:
            pg = bytearray(4096)
            base = p << 12
            for vm, vs, fo, fs in segs:
                if vm <= base < vm + vs:
                    off = base - vm
                    if off < fs:
                        chunk = data[fo + off: fo + min(fs, off + 4096)]
                        pg[:len(chunk)] = chunk
            self.pages[p] = pg
        return pg
    def read(self, a, n):
        out = bytearray()
        while n:
            pg = self.page(a); i = a & 0xFFF; k = min(n, 4096 - i)
            out += pg[i:i + k]; a += k; n -= k
        return bytes(out)
    def write(self, a, bs):
        for b in bs:
            self.page(a)[a & 0xFFF] = b; a += 1
    def r(self, a, n): return int.from_bytes(self.read(a, n), 'little')
    def w(self, a, v, n): self.write(a, (v & ((1 << (8 * n)) - 1)).to_bytes(n, 'little'))

class Stop(Exception): pass
class Unsupported(Exception): pass

def disasm(start, end):
    out = subprocess.run([OBJ, '-d', '--no-show-raw-insn', '--start-address=0x%x' % start,
                          '--stop-address=0x%x' % end, B], capture_output=True, text=True).stdout
    ins = {}
    for line in out.splitlines():
        m = re.match(r'\s*([0-9a-f]+):\s+(\S+)\s*(.*)$', line)
        if not m: continue
        a = int(m.group(1), 16); mn = m.group(2); ops = m.group(3).split(';')[0].strip()
        ops = re.sub(r'<[^>]*>', '', ops).strip()
        ins[a] = (mn, ops)
    return ins

COND = {
    'eq': lambda n, z, c, v: z, 'ne': lambda n, z, c, v: not z,
    'hs': lambda n, z, c, v: c, 'cs': lambda n, z, c, v: c, 'lo': lambda n, z, c, v: not c, 'cc': lambda n, z, c, v: not c,
    'mi': lambda n, z, c, v: n, 'pl': lambda n, z, c, v: not n, 'vs': lambda n, z, c, v: v, 'vc': lambda n, z, c, v: not v,
    'hi': lambda n, z, c, v: c and not z, 'ls': lambda n, z, c, v: not (c and not z),
    'ge': lambda n, z, c, v: n == v, 'lt': lambda n, z, c, v: n != v,
    'gt': lambda n, z, c, v: (not z) and n == v, 'le': lambda n, z, c, v: z or n != v, 'al': lambda n, z, c, v: True,
}

def sx(v, bits):
    v &= (1 << bits) - 1
    return v - (1 << bits) if v >> (bits - 1) else v

class CPU:
    def __init__(self, mem):
        self.m = mem; self.x = [0] * 32; self.sp = 0x7ff000000000; self.v = [0] * 32
        self.n = self.z = self.c = self.vf = False
        self.heap = 0x600000000000
        self.datas = {}; self.dh = 1
        self.result = None
    def alloc(self, n):
        a = self.heap; self.heap += (n + 0x40) & ~0xF; return a
    def reg(self, name):
        if name in ('sp', 'wsp'): return self.sp if name == 'sp' else self.sp & 0xFFFFFFFF
        if name in ('xzr',): return 0
        if name in ('wzr',): return 0
        if name == 'fp': return self.x[29]
        if name == 'lr': return self.x[30]
        k = name[0]; i = int(name[1:])
        if k == 'x': return self.x[i]
        if k == 'w': return self.x[i] & 0xFFFFFFFF
        raise Unsupported('reg ' + name)
    def setr(self, name, val):
        if name in ('xzr', 'wzr'): return
        if name == 'sp': self.sp = val & M64; return
        k = name[0]; i = int(name[1:]) if name[1:].isdigit() else None
        if name == 'fp': i = 29; k = 'x'
        if name == 'lr': i = 30; k = 'x'
        if k == 'x': self.x[i] = val & M64
        elif k == 'w': self.x[i] = val & 0xFFFFFFFF
        else: raise Unsupported('setreg ' + name)
    def width(self, name): return 32 if name[0] == 'w' else 64

def parse_imm(s):
    s = s.strip().lstrip('#')
    return int(s, 0)

def shift_val(v, kind, amt, bits):
    mask = (1 << bits) - 1
    v &= mask
    if kind == 'lsl': return (v << amt) & mask
    if kind == 'lsr': return v >> amt
    if kind == 'asr': return (sx(v, bits) >> amt) & mask
    if kind == 'ror': return ((v >> amt) | (v << (bits - amt))) & mask
    raise Unsupported(kind)

def operand(cpu, ops, bits):
    """second-source operand: reg, reg+shift, imm, imm lsl"""
    if ops[0].startswith('#'):
        v = parse_imm(ops[0])
        if len(ops) > 1 and ops[1].startswith('lsl'):
            v <<= int(ops[1].split('#')[1], 0)
        return v & ((1 << bits) - 1)
    v = cpu.reg(ops[0])
    if len(ops) > 1:
        kind, amt = ops[1].split()
        amt = int(amt.lstrip('#'), 0)
        if kind in ('uxtw', 'sxtw', 'uxtx', 'sxtx', 'uxtb', 'uxth'):
            if kind == 'sxtw': v = sx(v, 32) & M64
            elif kind == 'uxtw': v &= 0xFFFFFFFF
            elif kind == 'uxtb': v &= 0xFF
            elif kind == 'uxth': v &= 0xFFFF
            v = (v << amt) & ((1 << bits) - 1)
        else:
            v = shift_val(v, kind, amt, bits)
    return v & ((1 << bits) - 1)

def split_ops(s):
    # split by commas not inside brackets
    out, depth, cur = [], 0, ''
    for ch in s:
        if ch == '[': depth += 1
        if ch == ']': depth -= 1
        if ch == ',' and depth == 0:
            out.append(cur.strip()); cur = ''
        else: cur += ch
    if cur.strip(): out.append(cur.strip())
    return out

def addr_mode(cpu, ops):
    """returns (address, writeback_fn) for [base, off]{!} or [base], #post"""
    mem = ops[0] if ops[0].startswith('[') else None
    idx = 0
    for i, o in enumerate(ops):
        if o.startswith('['): idx = i; break
    m = ops[idx]
    pre = m.endswith('!')
    inner = m.strip('!').strip('[]')
    parts = [p.strip() for p in inner.split(',')]
    base = parts[0]; bval = cpu.reg(base)
    off = 0
    if len(parts) > 1:
        if parts[1].startswith('#'): off = parse_imm(parts[1])
        else:
            rv = cpu.reg(parts[1])
            if parts[1].startswith('w'):
                ext = parts[2].split()[0] if len(parts) > 2 else 'uxtw'
                rv = sx(rv, 32) & M64 if ext == 'sxtw' else rv & 0xFFFFFFFF
            if len(parts) > 2:
                tok = parts[2].split()
                if len(tok) > 1: rv = (rv << int(tok[1].lstrip('#'), 0)) & M64
            off = rv
    post = None
    if idx + 1 < len(ops):
        post = parse_imm(ops[idx + 1])
    if post is not None:
        return bval, (lambda: cpu.setr(base, bval + post)), idx
    a = (bval + off) & M64
    if pre:
        return a, (lambda: cpu.setr(base, a)), idx
    return a, None, idx

def flags_add(cpu, a, b, bits, carry_in=0):
    mask = (1 << bits) - 1
    r = (a + b + carry_in)
    res = r & mask
    cpu.n = bool(res >> (bits - 1)); cpu.z = res == 0; cpu.c = r > mask
    cpu.vf = (sx(a, bits) + sx(b, bits) + carry_in) != sx(res, bits)
    return res

def flags_sub(cpu, a, b, bits):
    mask = (1 << bits) - 1
    return flags_add(cpu, a, (~b) & mask, bits, 1)

def flags_logic(cpu, res, bits):
    cpu.n = bool(res >> (bits - 1)); cpu.z = res == 0; cpu.c = False; cpu.vf = False

FAKE_META = 0x500000000000

def helper(cpu, target, fin):
    """Model calls. Return True if handled."""
    name = stubs.get(target)
    m = cpu.m
    if target == 0x1002ad744:  # Data(bytes: start x0, end x1)
        bs = m.read(cpu.x[0], (cpu.x[1] - cpu.x[0]) & M64) if cpu.x[1] >= cpu.x[0] else b''
        h = cpu.dh; cpu.dh += 1; cpu.datas[h] = bytes(bs)
        cpu.x[0] = 0xD0D0000000000000 | h; cpu.x[1] = 0; return True
    if target == 0x10035aa50:  # Array(Data) -> [UInt8]
        h = cpu.x[0] & 0xFFFFFFFF; bs = cpu.datas[h]
        a = cpu.alloc(0x20 + len(bs) + 8)
        m.w(a + 0x10, len(bs), 8); m.w(a + 0x18, max(len(bs), 1) << 1, 8); m.write(a + 0x20, bs)
        cpu.x[0] = a; return True
    if target == 0x1002ce108:  # grow array: x1 minCapacity, x3 old buffer
        old = cpu.x[3]; cnt = m.r(old + 0x10, 8) if old else 0
        cap_old = (m.r(old + 0x18, 8) >> 1) if old else 0
        cap = max(cpu.x[1], cap_old * 2, 16)
        a = cpu.alloc(0x20 + cap + 8)
        m.w(a + 0x10, cnt, 8); m.w(a + 0x18, cap << 1, 8)
        if cnt: m.write(a + 0x20, m.read(old + 0x20, cnt))
        cpu.x[0] = a; return True
    if target == 0x100364ca4:  # make unique copy of array x0
        old = cpu.x[0]; cnt = m.r(old + 0x10, 8); cap = max(m.r(old + 0x18, 8) >> 1, cnt)
        a = cpu.alloc(0x20 + cap + 8); m.w(a + 0x10, cnt, 8); m.w(a + 0x18, cap << 1, 8)
        m.write(a + 0x20, m.read(old + 0x20, cnt)); cpu.x[0] = a; return True
    if target in (0x10001e290, 0x100013f68):  # outlined retain/release of Data
        return True
    if name is None:
        return False
    if 'allocateBufferUninitialized' in name:
        cap = cpu.x[0]; a = cpu.alloc(0x20 + cap + 8)
        m.w(a + 0x10, 0, 8); m.w(a + 0x18, cap << 1, 8); cpu.x[0] = a; return True
    if name.endswith('EncodingVMa'):
        cpu.x[0] = FAKE_META + 0x10; m.w(FAKE_META + 0x08, FAKE_META + 0x100, 8); m.w(FAKE_META + 0x100 + 0x40, 8, 8); return True
    if 'EncodingV4utf8' in name:
        return True
    if 'data8encoding' in name:  # String(data:encoding:)
        h = cpu.x[0] & 0xFFFFFFFF; bs = cpu.datas.get(h, b'')
        cpu.result = bs; cpu.x[0] = 0x5555; cpu.x[1] = 0x6666; return True
    if name in ('_swift_isUniquelyReferenced_nonNull_native',):
        cpu.x[0] = 1; return True
    if name in ('_swift_bridgeObjectRelease', '_swift_bridgeObjectRetain', '_swift_release', '_swift_retain',
                '_objc_release', '_objc_retain', '_swift_retain_n', '_swift_release_n', '_swift_endAccess',
                '_swift_beginAccess'):
        return True
    if 'removeSubrange' in name or 'index_8offsetBy' in name:
        raise Unsupported('string op ' + name)
    if name == '_memcpy':
        m.write(cpu.x[0], m.read(cpu.x[1], cpu.x[2])); return True
    if name == '_memmove':
        m.write(cpu.x[0], m.read(cpu.x[1], cpu.x[2])); return True
    if name == '___stack_chk_fail':
        raise Stop('stack chk')
    raise Unsupported('call ' + name)

REMOVE_LAST = {0x10035de48}

def run(fn, maxsteps=400000):
    j = bisect.bisect_right(fset, fn)
    end = fset[j] if j < len(fset) else fn + 0x4000
    ins = disasm(fn, end)
    mem = Mem(); cpu = CPU(mem)
    cpu.x[30] = 0xDEAD0000; cpu.x[29] = cpu.sp
    return run_from(fn, ins, cpu, mem, maxsteps)

def run_from(pc, ins, cpu, mem, maxsteps=400000):
    end = None; steps = 0
    while True:
        steps += 1
        if steps > maxsteps: raise Stop('steps')
        if pc == 0xDEAD0000: break
        if pc not in ins:
            # removeLast thunk lives elsewhere
            if pc in REMOVE_LAST:
                n = cpu.x[0]; arr = mem.r(cpu.x[20], 8); cnt = mem.r(arr + 0x10, 8)
                mem.w(arr + 0x10, cnt - n, 8); pc = cpu.x[30]; continue
            raise Unsupported('pc outside fn %x' % pc)
        mn, opstr = ins[pc]; ops = split_ops(opstr)
        nxt = pc + 4
        base = mn.split('.')[0]
        cond = mn.split('.')[1] if '.' in mn and base == 'b' else None
        if base == 'b' and cond:
            if COND[cond](cpu.n, cpu.z, cpu.c, cpu.vf): nxt = parse_imm(ops[0])
        elif mn == 'b':
            nxt = parse_imm(ops[0])
            if nxt not in ins:
                # tail call into helper
                if nxt in REMOVE_LAST or helper(cpu, nxt, end):
                    if nxt in REMOVE_LAST:
                        n = cpu.x[0]; arr = mem.r(cpu.x[20], 8); cnt = mem.r(arr + 0x10, 8); mem.w(arr + 0x10, cnt - n, 8)
                    nxt = cpu.x[30]
                else:
                    raise Unsupported('tail b %x' % nxt)
        elif mn == 'bl':
            t = parse_imm(ops[0])
            if t in REMOVE_LAST:
                n = cpu.x[0]; arr = mem.r(cpu.x[20], 8); cnt = mem.r(arr + 0x10, 8); mem.w(arr + 0x10, cnt - n, 8)
            elif not helper(cpu, t, end):
                if t in ins:
                    cpu.x[30] = nxt; nxt = t
                else:
                    raise Unsupported('bl %x %s' % (t, stubs.get(t)))
        elif mn == 'blr':
            pass  # ___chkstk_darwin
        elif mn in ('ret', 'retab'):
            nxt = cpu.x[30]
        elif mn in ('cbz', 'cbnz'):
            v = cpu.reg(ops[0])
            if (v == 0) == (mn == 'cbz'): nxt = parse_imm(ops[1])
        elif mn in ('tbz', 'tbnz'):
            v = cpu.reg(ops[0]); bit = parse_imm(ops[1])
            if (((v >> bit) & 1) == 0) == (mn == 'tbz'): nxt = parse_imm(ops[2])
        elif mn == 'brk':
            raise Stop('brk at %x' % pc)
        elif mn in ('mov', 'movz'):
            d = ops[0]
            if ops[1].startswith('#'): cpu.setr(d, parse_imm(ops[1]))
            else:
                src = ops[1]
                if src.startswith('v') or src.startswith('q'): raise Unsupported('vec mov')
                cpu.setr(d, cpu.reg(src))
        elif mn == 'movk':
            d = ops[0]; imm = parse_imm(ops[1]); sh = int(ops[2].split('#')[1], 0) if len(ops) > 2 else 0
            v = cpu.reg(d); v = (v & ~(0xFFFF << sh)) | (imm << sh); cpu.setr(d, v)
        elif mn == 'movn':
            bits = cpu.width(ops[0]); imm = parse_imm(ops[1]); sh = int(ops[2].split('#')[1], 0) if len(ops) > 2 else 0
            cpu.setr(ops[0], (~(imm << sh)) & ((1 << bits) - 1))
        elif mn in ('add', 'adds', 'sub', 'subs'):
            bits = cpu.width(ops[0]); a = cpu.reg(ops[1]); b = operand(cpu, ops[2:], bits)
            mask = (1 << bits) - 1
            if mn == 'add': r = (a + b) & mask
            elif mn == 'sub': r = (a - b) & mask
            elif mn == 'adds': r = flags_add(cpu, a & mask, b, bits)
            else: r = flags_sub(cpu, a & mask, b, bits)
            cpu.setr(ops[0], r)
        elif mn in ('cmp', 'cmn'):
            bits = cpu.width(ops[0]) if ops[0] not in ('sp',) else 64
            a = cpu.reg(ops[0]) & ((1 << bits) - 1); b = operand(cpu, ops[1:], bits)
            if mn == 'cmp': flags_sub(cpu, a, b, bits)
            else: flags_add(cpu, a, b, bits)
        elif mn in ('neg', 'negs'):
            bits = cpu.width(ops[0]); b = operand(cpu, ops[1:], bits)
            r = flags_sub(cpu, 0, b, bits) if mn == 'negs' else (-b) & ((1 << bits) - 1)
            cpu.setr(ops[0], r)
        elif mn in ('and', 'ands', 'orr', 'eor', 'bic', 'orn', 'eon', 'tst'):
            if mn == 'tst':
                bits = cpu.width(ops[0]); a = cpu.reg(ops[0]); b = operand(cpu, ops[1:], bits)
                flags_logic(cpu, a & b, bits)
            else:
                bits = cpu.width(ops[0]); a = cpu.reg(ops[1]); b = operand(cpu, ops[2:], bits); mask = (1 << bits) - 1
                r = {'and': a & b, 'ands': a & b, 'orr': a | b, 'eor': a ^ b, 'bic': a & ~b, 'orn': a | (~b & mask), 'eon': a ^ (~b & mask)}[mn] & mask
                if mn == 'ands': flags_logic(cpu, r, bits)
                cpu.setr(ops[0], r)
        elif mn == 'mvn':
            bits = cpu.width(ops[0]); cpu.setr(ops[0], (~operand(cpu, ops[1:], bits)) & ((1 << bits) - 1))
        elif mn in ('lsl', 'lsr', 'asr', 'ror'):
            bits = cpu.width(ops[0]); a = cpu.reg(ops[1])
            amt = parse_imm(ops[2]) if ops[2].startswith('#') else cpu.reg(ops[2]) % bits
            cpu.setr(ops[0], shift_val(a, mn, amt, bits))
        elif mn in ('madd', 'msub'):
            bits = cpu.width(ops[0]); a, b, c = (cpu.reg(o) for o in ops[1:4])
            r = (c + a * b) if mn == 'madd' else (c - a * b)
            cpu.setr(ops[0], r & ((1 << bits) - 1))
        elif mn in ('mul', 'mneg'):
            bits = cpu.width(ops[0]); r = cpu.reg(ops[1]) * cpu.reg(ops[2])
            cpu.setr(ops[0], (r if mn == 'mul' else -r) & ((1 << bits) - 1))
        elif mn in ('umulh', 'smulh'):
            a, b = cpu.reg(ops[1]), cpu.reg(ops[2])
            if mn == 'smulh': a, b = sx(a, 64), sx(b, 64)
            cpu.setr(ops[0], ((a * b) >> 64) & M64)
        elif mn in ('udiv', 'sdiv'):
            bits = cpu.width(ops[0]); a, b = cpu.reg(ops[1]), cpu.reg(ops[2])
            if mn == 'sdiv': a, b = sx(a, bits), sx(b, bits)
            r = 0 if b == 0 else int(a / b) if mn == 'sdiv' else a // b
            cpu.setr(ops[0], r & ((1 << bits) - 1))
        elif mn in ('ubfx', 'sbfx', 'ubfiz', 'sbfiz', 'bfi', 'bfxil'):
            bits = cpu.width(ops[0]); a = cpu.reg(ops[1]); lsb = parse_imm(ops[2]); w = parse_imm(ops[3])
            if mn == 'ubfx': r = (a >> lsb) & ((1 << w) - 1)
            elif mn == 'sbfx': r = sx((a >> lsb) & ((1 << w) - 1), w) & ((1 << bits) - 1)
            elif mn == 'ubfiz': r = ((a & ((1 << w) - 1)) << lsb) & ((1 << bits) - 1)
            elif mn == 'sbfiz': r = (sx(a & ((1 << w) - 1), w) << lsb) & ((1 << bits) - 1)
            elif mn == 'bfi':
                d = cpu.reg(ops[0]); fm = ((1 << w) - 1) << lsb; r = (d & ~fm) | ((a << lsb) & fm)
            else:
                d = cpu.reg(ops[0]); r = (d & ~((1 << w) - 1)) | ((a >> lsb) & ((1 << w) - 1))
            cpu.setr(ops[0], r)
        elif mn in ('sxtw', 'sxtb', 'sxth', 'uxtb', 'uxth'):
            a = cpu.reg(ops[1])
            r = {'sxtw': sx(a, 32), 'sxtb': sx(a, 8), 'sxth': sx(a, 16), 'uxtb': a & 0xFF, 'uxth': a & 0xFFFF}[mn]
            cpu.setr(ops[0], r & ((1 << cpu.width(ops[0])) - 1))
        elif mn == 'extr':
            bits = cpu.width(ops[0]); a, b = cpu.reg(ops[1]), cpu.reg(ops[2]); lsb = parse_imm(ops[3])
            r = ((a << bits) | b) >> lsb
            cpu.setr(ops[0], r & ((1 << bits) - 1))
        elif mn in ('csel', 'csinc', 'csinv', 'csneg'):
            bits = cpu.width(ops[0]); a, b = cpu.reg(ops[1]), cpu.reg(ops[2]); cnd = ops[3]
            if COND[cnd](cpu.n, cpu.z, cpu.c, cpu.vf): r = a
            else:
                r = {'csel': b, 'csinc': b + 1, 'csinv': ~b, 'csneg': -b}[mn]
            cpu.setr(ops[0], r & ((1 << bits) - 1))
        elif mn in ('cset', 'csetm'):
            t = COND[ops[1]](cpu.n, cpu.z, cpu.c, cpu.vf)
            cpu.setr(ops[0], (1 if mn == 'cset' else (1 << cpu.width(ops[0])) - 1) if t else 0)
        elif mn in ('cinc', 'cneg', 'cinv'):
            bits = cpu.width(ops[0]); a = cpu.reg(ops[1]); t = COND[ops[2]](cpu.n, cpu.z, cpu.c, cpu.vf)
            r = a if not t else {'cinc': a + 1, 'cneg': -a, 'cinv': ~a}[mn]
            cpu.setr(ops[0], r & ((1 << bits) - 1))
        elif mn in ('ccmp', 'ccmn'):
            bits = cpu.width(ops[0]); a = cpu.reg(ops[0]); b = operand(cpu, [ops[1]], bits)
            nzcv = parse_imm(ops[2]); cnd = ops[3]
            if COND[cnd](cpu.n, cpu.z, cpu.c, cpu.vf):
                (flags_sub if mn == 'ccmp' else flags_add)(cpu, a, b, bits)
            else:
                cpu.n, cpu.z, cpu.c, cpu.vf = bool(nzcv & 8), bool(nzcv & 4), bool(nzcv & 2), bool(nzcv & 1)
        elif mn == 'adrp' or mn == 'adr':
            cpu.setr(ops[0], parse_imm(ops[1]))
        elif mn.startswith('ld') or mn.startswith('st'):
            a, wb, idx = addr_mode(cpu, ops)
            regs = ops[:idx]
            sizes = {'b': 1, 'h': 2}
            base = mn
            signed = base.startswith('ldrs')
            if base in ('ldrb', 'ldurb', 'strb', 'sturb', 'ldrsb', 'ldursb'): sz = 1
            elif base in ('ldrh', 'ldurh', 'strh', 'sturh', 'ldrsh', 'ldursh'): sz = 2
            elif base in ('ldrsw', 'ldursw'): sz = 4
            else:
                r0 = regs[0]
                sz = {'x': 8, 'w': 4, 'q': 16, 'd': 8, 's': 4}.get(r0[0], 8)
            is_load = mn.startswith('ld')
            for k, r in enumerate(regs):
                ea = a + k * sz
                if is_load:
                    v = mem.r(ea, sz)
                    if signed: v = sx(v, sz * 8) & M64
                    if r[0] in 'qds': cpu.v[int(r[1:])] = v
                    else: cpu.setr(r, v)
                else:
                    if r[0] in 'qds': v = cpu.v[int(r[1:])]
                    else: v = cpu.reg(r)
                    mem.w(ea, v, sz)
            if wb: wb()
        elif mn.startswith('movi'):
            cpu.v[int(ops[0][1:])] = 0
        elif mn in ('nop', 'hint', 'pacibsp', 'autibsp', 'bti', 'dmb', 'isb', 'pacia', 'autia', 'xpaci', 'paciasp', 'autiasp'):
            pass
        else:
            raise Unsupported('insn %s %s at %x' % (mn, opstr, pc))
        pc = nxt
    return cpu.result

if __name__ == '__main__':
    targets = [int(a, 16) for a in sys.argv[2:]] or sorted(decoders)
    out = open(WORK + '/deobf.tsv', 'w') if len(sys.argv) <= 2 else sys.stdout
    ok = bad = 0
    errs = {}
    for f in targets:
        try:
            r = run(f)
            if r is None: raise Stop('no result')
            out.write('%x\t%s\n' % (f, r.decode('utf-8', 'replace').replace('\\', '\\\\').replace('\n', '\\n').replace('\t', '\\t')))
            ok += 1
        except (Stop, Unsupported, KeyError, IndexError, ValueError) as e:
            bad += 1; errs.setdefault(str(e)[:60], []).append('%x' % f)
    sys.stderr.write('ok %d bad %d\n' % (ok, bad))
    for k, v in sorted(errs.items(), key=lambda kv: -len(kv[1]))[:20]:
        sys.stderr.write('  %4d %s  e.g. %s\n' % (len(v), k, v[0]))
