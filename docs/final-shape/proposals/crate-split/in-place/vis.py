import sys, collections
src = open('mine.py').read().replace("if __name__ == '__main__':", "if False:")
exec(src)
crate_of = make(MINE)
modset = set(D['mods'].values())
items = collections.defaultdict(set); mods = collections.defaultdict(set)
for p, t, te, n in D['edges']:
    a = crate_of(D['mods'][p]); b = crate_of(t)
    if a == b or b.startswith('?') or b not in ALLOW.get(a, set()): continue
    s = t.split('::')
    # modules along the path within crate b
    for k in range(3, len(s) + 1):
        pref = '::'.join(s[:k])
        if pref in modset: mods[b].add(pref)
    if t not in modset: items[b].add(t)
print('crate       items  modules')
for c in sorted(set(items) | set(mods)): print(f'{c:11} {len(items[c]):5} {len(mods[c]):6}')
print('total', sum(map(len, items.values())), sum(map(len, mods.values())))
