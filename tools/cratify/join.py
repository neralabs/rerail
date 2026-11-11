"""Join SCIP indexes against the crate map.

usage: join.py <edges.json> <out dir> <decoded index dir>...

Writes, from every index given (their references are merged; a definition is taken from the first index
that has it):
  upward.tsv  every production reference that points up the crate DAG, one row per (using crate, item)
  tests.tsv   the same for test code (test files and inline #[cfg(test)] modules)
  widen.tsv   every item another crate reads that is not `pub`: where its visibility is, and what it is now
and prints a summary per crate pair.
"""
import collections, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from crate_map import ALLOW, ROOT, file_crates

KIND = {3: 'assoc_type', 8: 'const', 11: 'enum', 12: 'variant', 15: 'field', 17: 'fn', 25: 'macro', 26: 'method',
        29: 'module', 49: 'struct', 53: 'trait', 55: 'type', 80: 'static', 82: 'union'}
NO_VIS = {'variant', 'assoc_type'}  # never carry a visibility of their own

_lines = {}
def lines(f):
    if f not in _lines:
        _lines[f] = open(os.path.join(ROOT, f), encoding='utf-8', errors='ignore').read().split('\n')
    return _lines[f]

VIS = re.compile(r'\bpub(\s*\([^)]*\))?')
def visibility(f, l, c, sl, sc):
    """The visibility written between the item's start (sl, sc) and its name (l, c), and its position."""
    L = lines(f)
    best = None
    for i in range(sl, l + 1):
        text = L[i]
        lo = sc if i == sl else 0
        hi = c if i == l else len(text)
        seg = text[lo:hi]
        code = re.sub(r'//.*', '', seg)
        if '#[' in code and i < l:
            continue
        for m in VIS.finditer(code):
            best = (i, lo + m.start(), lo + m.end(), m.group(0).replace(' ', ''))
    return best

def test_starts(f):
    L = lines(f)
    for i, x in enumerate(L):
        if re.match(r'\s*#\[cfg\((test|all\(test)', x):
            for j in range(i + 1, min(i + 4, len(L))):
                if re.match(r'\s*(pub(\([^)]*\))?\s+)?mod\s+\w+\s*\{', L[j]):
                    return i
    return 10 ** 9

def main(edges_json, out, *indexes):
    F, D = file_crates(edges_json)
    test_file = {os.path.relpath(p, ROOT): D['test'][p] for p in D['mods']}
    defs = {}
    for idx in indexes:
        for row in open(os.path.join(idx, 'defs.tsv')):
            sym, f, l, c, el, ec, sl, sc, xl, xc, k = row.rstrip('\n').split('\t')
            if f in F and sym not in defs:
                defs[sym] = (f, int(l), int(c), int(sl) if sl else int(l), int(sc) if sc else 0, KIND.get(int(k), k))
    roots = {s for s, d in defs.items() if d[5] == 'module' and (s.endswith(' crate/') or s.endswith(' code_rule_dsl/'))}
    starts = {f: test_starts(f) for f in F}
    seen = set()
    up = collections.defaultdict(lambda: collections.defaultdict(set))
    uptest = collections.defaultdict(lambda: collections.defaultdict(set))
    reads = collections.defaultdict(set)
    for idx in indexes:
        for row in open(os.path.join(idx, 'refs.tsv')):
            f, l, c, ec, sym = row.rstrip('\n').split('\t')
            if f not in F or sym not in defs or sym in roots:
                continue
            key = (f, l, c, sym)
            if key in seen:
                continue
            seen.add(key)
            a = F[f]; d = defs[sym]; b = F[d[0]]
            if a == b:
                continue
            site = f'{f}:{int(l) + 1}'
            if b in ALLOW.get(a, set()):
                reads[sym].add(a)
            elif d[5] != 'module':
                (uptest if test_file.get(f) or int(l) >= starts[f] else up)[(a, b)][sym].add(site)
    os.makedirs(out, exist_ok=True)
    def dump(name, table):
        with open(os.path.join(out, name), 'w') as w:
            w.write('from\tto\tkind\titem\tdefined\tsites\n')
            for (a, b), items in sorted(table.items()):
                for sym, sites in sorted(items.items(), key=lambda kv: -len(kv[1])):
                    d = defs[sym]
                    w.write(f'{a}\t{b}\t{d[5]}\t{sym.split(" ", 4)[-1]}\t{d[0]}:{d[1] + 1}\t{" ".join(sorted(sites))}\n')
    dump('upward.tsv', up)
    dump('tests.tsv', uptest)
    with open(os.path.join(out, 'reads.tsv'), 'w') as w:
        w.write('file\tline\tkind\titem\tread_by\n')
        for sym, users in sorted(reads.items()):
            f, l, c, sl, sc, k = defs[sym]
            w.write(f'{f}\t{l + 1}\t{k}\t{sym.split(" ", 4)[-1]}\t{",".join(sorted(users))}\n')
    widen = []
    for sym, users in reads.items():
        f, l, c, sl, sc, k = defs[sym]
        if k in NO_VIS:
            continue
        v = visibility(f, l, c, sl, sc)
        if v and v[3] == 'pub':
            continue
        widen.append((f, sl, sc, l, c, k, v, sym, sorted(users)))
    with open(os.path.join(out, 'widen.tsv'), 'w') as w:
        w.write('file\tline\tcol\tkind\tnow\tvis_at\titem\tread_by\n')
        for f, sl, sc, l, c, k, v, sym, users in sorted(widen):
            at = f'{v[0] + 1}:{v[1]}-{v[2]}' if v else f'{l + 1}:{c}'
            w.write(f'{f}\t{l + 1}\t{c}\t{k}\t{v[3] if v else "private"}\t{at}\t{sym.split(" ", 4)[-1]}\t{",".join(users)}\n')
    print(f'items read across crates {len(reads)}; to widen {len(widen)}')
    print(f'upward (production): {sum(len(v) for v in up.values())} items; tests: {sum(len(v) for v in uptest.values())} items')
    for (a, b), items in sorted(up.items(), key=lambda kv: -len(kv[1])):
        tf = collections.Counter(defs[s][0].replace('native/src/code_rule_dsl/', '') for s in items)
        print(f'  {a:20} -> {b:20} {len(items):4}   ' + ', '.join(f'{x}({n})' for x, n in tf.most_common(4)))

if __name__ == '__main__':
    main(*sys.argv[1:])
