# prov.py <policy.rerail> <out.rerail> <meta.json> (run from this directory): adds drv_i rules (a derivation's whole binding) for every rule of a
# recursive component, and writes what each drv column means.
import sys,re,json,os
from rr import parse,rel,split_top
src=open(sys.argv[1]).read()
sys.argv=[sys.argv[0],sys.argv[1]]+sys.argv[2:]
rules=parse(sys.argv[1])
from collections import defaultdict
dep=defaultdict(set)
for h,b in rules:
    for l in b:
        r=rel(l)
        if r: dep[rel(h)].add(r)
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
rec=[sorted(c) for c in comps if len(c)>1 or c[0] in dep[c[0]]]
types0={}
for m in re.finditer(r'\.decl\s+([a-z_0-9]+)\s*\(([^)]*)\)',re.sub(r'//[^\n]*','',src)):
    types0[m.group(1)]=[a.split(':')[1].strip() for a in m.group(2).split(',') if a.strip()]
# A lattice relation's rows are a join of its derivations' values, not its derivations' heads: left out (and
# file-local where measured: workload.rerail's `cv`).
rec=[c for c in rec if not any('<' in t for r in c for t in types0.get(r,[]))]
types={}
# The engine's own declarations (core.rerail) first: a .rerail program reads `kind`, `field`, ... undeclared.
core=os.path.join(os.path.dirname(os.path.abspath(__file__)),'../../../../docs/rule-syntax/lib/core.rerail')
for m in re.finditer(r'\.decl\s+([a-z_0-9]+)\s*\(([^)]*)\)',re.sub(r'//[^\n]*','',open(core).read())):
    types[m.group(1)]=[a.split(':')[1].strip() for a in m.group(2).split(',') if a.strip()]
for m in re.finditer(r'\.decl\s+([a-z_0-9]+)\s*\(([^)]*)\)',re.sub(r'//[^\n]*','',src)):
    types[m.group(1)]=[a.split(':')[1].strip() for a in m.group(2).split(',') if a.strip()]
def args_of(l):
    l=l.strip().lstrip('!').strip()
    i=l.index('(');return l[:i].strip(),split_top(l[i+1:-1])
