import json,sys,collections,os,re
RULES=sys.argv[1]; names=sys.argv[2:]
def root_kinds(rule):
    # top-level kind(s) of an AST rule
    if not isinstance(rule,dict): return set()
    if 'kind' in rule: return {rule['kind']}
    if 'all' in rule:
        for r in rule['all']:
            k=root_kinds(r)
            if k: return k
    if 'any' in rule:
        s=set()
        for r in rule['any']: s|=root_kinds(r)
        return s
    if 'matches' in rule: return {'util:'+str(rule['matches'])[:30]}
    if 'pattern' in rule: return {'pattern'}
    return set()
def feats(rule,acc):
    if isinstance(rule,dict):
        for k,v in rule.items():
            if k in ('inside','has','precedes','follows'):
                sb=v.get('stopBy','neighbor') if isinstance(v,dict) else None
                acc[k+':'+('rule' if isinstance(sb,dict) else str(sb))]+=1
            elif k in ('not','regex','matches','calls','resolves','binds','count','nthChild','compare','arguments','parse','pattern','field','transform'):
                acc[k]+=1
            feats(v,acc)
    elif isinstance(rule,list):
        for x in rule: feats(x,acc)
for n in names:
    d=json.load(open(os.path.join(RULES,n+'.json')))
    print('#',n, 'bytes',os.path.getsize(os.path.join(RULES,n+'.json')))
    utils=d['clauses'][0].get('define',{}) if 'define' in d['clauses'][0] else {}
    for c in d['clauses']:
        kind=[k for k in c if k in ('forbid','relational','define','budget','require_each','select_when','violation')]
        if 'relational' in c:
            r=c['relational']; st=r.get('stages',[])
            rec=sum(1 for s in st if 'RECURSIVE' in json.dumps(s).upper())
            reach=sum(1 for s in st if 'reachability' in s)
            sqlb=len(json.dumps(r))
            ins=r.get('inputs',{})
            rk=collections.Counter()
            fa=collections.Counter()
            for k,v in ins.items():
                q=v.get('evidence',{}).get('query',{}).get('ast',{}).get('rule')
                for x in root_kinds(q): rk[x]+=1
                feats(q,fa)
            print(f"  {c.get('id')}: relational inputs={len(ins)} stages={len(st)} recursive={rec} reach={reach} syntax={'syntax' in r} refs={'references' in r} bytes={sqlb}")
            print(f"     input roots={dict(rk)}")
            print(f"     input feats={dict(fa)}")
        elif 'forbid' in c:
            q=c['forbid'].get('ast',{}).get('rule') if isinstance(c['forbid'],dict) else None
            fa=collections.Counter(); feats(q,fa)
            print(f"  {c.get('id')}: forbid roots={root_kinds(q)} feats={dict(fa)} bytes={len(json.dumps(c))}")
        elif 'budget' in c:
            print(f"  {c.get('id')}: budget {json.dumps(c['budget'])[:300]}")
        elif 'define' in c:
            dd=c['define']; print(f"  define: keys={list(dd)[:5]} utils={len(dd.get('utils',{}))} bytes={len(json.dumps(dd))}")
