import json, collections, re, sys
D = json.load(open('edges.json'))
def short(m): return m.replace('crate::code_rule_dsl::', '')
LANGS = {
 'python': r'python|cpython|identifier|module_path|ruff|pyright|mypy|flake8|pylint',
 'php': r'php|blade|phpstan|psalm|phpcs|phpdoc',
 'ecma': r'ecma|eslint|typescript|javascript|tsx|import_target|npm_ignore|semver|module_graph|package_manifest|vue|biome|tsc|oxlint',
 'rust': r'rust|clippy',
 'sql': r'^sql$|sqlfluff|^value::sql$',
 'json': r'^json_parser$',
 'format': r'printf|scanf|rust_format|brace|format_args',
}
SQLFALSE = ('logic::sql', 'profile::sql', 'sql_executor', 'waves::owner', 'matcher::p_anchors', 'matcher::d_anchors', 'logic::profile', 'prefilter')
def lang_of(m):
    sm = short(m)
    if sm.startswith('language::adapter::sql') or sm.startswith('value::sql'): return 'sql'
    for seg in short(m).split('::'):
        for l, rx in LANGS.items():
            if l == 'sql': continue
            if re.search(rx, seg): return l
    return None
loc = collections.Counter()
for p, m in D['mods'].items():
    if not D['test'][p]: loc[lang_of(m) or 'core'] += D['loc'][p]
print('prod file lines by language:', dict(loc))
C = collections.Counter(); T = collections.defaultdict(collections.Counter)
for p, t, te, n in D['edges']:
    if te: continue
    a, b = lang_of(D['mods'][p]), lang_of(t)
    if a is None and b is not None:
        C[b] += n; T[b]['::'.join(short(t).split('::')[:2])] += n
print('core -> language-specific prod refs:', sum(C.values()), dict(C))
for l in C: print(' ', l, T[l].most_common(10))
if '-m' in sys.argv:
    for p, m in sorted(D['mods'].items()):
        if lang_of(m) and not D['test'][p]: pass
