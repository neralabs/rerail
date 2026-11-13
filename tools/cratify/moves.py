"""Which single-file moves cut the most upward (file, item) references: a suggestion list, not a decision.

usage: moves.py <edges.json> <index dir>...
"""
import collections, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from crate_map import ALLOW, ROOT, file_crates
from join import test_starts

def main(edges_json, *indexes):
    F, D = file_crates(edges_json)
    test = {os.path.relpath(p, ROOT): D['test'][p] for p in D['mods']}
    loc = {os.path.relpath(p, ROOT): D['loc'][p] for p in D['mods']}
    where = {}
    for idx in indexes:
        for row in open(os.path.join(idx, 'defs.tsv')):
            sym, f = row.split('\t', 2)[:2]
            k = row.rstrip('\n').rsplit('\t', 1)[1]
            if f in F and sym not in where and k != '29':
                where[sym] = f
    starts = {f: test_starts(f) for f in F}
    E = set()
    for idx in indexes:
        for row in open(os.path.join(idx, 'refs.tsv')):
            f, l, c, ec, sym = row.rstrip('\n').split('\t')
            if f not in F or sym not in where or test.get(f) or int(l) >= starts[f]:
                continue
            g = where[sym]
            if g != f:
                E.add((f, g))
    W = collections.Counter()
    for idx in indexes:
        pass
    # weight: distinct items per (user file, defining file)
    items = collections.defaultdict(set)
    for idx in indexes:
        for row in open(os.path.join(idx, 'refs.tsv')):
            f, l, c, ec, sym = row.rstrip('\n').split('\t')
            if (f, where.get(sym)) in E and not (int(l) >= starts.get(f, 1e9)):
                items[(f, where[sym])].add(sym)
    out = collections.defaultdict(list); inn = collections.defaultdict(list)
    for (f, g), s in items.items():
        out[f].append((g, len(s))); inn[g].append((f, len(s)))
    def up(a, b):
        return a != b and b not in ALLOW.get(a, set())
    def cost(f, c):
        n = 0
        for g, w in out[f]:
            if up(c, F[g]): n += w
        for h, w in inn[f]:
            if up(F[h], c): n += w
        return n
    crates = sorted(set(F.values()))
    res = []
    for f in F:
        if test[f]: continue
        now = cost(f, F[f])
        if now == 0: continue
        best = min(crates, key=lambda c: cost(f, c))
        b = cost(f, best)
        if b < now:
            res.append((b - now, f, F[f], best, now, b, loc[f]))
    for d, f, a, b, now, after, n in sorted(res)[:60]:
        print(f'{d:5}  {f.replace("native/src/code_rule_dsl/", ""):60} {a:20} -> {b:20} {now:4} -> {after:4}  ({n} lines)')
main(*sys.argv[1:])
