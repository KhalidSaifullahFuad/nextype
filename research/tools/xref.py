#!/usr/bin/env python3
# Scratch: find every adrp+add/ldr (and bl) reference to given target addresses. Usage: xref.py addr...
import sys, struct, array, pickle, bisect
B='/Applications/Cotypist.app/Contents/MacOS/Cotypist'
data=open(B,'rb').read()
D=pickle.load(open(__import__('os').path.dirname(__import__('os').path.abspath(__file__))+'/work/scan.pkl','rb')); fs=D['fstarts']
TA,TFO,TSZ=0x1000029d0,0x29d0,0x1160cbc
w=array.array('I'); w.frombytes(data[TFO:TFO+TSZ])
targets=set(int(a,16) for a in sys.argv[1:])
adrp=[None]*32; hits=[]
for i,ins in enumerate(w):
    pc=TA+i*4
    if (ins&0x9F000000)==0x90000000:
        rd=ins&31; imm=((ins>>5)&0x7FFFF)<<2|((ins>>29)&3)
        if imm&(1<<20): imm-=1<<21
        adrp[rd]=(pc&~0xFFF)+(imm<<12); continue
    if (ins&0xFF800000)==0x91000000:
        rd=ins&31; rn=(ins>>5)&31; imm=(ins>>10)&0xFFF
        if adrp[rn] is not None:
            t=adrp[rn]+imm
            if t in targets: hits.append((pc,t,'add'))
        continue
    if (ins&0xFFC00000)==0xF9400000:
        rn=(ins>>5)&31; imm=((ins>>10)&0xFFF)*8
        if adrp[rn] is not None and adrp[rn]+imm in targets: hits.append((pc,adrp[rn]+imm,'ldr'))
        continue
    if (ins&0xFC000000)==0x94000000:
        imm=ins&0x3FFFFFF
        if imm&(1<<25): imm-=1<<26
        t=pc+imm*4
        if t in targets: hits.append((pc,t,'bl'))
for pc,t,k in hits:
    f=fs[bisect.bisect_right(fs,pc)-1]
    print('%x  in fn %x  -> %x (%s)'%(pc,f,t,k))
