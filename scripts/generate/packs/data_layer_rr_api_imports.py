"""Port API import collection and regex occurrences over decoded constant pieces."""
import json
from data_layer_rr_text import module, classes, character_class


def no_api_root_imports(source, target, header):
    original = module(source, 'no_api_root_imports')
    _, space, _ = classes()
    digits = character_class(str.isdecimal)
    text = header('no_api_root_imports', target)
    text = '\n'.join(line for line in text.splitlines() if not line.startswith('.scope ')) + '\n'
    text += '.scope include "impl/**/*.py", "data_layer/**/*.py", "_internal/**/*.py", "l????_*/**/*.py"\n'
    text += '.scope exclude "**/tests/**", "**/test/**", "**/.git/**", "**/.venv/**", "**/venv/**", "**/__pycache__/**", "**/tmp/**"\n'
    patterns = [original.ROOT_IMPORT_RE.pattern, original.ROOT_IMPORT_TEXT_RE.pattern]
    for i, regex in enumerate(patterns):
        # These patterns have no lookaround, boundaries, or case folding. Preserve consuming newlines.
        if regex.count('(?P<indent>') != 1 or regex.count(r'\s') != (1 if i else 0):
            raise ValueError('API root-import regex changed')
        regex = regex.replace('(?P<indent>', '(?P<indent_ignored>')
        regex = regex.replace(r'\s', space).replace('$', r'(?:\z|\n\z)')
        # Only match positions are needed; no capture is optional.
        regex = regex.replace('(?P<indent_ignored>[ \\t]*)', '[ \\t]*')
        text += f'.decl nari_regex_{i}(t: symbol, ordinal: number, start: number, end: number)\n'
        text += f'nari_regex_{i}(t, i, a, b) :- nari_piece(_, _, t, _, _), match_all({json.dumps(regex)}, t, i, a, b).\n'
    text += r'''
.decl nari_hidden(d: number)
nari_hidden(d) :- path_part(d, p), (p = "tests" ; p = "test").
nari_hidden(d) :- doc(d, _, _), option_item("pathParts", _, p), (p = "tests" ; p = "test").
.decl nari_framework_hidden(d: number)
nari_framework_hidden(d) :- path_part(d, p),
    (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp").
.decl nari_file(d: number)
nari_file(d) :- doc(d, _, _), option("entryPoint", entry),
    (entry = "check_file" ; entry = "check_file_content"), file_name(d, f), suffix(".py", f), !nari_hidden(d).
nari_file(d) :- doc(d, path, _), !option("entryPoint", _), file_name(d, f), suffix(".py", f),
    !nari_hidden(d), !nari_framework_hidden(d),
    (prefix("impl/", path) ; prefix("data_layer/", path) ; prefix("_internal/", path) ;
     match("l__DIGITS__{4}_[^/\n][^/]*/[\s\S]*", path)).
.decl nari_api_gate(d: number)
nari_api_gate(d) :- nari_file(d), !option("entryPoint", "check_file_content").
nari_api_gate(d) :- nari_file(d), option("entryPoint", "check_file_content"), option("providedSource", s),
    (contains("from api", s) ; contains("import api", s)).
.decl nari_active(d: number)
nari_active(d) :- nari_api_gate(d), !syntax_error(d, _, _), source(d, sp), text(sp, s),
    (contains("from api import", s) ; contains("import api", s)).
.decl nari_dotted(n: node, name: symbol)
nari_dotted(n, name) :- kind(n, "dotted_name"), name = join x by i sep "." : {
    child(n, part, i), kind(part, "identifier"), text(part, raw), x = @nfkc(raw) }.
.decl nari_module(i: node)
nari_module(i) :- kind(i, "import_from_statement"), field(i, "module_name", m), nari_dotted(m, "api").
nari_module(i) :- kind(i, "import_from_statement"), field(i, "module_name", r), kind(r, "relative_import"),
    child(r, m, _), nari_dotted(m, "api").
.decl nari_alias(i: node, index: number, name: symbol)
nari_alias(i, j, name) :- nari_module(i), child(i, a, j), field(i, "name", a), nari_dotted(a, name).
nari_alias(i, j, name) :- nari_module(i), child(i, a, j), field(i, "name", a), kind(a, "aliased_import"),
    field(a, "name", n), nari_dotted(n, name).
nari_alias(i, j, "*") :- nari_module(i), child(i, a, j), kind(a, "wildcard_import").
.violation RootImport(at: node, ordinal: number, pattern: number, occurrence: number, names: symbol) code "NARI001"
    "Deprecated root api import ({names}); use data_layer.* or explicit api.lXXXX_* imports"
RootImport(i, rank, 0, 0, names) :- nari_module(i), docof(i, d), nari_active(d), visit_index(i, rank),
    names = join name by index sep ", " : { nari_alias(i, index, name) }.
.decl nari_piece(n: node, piece: number, value: symbol, base_line: number, column: number)
nari_piece(n, piece, value, base, column) :- str_part(n, piece, value, _, _),
    str_part_start(n, piece, base, column), docof(n, d), nari_active(d).
.decl nari_text(n: node, piece: number, l: number, pattern: number, occurrence: number)
'''
    for i in range(2):
        # A match may consume the preceding newline. The indent starts after that newline.
        text += f'''nari_text(n, piece, base + strlen(@replace_all(@slice(t, 0, a), "[^\\\\n]", "")) + adjust,
    {i + 1}, occurrence) :- nari_piece(n, piece, t, base, _), nari_regex_{i}(t, occurrence, a, b),
    (@slice(t, a, a + 1) = "\\n", adjust = 1 ; @slice(t, a, a + 1) != "\\n", adjust = 0).
'''
    text += r'''
// Constants inside an interpolation precede the following outer Constant. A literal's own CST
// descendants provide this order; scan only matched literals, never all nodes in their file.
.decl nari_matched_piece(n: node, piece: number)
nari_matched_piece(n, piece) :- nari_text(n, piece, _, _, _).
.decl nari_part_anchor(n: node, piece: number, a: node)
nari_part_anchor(n, piece, n) :- nari_matched_piece(n, piece).
nari_part_anchor(n, piece, a) :- nari_matched_piece(n, piece), within(a, n).
.decl nari_part_before(n: node, piece: number, rank: number)
nari_part_before(n, piece, rank) :- nari_part_anchor(n, piece, a), str_part_start(n, piece, l, c),
    line(a, first), col(a, column), (first < l ; first = l, column <= c), visit_index(a, rank).
.decl nari_part_rank(n: node, piece: number, rank: number)
nari_part_rank(n, piece, rank) :- nari_matched_piece(n, piece), rank = max r : { nari_part_before(n, piece, r) }.
.violation RootExample(at: span, ordinal: number, piece: number, pattern: number, occurrence: number) code "NARI001"
    "String literal or doc example uses deprecated root api import; use data_layer.* or explicit api.lXXXX_* imports"
RootExample(@place(path, l), rank, piece, pattern, occurrence) :- nari_text(n, piece, l, pattern, occurrence),
    nari_part_rank(n, piece, rank), docof(n, d), doc(d, path, _).
'''
    from data_layer_rr_cli import api_scope
    return api_scope(text.replace('__DIGITS__', json.dumps(digits)[1:-1]))