out=[];meta=[]
n=0
for ci,comp in enumerate(rec):
    for h,b in rules:
        if rel(h) not in comp: continue
        cols=[];ctype=[];extra=[]
        def var(name,t):
            if name not in cols: cols.append(name);ctype.append(t)
            return cols.index(name)
        fresh=[0]
        def term(t,ty):
            t=t.strip()
            if t=='_':
                fresh[0]+=1;v=f'W{fresh[0]}';var(v,ty);return v,('col',cols.index(v))
            if t.startswith('"') or re.fullmatch(r'-?\d+',t) or t.startswith('@'):
                fresh[0]+=1;v=f'K{fresh[0]}';var(v,ty);extra.append(f'{v} = {t}');return v,('col',cols.index(v))
            return t,('col',var(t,ty))
        lits=[];body=[]
        hr,hargs=args_of(h)
        # positive atoms first (they bind), then the rest
        for l in b:
            l=l.strip()
            if l.startswith('!') or '!=' in l or ' = ' in l: continue
            # A comparison, or a built-in test (`prefix`, `match`, ...): a filter on bound columns, kept as written.
            if '(' not in l or args_of(l)[0] not in types: body.append(l);continue
            r,args=args_of(l)
            # A built-in over named sets (`nearest(n, a, set)`, `inside`, `has`, ...): a relation per set, its columns
            # the others; copied below as `cp_<rel>_<sets>` from the same call.
            if 'relation' in types[r]:
                ks=[a.strip() for a,t in zip(args,types[r]) if t=='relation']
                name=r+'_'+'_'.join(re.sub(r'\W','_',k) for k in ks)
                new=[];cs=[]
                for a,t in zip(args,types[r]):
                    if t=='relation': new.append(a.strip());continue
                    x,cc=term(a,t);new.append(x);cs.append(cc)
                body.append(f'{r}({", ".join(new)})')
                lits.append({'rel':name,'neg':False,'args':cs,'call':[r,[a.strip() if t=='relation' else None for a,t in zip(args,types[r])],[t for t in types[r] if t!='relation']]})
                continue
            ty=types[r];new=[];cs=[]
            for j,a in enumerate(args):
                x,cc=term(a,ty[j]);new.append(x);cs.append(cc)
            body.append(f'{r}({", ".join(new)})');lits.append({'rel':r,'neg':False,'args':cs,'own':r in comp})
        for l in b:
            l=l.strip()
            if '!=' in l or ' = ' in l and not l.startswith('!'): body.append(l);continue
            if not l.startswith('!'): continue
            if args_of(l)[0] not in types or 'relation' in types[args_of(l)[0]]: body.append(l);continue
            r,args=args_of(l);ty=types[r];new=[];cs=[]
            for j,a in enumerate(args):
                a=a.strip()
                if a=='_': new.append('_');cs.append(None)
                else:
                    x,cc=term(a,ty[j]);new.append(x);cs.append(cc)
            body.append(f'!{r}({", ".join(new)})');lits.append({'rel':r,'neg':True,'args':cs,'own':r in comp})
        hcs=[]
        hty=types[hr]
        before=len(extra)
        for j,a in enumerate(hargs):
            x,cc=term(a,hty[j]);hcs.append(cc)
        # A head's computed column (`@dot(p, x)`) stays in the head, as the rule has it: a body equation on a
        # function the native engine runs only as a head's (`@cat`) is refused under RERAIL_SQL_STRICT.
        made={e.split(' = ',1)[0]:e.split(' = ',1)[1] for e in extra[before:] if e.split(' = ',1)[1].startswith('@')}
        extra=[e for e in extra if e.split(' = ',1)[0] not in made]
        name=f'drv_{n}';n+=1
        decl=', '.join(f'{cols[i].lower()}_{i}: {ctype[i]}' for i in range(len(cols)))
        sub=lambda s: re.sub(r'\b([A-Za-z_][A-Za-z0-9_]*)\b',lambda m: (m.group(1)) ,s)
        out.append(f'.decl {name}({decl})')
        out.append(f'{name}({", ".join(made.get(c,c) for c in cols)}) :- {", ".join(body+extra)}, prov_all(_).')
        out.append(f'.output {name}')
        meta.append({'drv':name,'comp':ci,'head':hr,'hargs':hcs,'lits':lits,'ncols':len(cols),'rule':h+' :- '+', '.join(b)})
cps=[]
seen=set()
for m in meta:
    for l in m['lits']:
        r=l['rel']
        if l.get('own') or r in seen: continue
        seen.add(r)
        if 'call' in l:
            f,ks,ts=l['call'];vs=[f'v{i}' for i in range(len(ts))];it=iter(vs)
            call=', '.join(k if k is not None else next(it) for k in ks)
            cps+=[f'.decl cp_{r}('+', '.join(f'{v}: {t}' for v,t in zip(vs,ts))+')',f'cp_{r}({", ".join(vs)}) :- {f}({call}), prov_all(_).',f'.output cp_{r}']
        else:
            vs=[f'v{i}' for i in range(len(types[r]))]
            cps+=[f'.decl cp_{r}('+', '.join(f'{v}: {t}' for v,t in zip(vs,types[r]))+')',f'cp_{r}({", ".join(vs)}) :- {r}({", ".join(vs)}), prov_all(_).',f'.output cp_{r}']
out+=cps
print(len(seen),'relations copied')
head='.decl prov_all(n: number)\n// Over every document: a cross-file fact, so every drv rule runs in the cross part, over whole relations.\nprov_all(n) :- n = count : { doc(_, _, _) }.'
open(sys.argv[2],'w').write(src+'\n// ---- provenance (prov.py) ----\n'+head+'\n'+'\n'.join(out)+'\n')
json.dump({'comps':rec,'drv':meta},open(sys.argv[3],'w'),indent=0)
print(len(meta),'drv rules over',len(rec),'components')
