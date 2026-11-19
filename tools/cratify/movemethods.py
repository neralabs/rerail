# movemethods.py <file> <child-stem> <doc> <method>...: move methods (with docs/attrs) out of their impl into
# <file-dir>/<file-stem>/<child>.rs as `impl <Type> { ... }` blocks (child module: private items stay visible).
import re,sys,os
p,child,doc=sys.argv[1:4]; want=set(sys.argv[4:])
L=open(p).read().split('\n')
meth=[]; cur=None; impl=None
for i,l in enumerate(L):
    if re.match(r'^impl\b',l): impl=l
    m=re.match(r'^    (?:pub\([^)]*\) |pub )?fn (\w+)',l)
    if m and impl:
        j=i
        while j>0 and (L[j-1].startswith('    ///') or L[j-1].startswith('    #[') or L[j-1].startswith('    //')): j-=1
        cur=[m.group(1),j,None,impl]; meth.append(cur)
    if cur and l=='    }' and cur[2] is None: cur[2]=i; cur=None
take=[m for m in meth if m[0] in want]
assert {m[0] for m in take}==want, want-{m[0] for m in take}
by_impl={}
drop=set()
for n,a,b,imp in take:
    by_impl.setdefault(imp,[]).append('\n'.join(L[a:b+1]))
    e=b+1
    if e<len(L) and L[e]=='' : e+=1
    drop.update(range(a,e))
out=[l for i,l in enumerate(L) if i not in drop]
stem=os.path.splitext(p)[0]
os.makedirs(stem,exist_ok=True)
body='//! '+doc+'\n\nuse super::*;\n'
for imp,ms in by_impl.items():
    hdr=imp.rstrip(' {')
    body+='\n'+hdr+' {\n'+'\n\n'.join(ms)+'\n}\n'
cp=os.path.join(stem,child+'.rs')
assert not os.path.exists(cp), cp
open(cp,'w').write(body)
text='\n'.join(out).rstrip('\n')+f'\n\n// {doc.split(":")[0]} (`{child}`, its crate\'s).\nmod {child};\n'
open(p,'w').write(text)
print(cp, sum(len(m.split(chr(10))) for ms in by_impl.values() for m in ms))
