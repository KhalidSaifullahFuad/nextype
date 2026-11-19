#!/usr/bin/env python3
# Scratch: dump the getter around each call to a key decoder. Usage: getter.py <decoder-hex>...
import sys, os, pickle, bisect, subprocess, re
H=os.path.dirname(os.path.abspath(__file__))
D=pickle.load(open(H+'/work/scan.pkl','rb')); F=D['funcs']; fs=D['fstarts']
OBJ='/Library/Developer/CommandLineTools/usr/bin/llvm-objdump'; B='/Applications/Cotypist.app/Contents/MacOS/Cotypist'
for a in sys.argv[1:]:
    t=int(a,16)
    for f,X in F.items():
        for pc,tt in X['bl']:
            if tt!=t: continue
            end=fs[bisect.bisect_right(fs,f)]
            out=subprocess.run([OBJ,'-d','--no-show-raw-insn','--start-address=0x%x'%max(f,pc-0x30),'--stop-address=0x%x'%min(end,pc+0xc0),B],capture_output=True,text=True).stdout
            print('==== decoder %s caller fn %x call %x'%(a,f,pc))
            for l in out.splitlines()[6:]:
                l=re.sub(r'<[^>]*>','',l).rstrip()
                if l.strip(): print(l[:96])
