# expand.py <policy.rr> <out.rr>: the policy with its `#include`s read in (each file once), searched in the including
# file's directory, then docs/rule-syntax and docs/rule-syntax/lib, as the engine reads them. Run from the worktree.
import os,re,sys
PATHS=['docs/rule-syntax','docs/rule-syntax/lib']
def expand(p,seen):
    p=os.path.normpath(p)
    if p in seen: return ''
    seen.add(p);out=[]
    for l in open(p):
        m=re.match(r'\s*#include\s+"([^"]+)"',l)
        if not m: out.append(l);continue
        for d in [os.path.dirname(p)]+PATHS:
            q=os.path.join(d,m.group(1))
            if os.path.exists(q): out.append(expand(q,seen));break
        else: sys.exit(f'{p}: no {m.group(1)}')
    return ''.join(out)
if __name__=='__main__':
    open(sys.argv[2],'w').write(expand(sys.argv[1],set()))
