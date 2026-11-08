"""Extract node-text regex atoms from fusion-model queries_full.json (588 queries incl. utils).
Each atom: pattern, host kind(s) (same dict 'kind' or sibling all-list kind), source ('node' or 'value')."""
import json, sys, collections
Q = json.load(open(sys.argv[1]))
atoms = []
PATHK = {'filePath','path','name','from','to','grammar'}  # (rule, pattern, kinds tuple, source, active)
def kinds_of(d):
    k = d.get('kind')
    if isinstance(k, str): return [k]
    return []
def walk(x, inherited, rule, active, src, pk=''):
    if isinstance(x, dict):
        own = kinds_of(x)
        ctx = own or inherited
        if isinstance(x.get('regex'), str) and pk not in PATHK:
            atoms.append((rule, x['regex'], tuple(ctx), src, active))
        for k, v in x.items():
            if k == 'all' and isinstance(v, list):
                sib = [kk for e in v if isinstance(e, dict) for kk in kinds_of(e)]
                anyk = []
                for e in v:
                    if isinstance(e, dict) and isinstance(e.get('any'), list) and all(isinstance(a, dict) and set(a) == {'kind'} for a in e['any']):
                        anyk += [a['kind'] for a in e['any']]
                c2 = sib or anyk or ctx
                for e in v: walk(e, c2, rule, active, src)
            elif k == 'value':
                walk(v, ctx, rule, active, 'value', k)
            elif k in ('has', 'inside', 'follows', 'precedes', 'not', 'any'):
                walk(v, [] if k != 'not' and k != 'any' else ctx, rule, active, src, k)
            elif k not in ('regex', 'kind'):
                walk(v, [], rule, active, src, k)
    elif isinstance(x, list):
        for v in x: walk(v, inherited, rule, active, src)
for q in Q:
    walk(q['query'].get('rule'), [], q['rule'], q['active'], 'node')
    for u, body in (q.get('utils') or {}).items():
        walk(body, [], q['rule'], q['active'], 'node')
print('atoms', len(atoms))
dist = collections.Counter((p, s) for _, p, _, s, _ in atoms)
print('distinct (pattern,src)', len(dist), 'distinct patterns', len({p for _, p, _, _, _ in atoms}))
kc = collections.Counter()
for _, p, k, s, _ in atoms: kc[(s, k[0] if len(k) == 1 else ('multi' if k else 'none'))] += 1
print(kc.most_common(25))
json.dump([dict(rule=r, pattern=p, kinds=list(k), source=s, active=a) for r, p, k, s, a in atoms], open(sys.argv[2], 'w'))
