# wf.py <program.rerail>...: which recursive components cannot hold a row up by itself, read off the rules (decision
# 0328, the static check levels could skip). A component is well-founded here when each relation has a measure
# column such that every recursive rule's own atom is strictly below (or every one strictly above) its head in the
# tree, or at the same node with no cycle of such steps, or a self-recursion whose number column steps by @add/@sub.
# Then no derivation cycle exists among its rows, so one probe a candidate is sound. Conservative: an atom it does
# not know (a helper between the two nodes) fails the component, and the reason is printed.
import itertools,os,re,sys
from collections import defaultdict,Counter
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
from rr import parse,rel,split_top
CORE=os.path.join(os.path.dirname(os.path.abspath(__file__)),'../../../../docs/rule-syntax/lib/core.rerail')
# (atom, below, above): atom(args) says args[below] is strictly below args[above] in the tree; SIB: document order
BELOW={'child':(1,0),'field':(2,0),'within':(0,1),'inside':(0,1),'has':(1,0),'nearest':(0,1),'within_stop':(0,1)}
SIB={'precedes':(1,0),'next_sibling':(1,0)}
def decls(text):
    t={}
    for m in re.finditer(r'\.decl\s+([a-z_0-9]+)\s*\(([^)]*)\)',re.sub(r'//[^\n]*','',text)):
        t[m.group(1)]=[a.split(':')[1].strip() for a in m.group(2).split(',') if a.strip()]
    return t
def args(l):
    l=l.strip().lstrip('!').strip();i=l.index('(');return [a.strip() for a in split_top(l[i+1:-1])]
def sccs(rules):
    dep=defaultdict(set)
    for h,b in rules:
        for x in b:
            if not x.startswith('!') and rel(x): dep[rel(h)].add(rel(x))
    idx={};low={};st=[];on=set();out=[];c=[0];sys.setrecursionlimit(100000)
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
            out.append(comp)
    for v in list(dep):
        if v not in idx: sc(v)
    return [c for c in out if len(c)>1 or c[0] in dep[c[0]]]
HELP={}   # (relation, i, j) -> step from column i to column j, read off the relation's own rules (`summarize`)
def comb(k,s):
    if k=='EQ': return s
    if s=='EQ': return k
    d1,d2=k[0],s[0]
    if d1!=d2 or d1=='S': return 'X'
    return d1 if (len(k)==1 or len(s)==1) else d1+'q'
def summarize(rules,types,rounds=3):
    """Each relation with two node columns whose every rule relates them the same way: 'D' (strictly below), 'Dq'
    (below or the same), 'U', 'Uq'. Self-recursion is assumed to repeat its non-recursive rules' step."""
    by=defaultdict(list)
    for h,b in rules: by[rel(h)].append((h,b))
    for _ in range(rounds):
        for r,rs in by.items():
            cols=[i for i,t in enumerate(types.get(r,[])) if t=='node']
            for i in cols:
                for j in cols:
                    if i==j: continue
                    ks=set()
                    for h,b in rs:
                        ha=args(h)
                        if len(ha)<=max(i,j): ks.add(None);break
                        recs=[args(x) for x in b if not x.startswith('!') and rel(x)==r]
                        if recs:
                            # a step from the head's column i to the recursive atom's, its column j the head's
                            for ra in recs:
                                if len(ra)<=max(i,j) or ra[j].lstrip('$')!=ha[j].lstrip('$'): ks.add(None);continue
                                ks.add(('R',order(b,ha[i].lstrip('$'),ra[i].lstrip('$'))))
                            continue
                        ks.add(order(b,ha[i].lstrip('$'),ha[j].lstrip('$')))
                    rk={k[1] for k in ks if isinstance(k,tuple)}
                    ks={k for k in ks if not isinstance(k,tuple)}
                    if None in rk or None in ks or not ks: continue
                    # base steps one way; recursive steps the same way (strict or not) keep it that way, not strict
                    # when a base step is the same node
                    if rk and not all(k!='EQ' and k[0]==next(iter({b[0] for b in ks if b!='EQ'} or {k[0]})) for k in rk): continue
                    if rk: ks=ks|{k[0]+'q' if 'EQ' in ks else k for k in rk}
                    dirs={k[0] for k in ks if k!='EQ'}
                    if len(dirs)!=1 or 'S' in dirs: continue
                    d=dirs.pop()
                    HELP[(r,i,j)]=d if ks=={d} else d+'q'
