#!/usr/bin/env python3
# Scratch query tool over scan.pkl. Usage:
#   q.py str <substring>        functions referencing a cstring containing substring
#   q.py small <substring>      functions whose decoded small strings contain substring
#   q.py call <symbol-substr>   functions calling an imported stub
#   q.py sel <selector-substr>  functions sending an ObjC selector
#   q.py fn <hexaddr> [...]     dump function summary
#   q.py callers <hexaddr>      functions that BL to addr
import pickle, sys, struct, bisect, os
D = pickle.load(open(os.path.join(os.path.dirname(os.path.abspath(sys.argv[0])), 'work', 'scan.pkl'), 'rb'))
funcs, fset = D['funcs'], D['fstarts']
data = open('/Applications/Cotypist.app/Contents/MacOS/Cotypist', 'rb').read()

def cstr(va):
    o = va - 0x100000000
    e = data.find(b'\0', o, o + 8192)
    return data[o:e].decode('utf-8', 'replace')

def dec_small(v):
    bs = v.to_bytes(8, 'little')
    if (v >> 56) & 0xF0 == 0xE0:
        return bs[:7].rstrip(b'\0').decode('utf-8', 'replace'), (v >> 56) & 0xF
    return bs.rstrip(b'\0').decode('utf-8', 'replace'), None

def small_strings(F):
    """Pair consecutive small words into Swift small strings (best effort)."""
    out = []; pending = {}
    for pc, rd, v in F['small']:
        s, cnt = dec_small(v)
        if cnt is None:
            pending[rd] = (pc, s)
        else:
            # find first word in some register set just before
            first = None
            for r in (rd - 1, rd - 2, rd + 1):
                if r in pending:
                    first = pending.pop(r); break
            txt = (first[1].ljust(8, '\0') if first else '') + s
            txt = txt.replace('\0', '')[:cnt] if cnt else txt
            out.append((pc, txt))
    for r, (pc, s) in pending.items():
        out.append((pc, s + '…'))
    out.sort()
    return out

def size_of(f):
    i = bisect.bisect_right(fset, f)
    return (fset[i] - f) if i < len(fset) else 0

def summary(f, full=True, maxs=400):
    F = funcs[f]
    print('=== fn 0x%x  size=%d  callers=%d' % (f, size_of(f), len(callers_of(f))))
    for pc, t in F['strs'][:maxs]:
        print('  str  %x: %r' % (pc, cstr(t)[:200]))
    for pc, s in small_strings(F)[:maxs]:
        print('  sml  %x: %r' % (pc, s))
    for pc, s in F['calls'][:maxs]:
        print('  api  %x: %s' % (pc, s))
    for pc, s in F['objc'][:maxs]:
        print('  objc %x: %s' % (pc, s))
    if full:
        bl = sorted(set(t for _, t in F['bl']))
        print('  bl -> ' + ' '.join('%x' % t for t in bl[:120]))

_callers = None
def callers_of(addr):
    global _callers
    if _callers is None:
        _callers = {}
        for f, F in funcs.items():
            for pc, t in F['bl']:
                _callers.setdefault(t, set()).add(f)
    return _callers.get(addr, set())

cmd = sys.argv[1]; arg = sys.argv[2] if len(sys.argv) > 2 else ''
if cmd == 'str':
    for f, F in sorted(funcs.items()):
        hits = [cstr(t) for pc, t in F['strs'] if arg in cstr(t)]
        if hits: print('0x%x size=%d  %s' % (f, size_of(f), ' | '.join(h[:80] for h in hits[:3])))
elif cmd == 'small':
    for f, F in sorted(funcs.items()):
        hits = [s for pc, s in small_strings(F) if arg in s]
        if hits: print('0x%x size=%d  %s' % (f, size_of(f), ' | '.join(hits[:6])))
elif cmd == 'call':
    for f, F in sorted(funcs.items()):
        hits = sorted(set(s for pc, s in F['calls'] if arg in s))
        if hits: print('0x%x size=%d  %s' % (f, size_of(f), ' '.join(hits[:8])))
elif cmd == 'sel':
    for f, F in sorted(funcs.items()):
        hits = sorted(set(s for pc, s in F['objc'] if s and arg in s))
        if hits: print('0x%x size=%d  %s' % (f, size_of(f), ' '.join(hits[:8])))
elif cmd == 'fn':
    for a in sys.argv[2:]:
        summary(int(a, 16))
elif cmd == 'callers':
    for c in sorted(callers_of(int(arg, 16))):
        print('0x%x size=%d' % (c, size_of(c)))
