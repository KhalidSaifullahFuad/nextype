#!/usr/bin/env python3
# Scratch analysis helper: linear ARM64 sweep of a Mach-O __text.
# Recovers per-function: cstring refs (incl. Swift large-literal -0x20 form), Swift small-string
# immediates (mov/movk), calls to imported stubs and ObjC selector stubs, local BL targets.
import struct, sys, pickle, array

B = sys.argv[1]
OUT = sys.argv[2]
data = open(B, 'rb').read()

def u32(o): return struct.unpack_from('<I', data, o)[0]

ncmds = u32(16); off = 32
segs, sects = [], {}
fstarts_off = fstarts_sz = None
symoff = nsyms = stroff = strsize = None
indirectsymoff = nindirect = None
for _ in range(ncmds):
    cmd, size = u32(off), u32(off + 4)
    if cmd == 0x19:
        segname = data[off+8:off+24].rstrip(b'\0').decode()
        vmaddr, vmsize, fileoff, filesize = struct.unpack_from('<QQQQ', data, off + 24)
        segs.append((segname, vmaddr, vmsize, fileoff, filesize))
        nsect = u32(off + 64); so = off + 72
        for _ in range(nsect):
            sn = data[so:so+16].rstrip(b'\0').decode(); sg = data[so+16:so+32].rstrip(b'\0').decode()
            addr, sz = struct.unpack_from('<QQ', data, so + 32)
            foff = u32(so + 48); res1 = u32(so + 68); res2 = u32(so + 72)
            sects[(sg, sn)] = (addr, sz, foff, res1, res2)
            so += 80
    elif cmd == 0x26:  # LC_FUNCTION_STARTS
        fstarts_off, fstarts_sz = u32(off + 8), u32(off + 12)
    elif cmd == 0x2:  # LC_SYMTAB
        symoff, nsyms, stroff, strsize = struct.unpack_from('<IIII', data, off + 8)
    elif cmd == 0xb:  # LC_DYSYMTAB
        indirectsymoff, nindirect = u32(off + 56), u32(off + 60)
    off += size

text_vm = [s for s in segs if s[0] == '__TEXT'][0][1]

def va2off(va):
    for (sg, vm, vs, fo, fs) in segs:
        if vm <= va < vm + fs:
            return va - vm + fo
    return None

def cstr_at_va(va):
    o = va2off(va)
    if o is None: return None
    e = data.find(b'\0', o, o + 8192)
    if e < 0: return None
    return data[o:e].decode('utf-8', 'replace')

# function starts
fstarts = []
p = fstarts_off; end = fstarts_off + fstarts_sz; addr = text_vm
while p < end:
    v = 0; sh = 0
    while True:
        b = data[p]; p += 1
        v |= (b & 0x7f) << sh; sh += 7
        if not b & 0x80: break
    if v == 0: break
    addr += v; fstarts.append(addr)

# symbols for stubs
def sym_name(idx):
    n_strx = u32(symoff + idx * 16)
    o = stroff + n_strx
    e = data.find(b'\0', o)
    return data[o:e].decode('latin1')