def order(body,x,y):
    """How y relates to x through the body's tree atoms and equalities: 'EQ', 'D' (y strictly below x), 'U', 'S'
    (after in document order), or None."""
    if x==y: return 'EQ'
    E=defaultdict(list)   # u -> (v, step): v is `step` of u
    for l in body:
        if l.startswith('!') or '(' not in l:
            m=re.fullmatch(r'\$?(\w+)\s*=\s*\$?(\w+)',l.strip())
            if m: E[m.group(1)].append((m.group(2),'EQ'));E[m.group(2)].append((m.group(1),'EQ'))
            continue
        r=rel(l);a=args(l)
        if r in BELOW and len(a)>max(BELOW[r]):
            b,u=a[BELOW[r][0]].lstrip('$'),a[BELOW[r][1]].lstrip('$')
            E[u].append((b,'D'));E[b].append((u,'U'))
        elif r and not r in BELOW:
            for (hr,i,j),k in HELP.items():
                if hr==r and len(a)>max(i,j):
                    E[a[i].lstrip('$')].append((a[j].lstrip('$'),k))
        if r in SIB and len(a)>1:
            b,u=a[SIB[r][0]],a[SIB[r][1]]
            E[u].append((b,'S'))
    seen={(x,'EQ')};st=[(x,'EQ')]
    while st:
        v,k=st.pop()
        for w,s in E[v]:
            nk=comb(k,s)
            if nk=='X' or (w,nk) in seen: continue
            seen.add((w,nk));st.append((w,nk))
    ks={k for v,k in seen if v==y and k!='EQ'}
    return ks.pop() if len(ks)==1 else ('EQ' if (y,'EQ') in seen else None)
def numstep(h,body,ha,ba):
    """A self-recursion whose number column moves by @add/@sub of the body's: the index climbs or falls."""
    for i,(t,u) in enumerate(zip(ha,ba)):
        for l in [t]+body:
            m=re.search(r'@(add|sub)\(\$?'+re.escape(u.lstrip('$'))+r'\s*,\s*1\)',l)
            if m and (t.startswith('@') or re.search(re.escape(t.lstrip('$'))+r'\s*=\s*@(add|sub)',l)): return 'N'+m.group(1)
    return None
def classify(comp,rules,types):
    rs=[(h,b) for h,b in rules if rel(h) in comp and any(rel(x) in comp for x in b if not x.startswith('!'))]
    cols={r:[i for i,t in enumerate(types.get(r,[])) if t=='node'] for r in comp}
    if any(not c for c in cols.values()): return False,'a relation with no node column'
    combos=itertools.product(*[cols[r] for r in comp])
    why=Counter()
    for n,choice in enumerate(combos):
        if n>4000: return False,'too many measures to try'
        m=dict(zip(comp,choice));labels=set();eq=defaultdict(set);ok=True
        for h,b in rs:
            ha=args(h)
            for x in b:
                if x.startswith('!') or rel(x) not in comp: continue
                ba=args(x);k=order(b,ha[m[rel(h)]].lstrip('$'),ba[m[rel(x)]].lstrip('$')) if len(ha)>m[rel(h)] and len(ba)>m[rel(x)] else None
                if k=='EQ' and rel(x)==rel(h) and numstep(h,b,ha,ba): k=numstep(h,b,ha,ba)
                if k is None: ok=False;why['an unknown step: '+re.sub(r'\d+','N',', '.join(sorted({rel(y) for y in b if rel(y) and rel(y) not in comp and not y.startswith('!')}))[:80])]+=1;break
                if k in ('EQ','Dq','Uq'):
                    eq[rel(x)].add(rel(h))
                    if k!='EQ': labels.add(k[0])
                else: labels.add(k)
            if not ok: break
        if not ok: continue
        if len(labels-{'Nadd','Nsub'})>1 or {'Nadd','Nsub'}<=labels: why['steps both ways']+=1;continue
        # no cycle of same-node steps
        state={}
        def cyc(v):
            state[v]=1
            for w in eq.get(v,()):
                if state.get(w)==1 or (w not in state and cyc(w)): return True
            state[v]=2;return False
        if any(v not in state and cyc(v) for v in list(eq)): why['a cycle at one node']+=1;continue
        return True,'+'.join(sorted(labels)) or 'none'
    return False,(why.most_common(1)[0][0] if why else 'no measure')
if __name__=='__main__':
    core=decls(open(CORE).read())
    tot=Counter();reasons=Counter();done=set()   # a library's component once, however many programs read it
    for p in sys.argv[1:]:
        rules=parse(p);types=dict(core);types.update(decls(open(p).read()));HELP.clear();summarize(rules,types)
        for comp in sccs(rules):
            if tuple(sorted(comp)) in done: continue
            done.add(tuple(sorted(comp)))
            ok,why=classify(sorted(comp),rules,types)
            tot[ok]+=1
            if not ok: reasons[why]+=1
            if os.environ.get('WF_ALL') or not ok: print(f'  {"yes" if ok else "no "} {"+".join(sorted(comp))[:70]}: {why}')
    print(f'well-founded {tot[True]} of {tot[True]+tot[False]} recursive components; reasons not: {reasons.most_common(8)}')
