import sys, collections
sys.argv = ['mine.py']
src = open('mine.py').read().replace("if __name__ == '__main__':", "if False:")
exec(src)
crate_of = make(MINE)
V = collections.defaultdict(collections.Counter)
for p, t, te, n in D['edges']:
    if te: continue
    a = crate_of(D['mods'][p]); b = crate_of(t)
    if a == b or b in ALLOW.get(a, set()): continue
    tgt = '::'.join(t.replace('crate::code_rule_dsl::', '').split('::')[:4])
    V[(a, b)][(p.split('native/src/')[-1].replace('code_rule_dsl/', ''), tgt)] += n
for (a, b), c in sorted(V.items(), key=lambda kv: -sum(kv[1].values())):
    print(f'### {a} -> {b}  ({sum(c.values())})')
    agg = collections.Counter()
    for (f, t), n in c.items(): agg[t] += n
    for t, n in agg.most_common(12):
        fs = sorted({f for (f, tt) in c if tt == t})
        print(f'   {n:3} {t}   <- {", ".join(fs[:3])}{" …" if len(fs) > 3 else ""}')
