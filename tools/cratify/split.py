"""Per item of a file, its upward references: what a split of the file would move.

usage: split.py <edges.json> <file> <index dir>...
Prints each top-level-most enclosing item (fn, method, struct, ...) in the file that holds an upward reference:
its span, size, and the crates and items it reads above the file's crate.
"""
import collections, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from crate_map import ALLOW, ROOT, file_crates
from join import test_starts, KIND

def load(edges_json, indexes):
    F, D = file_crates(edges_json)
    where, spans = {}, collections.defaultdict(list)
    for idx in indexes:
        for row in open(os.path.join(idx, 'defs.tsv')):
            sym, f, l, c, el, ec, sl, sc, xl, xc, k = row.rstrip('\n').split('\t')
            if f in F and sym not in where and k != '29':
                where[sym] = f
            if f in F and sl and KIND.get(int(k)) in ('fn', 'method', 'struct', 'enum', 'trait', 'const', 'static', 'type', 'macro'):
                spans[f].append((int(sl), int(xl), sym, KIND.get(int(k))))
    refs = collections.defaultdict(list)
    for idx in indexes:
        for row in open(os.path.join(idx, 'refs.tsv')):
            f, l, c, ec, sym = row.rstrip('\n').split('\t')
            if f in F and sym in where: refs[f].append((int(l), int(c), sym))
    return F, D, where, {f: sorted(set(v)) for f, v in spans.items()}, refs

def report(F, D, where, SP, R, target, out=print):
    if not target.startswith('native/'): target = 'native/src/code_rule_dsl/' + target
    me = F[target]
    spans = SP.get(target, [])
    start = test_starts(target)
    def outer(line):
        best = None
        for s, e, sym, k in spans:
            if s <= line <= e and (best is None or s < best[0]): best = (s, e, sym, k)
        return best
    hits = collections.defaultdict(lambda: collections.defaultdict(set))
    seen = set()
    for l, c, sym in R.get(target, ()):
            if l >= start: continue
            b = F[where[sym]]
            if b == me or b in ALLOW[me] or (l, c) in seen: continue
            seen.add((l, c))
            o = outer(l)
            hits[o][b].add(sym.split('code_rule_dsl/')[-1])
    tot = 0
    for o, by in sorted(hits.items(), key=lambda kv: kv[0][0] if kv[0] else -1):
        if o:
            s, e, sym, k = o; tot += e - s + 1
            out(f'{s + 1:5}-{e + 1:<5} {e - s + 1:4} lines  {k:6} {sym.split("code_rule_dsl/")[-1]}')
        else:
            out('    (outside any item: use lines)')
        for b, items in sorted(by.items()):
            out(f'                 -> {b}: ' + ', '.join(sorted(items))[:300])
    if hits: out(f'{me}: {len(hits)} items, {tot} lines would move')
    return len(hits), tot

def main(edges_json, target, *indexes):
    F, D, where, SP, R = load(edges_json, indexes)
    if target == 'all':
        test = {os.path.relpath(p, ROOT): D['test'][p] for p in D['mods']}
        for f in sorted(F):
            if test[f]: continue
            buf = []
            n, tot = report(F, D, where, SP, R, f, buf.append)
            if n:
                print(f'=== {f.replace("native/src/code_rule_dsl/", "")}  [{F[f]}]')
                print('\n'.join(buf))
    else:
        report(F, D, where, SP, R, target)

main(*sys.argv[1:])
