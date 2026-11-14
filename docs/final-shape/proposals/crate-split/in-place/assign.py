import json, collections, sys
D = json.load(open('edges.json'))
# Tim's §11.4 recommended DAG (rr-ast inside rr-syntax for us, base=prof)
TIM = """
prof: prof phase_clock logic::profile
cpython: cpython
rr: logic::front logic::magic logic::rr
facts: budget facts identifier json language lines logic::plan::sketch logic::strings module_graph module_path npm_ignore package_manifest producers semver sql_profile tape text value::python::words value::text_regex tree
logic: logic sql_executor typing waves::owner waves::schedule waves::storage threads logic::rr::from_tape
backends: demand residency space transducer transducer::packed
w: blade calls compile embed eval findings import_target language::adapter literal matcher model origin php_literal php_view placement query scope soundness stats suppress value violations vue workspace facts::facts_a facts::producers::python::import_screen facts::producers::python::python_facts hooks test_support
top: dsl_root slice tools prefilter waves waves::matches waves::findings waves::policies waves::relational logic::convert logic::plan::corpus::references transducer::packed::compile_matcher transducer::semantic_facts
"""
def parse(spec):
    m = {}
    for line in spec.strip().splitlines():
        c, mods = line.split(':', 1)
        for x in mods.split(): m[x] = c.strip()
    return m
def make(spec):
    M = parse(spec)
    def crate_of(mod):
        s = mod.split('::')
        if s[:2] != ['crate', 'code_rule_dsl']:
            if len(s) > 1 and s[1] in ('content_store', 'git_workspace', 'probes'): return 'store'
            if len(s) > 1 and s[1] in M: return M[s[1]]
            return 'top'
        rest = s[2:]
        if not rest: return M.get('dsl_root', 'top')
        for k in range(len(rest), 0, -1):
            key = '::'.join(rest[:k])
            if key in M: return M[key]
        return '?' + rest[0]
    return crate_of
def run(spec, allow, label, detail=False):
    crate_of = make(spec)
    loc = collections.Counter(); tloc = collections.Counter(); unk = collections.Counter()
    for p, m in D['mods'].items():
        c = crate_of(m)
        if c.startswith('?'): unk[c] += D['loc'][p]
        (tloc if D['test'][p] else loc)[c] += D['loc'][p]
    V = collections.Counter(); VT = collections.Counter(); VX = collections.defaultdict(collections.Counter)
    for p, t, te, n in D['edges']:
        a = crate_of(D['mods'][p]); b = crate_of(t)
        if a == b or b.startswith('?') or a.startswith('?'): continue
        if b in allow.get(a, set()): continue
        if te: VT[(a, b)] += n
        else:
            V[(a, b)] += n
            src = D['mods'][p].replace('crate::code_rule_dsl::', '')
            VX[(a, b)][src + ' -> ' + t.replace('crate::code_rule_dsl::', '')] += n
    print('==', label)
    print('unassigned', dict(unk))
    print('prod lines', dict(loc)); print('test lines', dict(tloc))
    print('prod violations', sum(V.values()), 'test violations', sum(VT.values()))
    for (a, b), n in V.most_common(): 
        print(f'  {a:9}->{b:9} {n:4}  test {VT[(a,b)]}')
        if detail:
            for x, k in VX[(a, b)].most_common(6): print('        ', k, x)
    return V, VX
def closure(direct):
    A = {}
    def go(c, seen):
        for d in direct.get(c, ()):
            if d not in seen: seen.add(d); go(d, seen)
        return seen
    for c in direct: A[c] = go(c, set())
    return A
TIM_ALLOW = closure({'prof': [], 'store': [], 'cpython': ['prof'], 'rr': ['prof', 'facts_ast'], 'facts': ['prof', 'store'],
  'logic': ['rr', 'facts'], 'backends': ['facts', 'prof'], 'w': ['facts'], 'top': ['w', 'logic', 'backends', 'cpython', 'rr', 'facts', 'prof', 'store']})
if __name__ == '__main__':
    run(TIM, TIM_ALLOW, 'Tim 11.4 at HEAD', detail='-d' in sys.argv)
