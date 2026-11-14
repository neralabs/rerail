import sys, collections
src = open('mine.py').read().replace("if __name__ == '__main__':", "if False:")
exec(src)
crate_of = make(MINE)
F = collections.defaultdict(collections.Counter); L = {}
for p, t, te, n in D['edges']:
    if not te: continue
    a = crate_of(D['mods'][p]); b = crate_of(t)
    if a == b or b in ALLOW.get(a, set()) or b.startswith('?') or a == 'top': continue
    F[a][p] += n
for c, fs in sorted(F.items()):
    lines = sum(D['loc'][p] for p in fs)
    print(f'{c:11} files {len(fs):3} lines {lines:6} refs {sum(fs.values()):4}')