stub_names = {}
sa, ssz, sfo, sres1, sres2 = sects[('__TEXT', '__stubs')]
stub_size = sres2 or 12
for i in range(ssz // stub_size):
    isym = u32(indirectsymoff + (sres1 + i) * 4)
    if isym & 0xC0000000: continue
    stub_names[sa + i * stub_size] = sym_name(isym)

# objc stubs -> selector
objc_stub_sel = {}
if ('__TEXT', '__objc_stubs') in sects:
    oa, osz, ofo, _, _ = sects[('__TEXT', '__objc_stubs')]
    for i in range(osz // 32):
        pc = oa + i * 32; fo = ofo + i * 32
        i0, i1 = u32(fo), u32(fo + 4)
        if (i0 & 0x9F000000) == 0x90000000 and (i1 & 0xFFC00000) == 0xF9400000:
            immlo = (i0 >> 29) & 3; immhi = (i0 >> 5) & 0x7FFFF
            imm = ((immhi << 2) | immlo)
            if imm & (1 << 20): imm -= (1 << 21)
            page = (pc & ~0xFFF) + (imm << 12)
            selref = page + ((i1 >> 10) & 0xFFF) * 8
            so_ = va2off(selref)
            if so_ is not None:
                ptr = struct.unpack_from('<Q', data, so_)[0]
                # chained fixups: low bits hold target offset for rebase
                tgt = ptr & 0xFFFFFFFFF
                if tgt < 0x100000000: tgt += 0x100000000
                s = cstr_at_va(tgt)
                objc_stub_sel[pc] = s

cs_a, cs_sz = sects[('__TEXT', '__cstring')][:2]
def in_cstring(va): return cs_a <= va < cs_a + cs_sz
const_a, const_sz = sects[('__TEXT', '__const')][:2]

ta, tsz, tfo = sects[('__TEXT', '__text')][:3]
words = array.array('I'); words.frombytes(data[tfo:tfo + tsz])

import bisect
fset = sorted(fstarts)

funcs = {}  # start -> dict
def func_of(pc):
    i = bisect.bisect_right(fset, pc) - 1
    return fset[i] if i >= 0 else None

regs = [None] * 32  # immediate tracking (mov/movk)
adrp = [None] * 32
cur = None
smallbuf = {}
def printable(bs):
    return all(32 <= c < 127 or c >= 0x80 for c in bs)

for idx in range(len(words)):
    pc = ta + idx * 4
    if cur is None or (idx and pc in funcs_starts_set if False else False):
        pass
    f = func_of(pc)
    if f != cur:
        cur = f
        regs = [None] * 32; adrp = [None] * 32
        funcs.setdefault(f, {'strs': [], 'small': [], 'calls': [], 'objc': [], 'bl': [], 'consts': []})
    F = funcs[f]
    ins = words[idx]
    # MOVZ / MOVN / MOVK 64 and 32
    op = ins & 0xFF800000
    if op in (0xD2800000, 0x52800000):  # movz
        rd = ins & 31; hw = (ins >> 21) & 3; imm = (ins >> 5) & 0xFFFF
        regs[rd] = imm << (16 * hw)
        continue
    if op in (0xF2800000, 0x72800000):  # movk
        rd = ins & 31; hw = (ins >> 21) & 3; imm = (ins >> 5) & 0xFFFF
        if regs[rd] is not None:
            v = regs[rd] & ~(0xFFFF << (16 * hw)) | (imm << (16 * hw))
            regs[rd] = v
            # small-string discriminator word: top byte 0xE0..0xEF
            if hw == 3 and (v >> 56) & 0xF0 == 0xE0:
                F['small'].append((pc, rd, v))
            elif hw >= 1:
                bs = v.to_bytes(8, 'little').rstrip(b'\0')
                if len(bs) >= 3 and printable(bs):
                    F['small'].append((pc, rd, v))
        continue
    if (ins & 0x9F000000) == 0x90000000:  # adrp
        rd = ins & 31
        immlo = (ins >> 29) & 3; immhi = (ins >> 5) & 0x7FFFF
        imm = ((immhi << 2) | immlo)
        if imm & (1 << 20): imm -= (1 << 21)
        adrp[rd] = (pc & ~0xFFF) + (imm << 12)
        regs[rd] = None
        continue
    if (ins & 0xFF800000) == 0x91000000:  # add imm 64
        rd = ins & 31; rn = (ins >> 5) & 31; imm = (ins >> 10) & 0xFFF
        if (ins >> 22) & 1: imm <<= 12
        if adrp[rn] is not None:
            t = adrp[rn] + imm
            if in_cstring(t):
                F['strs'].append((pc, t))
            elif in_cstring(t + 0x20):
                F['strs'].append((pc, t + 0x20))
            elif const_a <= t < const_a + const_sz:
                F['consts'].append((pc, t))
            if rd != rn: adrp[rd] = None
            else: adrp[rd] = None
        continue
    if (ins & 0xFC000000) == 0x94000000:  # bl
        imm = ins & 0x3FFFFFF
        if imm & (1 << 25): imm -= (1 << 26)
        t = pc + imm * 4
        if t in stub_names:
            F['calls'].append((pc, stub_names[t]))
        elif t in objc_stub_sel:
            F['objc'].append((pc, objc_stub_sel[t]))
        else:
            F['bl'].append((pc, t))
        # calls clobber x0-x18
        for r in range(19): regs[r] = None; adrp[r] = None
        continue

pickle.dump({'funcs': funcs, 'fstarts': fset, 'stubs': stub_names, 'objc': objc_stub_sel}, open(OUT, 'wb'))
print('functions', len(fset), 'with data', len(funcs), 'stubs', len(stub_names), 'objc stubs', len(objc_stub_sel))
