import collections, re
src = open('mine.py').read().replace("if __name__ == '__main__':", "if False:")
exec(src)
lsrc = open('langs.py').read().split("loc = collections.Counter()")[0].replace("D = json.load(open('edges.json'))", "")
exec(lsrc)
base = make(MINE)
def crate(m):
    l = lang_of(m)
    if l in ('format',): l = 'rust' if 'rust' in m else None
    if l: return 'pack_' + l
    return base(m)
loc = collections.Counter()
for p, m in D['mods'].items():
    if not D['test'][p]: loc[crate(m)] += D['loc'][p]
loc['rrsyn'] = 2000
packs = [c for c in loc if c.startswith('pack_')]
DAG2 = dict(DAG); DAG2['metal'] = ['transducer', 'logic']
for c in packs: DAG2[c] = ['w', 'facts', 'lang']
DAG2['cpython'] = ['prof']
DAG2['top'] = ['w', 'sessions', 'metal', 'cpython'] + packs
A = closure(DAG2)
total = sum(v for k, v in loc.items() if k != 'store')
print({k: v for k, v in sorted(loc.items(), key=lambda kv: -kv[1])})
for c in sorted(loc, key=lambda c: -loc[c]):
    if c == 'store': continue
    deps = [d for d in loc if d == c or c in A.get(d, set())]
    n = sum(loc[d] for d in deps if d != 'store')
    print(f'{c:12} own {loc[c]:7} rebuilt {n:7} ({100*n/total:3.0f}%)')
