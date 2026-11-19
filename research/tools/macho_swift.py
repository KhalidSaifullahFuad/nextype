#!/usr/bin/env python3
# Analysis helper (scratch): parse Mach-O sections + Swift type/field metadata. Read-only.
import struct, sys, json

B = sys.argv[1]
data = open(B, 'rb').read()

def u32(o): return struct.unpack_from('<I', data, o)[0]
def i32(o): return struct.unpack_from('<i', data, o)[0]
def u64(o): return struct.unpack_from('<Q', data, o)[0]

magic = u32(0)
assert magic == 0xfeedfacf, hex(magic)
ncmds = u32(16)
off = 32
segs, sects = [], {}
for _ in range(ncmds):
    cmd, size = u32(off), u32(off + 4)
    if cmd == 0x19:  # LC_SEGMENT_64
        segname = data[off+8:off+24].rstrip(b'\0').decode()
        vmaddr, vmsize, fileoff, filesize = struct.unpack_from('<QQQQ', data, off + 24)
        segs.append((segname, vmaddr, vmsize, fileoff, filesize))
        nsect = u32(off + 64)
        so = off + 72
        for _ in range(nsect):
            sn = data[so:so+16].rstrip(b'\0').decode()
            sg = data[so+16:so+32].rstrip(b'\0').decode()
            addr, sz = struct.unpack_from('<QQ', data, so + 32)
            foff = u32(so + 48)
            sects[(sg, sn)] = (addr, sz, foff)
            so += 80
    off += size

def va2off(va):
    for (sg, vm, vs, fo, fs) in segs:
        if vm <= va < vm + vs:
            return va - vm + fo
    return None

def off2va(o):
    for (sg, vm, vs, fo, fs) in segs:
        if fo <= o < fo + fs:
            return o - fo + vm
    return None

def cstr(o, maxlen=4096):
    if o is None or o < 0 or o >= len(data): return None
    e = data.find(b'\0', o, o + maxlen)
    if e < 0: return None
    return data[o:e]

def relptr(o):
    return o + i32(o)

def mangled(o):
    """Read a mangled name with symbolic refs; render symbolic refs as placeholders."""
    out = b''
    i = o
    while True:
        c = data[i]
        if c == 0: break
        if 1 <= c <= 0x17:
            # symbolic reference: 1 byte kind + 4 byte rel offset
            tgt = i + 1 + i32(i + 1)
            nm = ctx_name(tgt) if c in (1, 2) else None
            out += ('{' + (nm or ('sym%02x@%x' % (c, off2va(tgt) or 0))) + '}').encode()
            i += 5
            continue
        if 0x18 <= c <= 0x1f:
            i += 9; out += b'{abs}'; continue
        out += bytes([c]); i += 1
    return out.decode('latin1')

_ctx_cache = {}
def ctx_name(o, depth=0):
    """Context descriptor full name: Parent.Child."""
    if o in _ctx_cache: return _ctx_cache[o]
    if depth > 12: return '?'
    try:
        flags = u32(o)
    except Exception:
        return None
    kind = flags & 0x1f
    parent_rel = i32(o + 4)
    parent = (o + 4 + parent_rel) if parent_rel else None
    if parent is not None and (parent_rel & 1):
        parent = None  # indirect; skip
    name = None
    if kind == 0:  # module
        s = cstr(relptr(o + 8)); name = s.decode() if s else '?'
    elif kind == 1:  # extension
        name = 'ext'
    elif kind in (16, 17, 18, 3):  # class struct enum protocol
        s = cstr(relptr(o + 8)); name = s.decode() if s else '?'
    elif kind == 2:  # anonymous
        name = '(anon)'
    else:
        name = 'k%d' % kind
    pn = ctx_name(parent, depth + 1) if parent else None
    full = (pn + '.' + name) if pn and pn not in ('Cotypist',) else name
    _ctx_cache[o] = full
    return full

KIND = {16: 'class', 17: 'struct', 18: 'enum', 3: 'protocol'}

def types():
    addr, sz, foff = sects[('__TEXT', '__swift5_types')]
    res = []
    for k in range(sz // 4):
        p = foff + k * 4
        tgt = p + i32(p)
        if tgt & 1: continue
        flags = u32(tgt)
        kind = flags & 0x1f
        nm = ctx_name(tgt)
        fd_rel_at = tgt + 16
        fdo = fd_rel_at + i32(fd_rel_at) if i32(fd_rel_at) else None
        res.append((nm, KIND.get(kind, kind), tgt, fdo))
    return res

def fields(fdo):
    if fdo is None: return []
    mtn = i32(fdo)
    kind = struct.unpack_from('<H', data, fdo + 8)[0]
    recsz = struct.unpack_from('<H', data, fdo + 10)[0]
    n = u32(fdo + 12)
    out = []
    for j in range(n):
        r = fdo + 16 + j * recsz
        fl = u32(r)
        tn = i32(r + 4); fnr = i32(r + 8)
        t = mangled(r + 4 + tn) if tn else ''
        f = cstr(r + 8 + fnr) if fnr else b''
        out.append((f.decode('latin1') if f else '', t, fl))
    return out

if __name__ == '__main__':
    mode = sys.argv[2] if len(sys.argv) > 2 else 'types'
    if mode == 'sects':
        for k, v in sects.items(): print(k, hex(v[0]), hex(v[1]), hex(v[2]))
    elif mode == 'types':
        for nm, kind, tgt, fdo in types():
            fs = fields(fdo)
            print('%s %s' % (kind, nm))
            for f, t, fl in fs:
                print('    %s%s: %s' % ('var ' if fl & 2 else '', f, t))
