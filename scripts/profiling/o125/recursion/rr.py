import re,sys
from collections import defaultdict
def parse(path):
    src=open(path).read()
    src=re.sub(r'//[^\n]*','',src)
    # directives (.decl, .output, ...) are one line each and end with no '.'
    src='\n'.join(l for l in src.split('\n') if not l.lstrip().startswith('.'))
    # statements end with '.' followed by whitespace/newline (not inside strings)
    stmts=[];cur='';instr=False;depth=0
    for ch in src:
        cur+=ch
        if ch=='"': instr=not instr
        elif not instr and ch in '([': depth+=1
        elif not instr and ch in ')]': depth-=1
        elif not instr and depth==0 and ch=='.':
            stmts.append(cur.strip());cur=''
    rules=[]
    for st in stmts:
        if st.startswith('.') or ':-' not in st: continue
        h,b=st[:-1].split(':-',1)
        lits=split_top(b)
        rules.append((h.strip(),[l.strip() for l in lits]))
    return rules
def split_top(s):
    out=[];cur='';d=0;q=False
    for ch in s:
        if ch=='"': q=not q
        if not q and ch in '([': d+=1
        if not q and ch in ')]': d-=1
        if ch==',' and d==0 and not q: out.append(cur);cur=''
        else: cur+=ch
    if cur.strip(): out.append(cur)
    return out
def rel(l):
    m=re.match(r'!?\s*([a-z_][a-z0-9_]*)\s*\(',l)
    return m.group(1) if m else None
if __name__=='__main__':
    rules=parse(sys.argv[1])
    dep=defaultdict(set)
    for h,b in rules:
        hr=rel(h)
        for l in b:
            r=rel(l)
            if r: dep[hr].add(r)
    # tarjan
    idx={};low={};st=[];on=set();comps=[];c=[0]
    sys.setrecursionlimit(10000)
    def sc(v):
        idx[v]=low[v]=c[0];c[0]+=1;st.append(v);on.add(v)
        for w in dep.get(v,()):
            if w not in idx: sc(w);low[v]=min(low[v],low[w])
            elif w in on: low[v]=min(low[v],idx[w])
        if low[v]==idx[v]:
            comp=[]
            while True:
                w=st.pop();on.discard(w);comp.append(w)
                if w==v: break
            comps.append(comp)
    for v in list(dep): 
        if v not in idx: sc(v)
    for comp in comps:
        if len(comp)>1 or comp[0] in dep[comp[0]]:
            rs=[(h,b) for h,b in rules if rel(h) in comp]
            print(len(comp),'rels',len(rs),'rules:',' '.join(sorted(comp)))
