import sys, collections
src = open('mine.py').read().replace("if __name__ == '__main__':", "if False:")
exec(src)
crate_of = make(MINE)
loc = collections.Counter()
for p, m in D['mods'].items():
    if not D['test'][p]: loc[crate_of(m)] += D['loc'][p]
loc['rrsyn'] = 2000; loc['store'] = 14725
total = sum(v for k, v in loc.items() if k != 'store')
order = ['prof', 'rrsyn', 'cpython', 'lang', 'facts', 'rr', 'logic', 'sessions', 'transducer', 'metal', 'w', 'top']
for c in order:
    deps = [d for d in order if d == c or c in ALLOW.get(d, set())]
    n = sum(loc[d] for d in deps)
    print(f'{c:11} own {loc[c]:7}  rebuilt {n:7} ({100*n/total:3.0f}% of {total})  {" ".join(d for d in deps if d != c)}')
