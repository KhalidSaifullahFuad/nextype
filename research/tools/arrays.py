#!/usr/bin/env python3
# Scratch: group static Swift [String] arrays in __DATA,__data whose elements point into a cstring range;
# find code referencing each array and the decoded keys that code uses.
import struct, array, pickle, bisect, os
H=os.path.dirname(os.path.abspath(__file__))
B='/Applications/Cotypist.app/Contents/MacOS/Cotypist'; d=open(B,'rb').read()
DA,DFO,DS=0x1014193e0,0x14193e0,None
ncmds=struct.unpack_from('<I',d,16)[0]; off=32
for _ in range(ncmds):
    cmd,sz=struct.unpack_from('<II',d,off)
    if cmd==0x19:
        nsect=struct.unpack_from('<I',d,off+64)[0]; so=off+72
        for _ in range(nsect):
            sn=d[so:so+16].rstrip(b'\0').decode(); sg=d[so+16:so+32].rstrip(b'\0').decode()
            a,s=struct.unpack_from('<QQ',d,so+32); fo=struct.unpack_from('<I',d,so+48)[0]
            if (sg,sn)==('__DATA','__data'): DA,DS,DFO=a,s,fo
            so+=80
    off+=sz
def cstr(va):
    o=va-0x100000000; e=d.find(b'\0',o,o+200); return d[o:e].decode('utf-8','replace')
def W(va): return struct.unpack_from('<Q',d,DFO+(va-DA))[0]
def sval(va):
    c,o=W(va),W(va+8)
    if (c>>60)==0xD:
        for t in ((o&0x0000FFFFFFFFFFFF)+0x20,(o&0xFFFFFFFFF)+0x100000000+0x20):
            if 0x1012a3b50<=t<0x1012ff9b0: return cstr(t)
    if (o>>56)&0xF0==0xE0:
        n=(o>>56)&0xF; return (c.to_bytes(8,'little')+o.to_bytes(8,'little')[:7])[:n].decode('latin1')
    return None
arrays=[]
va=DA
while va<DA+DS-0x30:
    cnt,cap=W(va+0x10),W(va+0x18)
    if 1<=cnt<=200 and cap==cnt*2 and va%8==0:
        els=[sval(va+0x20+i*16) for i in range(cnt)]
        if all(els) and sum('.' in e for e in els)>=max(1,cnt//2):
            arrays.append((va,els)); va+=0x20+cnt*16; continue
    va+=8
D=pickle.load(open(H+'/work/scan.pkl','rb')); fs=D['fstarts']; F=D['funcs']
dec={}
for l in open(H+'/work/deobf.tsv'):
    a,s=l.rstrip('\n').split('\t',1); dec[int(a,16)]=s
targets={a for a,_ in arrays}
TA,TFO,TSZ=0x1000029d0,0x29d0,0x1160cbc
w=array.array('I'); w.frombytes(d[TFO:TFO+TSZ]); adrp=[None]*32; refs={}
for i,ins in enumerate(w):
    pc=TA+i*4
    if (ins&0x9F000000)==0x90000000:
        rd=ins&31; imm=((ins>>5)&0x7FFFF)<<2|((ins>>29)&3)
        if imm&(1<<20): imm-=1<<21
        adrp[rd]=(pc&~0xFFF)+(imm<<12); continue
    if (ins&0xFF800000)==0x91000000:
        rn=(ins>>5)&31
        if adrp[rn] is not None and adrp[rn]+((ins>>10)&0xFFF) in targets:
            refs.setdefault(adrp[rn]+((ins>>10)&0xFFF),[]).append(fs[bisect.bisect_right(fs,pc)-1])
for a,els in arrays:
    fns=sorted(set(refs.get(a,[])))
    keys=[]
    for f in fns: keys+= [dec[t] for pc,t in F.get(f,{'bl':[]})['bl'] if t in dec]
    print('== array %x n=%d fns=%s keys=%s'%(a,len(els),['%x'%f for f in fns][:3],list(dict.fromkeys(keys))[:4]))
    print('   '+', '.join(els))
