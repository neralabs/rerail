"""file -> crate, from the module tree (modtree.py's edges.json) and crate-map.tsv."""
import json, os, sys
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
DAG = {
    'rerail-prof': [], 'rerail-rr-syntax': [], 'rerail-store': [],
    'rerail-lang': ['rerail-prof', 'rerail-rr-syntax'],
    'rerail-facts': ['rerail-lang', 'rerail-store'],
    'rerail-rr': ['rerail-facts', 'rerail-rr-syntax'],
    'rerail-logic': ['rerail-rr', 'rerail-facts'],
    'rerail-sessions': ['rerail-logic'],
    'rerail-transducer': ['rerail-w'],
    'rerail-w': ['rerail-facts'],
    'rerail-device-metal': ['rerail-transducer', 'rerail-logic'],
    'rerail-lang-python': ['rerail-w'], 'rerail-lang-ecma': ['rerail-w'], 'rerail-lang-php': ['rerail-w'],
    'rerail-lang-rust': ['rerail-w'], 'rerail-lang-sql': ['rerail-w'], 'rerail-lang-json': ['rerail-w', 'rerail-transducer'],
}
for e in filter(None, os.environ.get('EXTRA_DAG', '').split(',')):
    a, b = e.split('>'); DAG[a].append(b)
DAG['rerail-engine'] = [c for c in DAG]
def closure():
    A = {}
    def go(c, seen):
        for d in DAG.get(c, ()):
            if d not in seen: seen.add(d); go(d, seen)
        return seen
    for c in DAG: A[c] = go(c, set())
    return A
ALLOW = closure()
def load_map():
    M = {}
    # crate-map.tsv, then the generated test hosts (hosts.py), which override it
    files = [os.path.join(HERE, 'crate-map.tsv')] + [p for p in [os.path.join(HERE, 'test-hosts.tsv')] if os.path.exists(p)]
    for line in (l for f in files for l in open(f)):
        line = line.split('#')[0].rstrip('\n')
        if not line.strip(): continue
        k, v = line.split('\t'); M[k.strip()] = v.strip()
    return M
def crate_of_module(m, D=None):
    """The crate of module path `m` (crate::code_rule_dsl::... or crate::...) by the map alone."""
    M = load_map()
    s = m[len('crate::code_rule_dsl'):].lstrip(':') or 'dsl_root' if m.startswith('crate::code_rule_dsl') else m
    segs = s.split('::')
    for k in range(len(segs), 0, -1):
        key = '::'.join(segs[:k])
        if key in M: return M[key]
    return None
def module_of_file(rel, D):
    """A file's module path, for files that do not exist yet too: its parent module's path plus its stem."""
    for p, m in D['mods'].items():
        if os.path.relpath(p, ROOT) == rel: return m
    d, f = os.path.split(rel); stem = os.path.splitext(f)[0]
    for cand in (d + '.rs', os.path.join(d, 'mod.rs')):
        for p, m in D['mods'].items():
            if os.path.relpath(p, ROOT) == cand: return m + '::' + stem
    return None
def file_crates(edges_json):
    D = json.load(open(edges_json)); M = load_map(); out = {}
    for p, m in D['mods'].items():
        rel = os.path.relpath(p, ROOT)
        if m.startswith('crate::code_rule_dsl'):
            s = m[len('crate::code_rule_dsl'):].lstrip(':') or 'dsl_root'
        else:
            s = m
        segs = s.split('::')
        c = None
        for k in range(len(segs), 0, -1):
            key = '::'.join(segs[:k])
            if key in M: c = M[key]; break
        if c is None: c = 'rerail-engine' if not m.startswith('crate::code_rule_dsl') else '?' + segs[0]
        if D['mac'][p] and s.startswith('transducer::packed::'): c = 'rerail-device-metal'
        out[rel] = c
    return out, D
if __name__ == '__main__':
    F, D = file_crates(sys.argv[1])
    import collections
    loc = collections.Counter(); tl = collections.Counter()
    for p, m in D['mods'].items():
        rel = os.path.relpath(p, ROOT); (tl if D['test'][p] else loc)[F[rel]] += D['loc'][p]
    for c in sorted(set(F.values()), key=lambda c: -loc[c]): print(f'{c:22} prod {loc[c]:7} test {tl[c]:7}')
