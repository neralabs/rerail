"""Translate the facade's exact top-level AST whitelist without evaluating values."""
import json
from data_layer_rr_text import module


def data_layer_facade_purity(source, target, header):
    original = module(source, 'data_layer_facade_purity')
    text = header('data_layer_facade_purity', target)
    text += '.scope dirs option("apiInputDirs")\n'
    text += '.decl fac_default(i: number, path: symbol)\n'
    for index, path in enumerate(original.DEFAULT_TARGETS):
        text += f'fac_default({index}, {json.dumps(path.relative_to(original.REPO_ROOT).as_posix())}).\n'
    text += '.decl fac_message(code: symbol, message: symbol)\n'
    for code, message in original.VIOLATIONS.items():
        text += f'fac_message({json.dumps(code)}, {json.dumps(message)}).\n'
    return text + r'''
.decl fac_trigger(d: number)
fac_trigger(d) :- source(d, sp), text(sp, s),
    (contains("def ", s) ; contains("class ", s) ; contains("impl.", s) ; contains("=", s) ; contains("(", s)).
.decl fac_selected(d: number)
fac_selected(d) :- doc(d, _, _), option("entryPoint", entry), (entry = "lint_file" ; entry = "lint_targets").
fac_selected(d) :- doc(d, path, _), !option("entryPoint", _), fac_default(_, path).
.decl fac_document(d: number)
fac_document(d) :- fac_selected(d), file_name(d, f), suffix(".py", f), fac_trigger(d), !syntax_error(d, _, _).
.decl fac_top(st: node, i: number)
fac_top(st, i) :- kind(m, "module"), docof(m, d), fac_document(d), body_stmt(m, st, i).
.decl fac_assignment(st: node, a: node)
fac_assignment(st, a) :- fac_top(st, _), asgstmt(a, st).
.decl fac_target_name(t: node)
fac_target_name(t) :- kind(t, "identifier").
fac_target_name(t) :- kind(t, "parenthesized_expression"), child(t, c, _), !kind(c, "comment"), fac_target_name(c).
fac_target_name(t) :- kind(t, "tuple_pattern"), !op(t, ","), child(t, c, _), !kind(c, "comment"), fac_target_name(c).
.decl fac_bad_target(a: node)
fac_bad_target(a) :- fac_assignment(_, a), assign_target(a, t), !fac_target_name(t).

// Visit only the value edges on which _is_allowed_value recurses. Unsupported nodes stop the walk.
.decl fac_value_edge(e: node, c: node)
fac_value_edge(e, c) :- kind(e, "parenthesized_expression"), child(e, c, _), !kind(c, "comment").
fac_value_edge(e, c) :- kind(e, "attribute"), field(e, "object", c).
fac_value_edge(e, c) :- (kind(e, "list") ; kind(e, "tuple") ; kind(e, "set") ; kind(e, "expression_list")),
    child(e, c, _), !kind(c, "comment").
fac_value_edge(e, c) :- kind(e, "dictionary"), child(e, p, _), kind(p, "pair"),
    (field(p, "key", c) ; field(p, "value", c)).
// Dict unpacking has a None key in ast.Dict: None is allowed, and the value is checked.
fac_value_edge(e, c) :- kind(e, "dictionary"), child(e, s, _), kind(s, "dictionary_splat"), child(s, c, _), !kind(c, "comment").
.decl fac_value_reach(root: node, e: node)
fac_value_reach(v, v) :- fac_assignment(_, a), assign_value(a, v).
fac_value_reach(root, c) :- fac_value_reach(root, e), fac_value_edge(e, c).
.decl fac_supported(e: node)
fac_supported(e) :- kind(e, k), const_kind(k).
fac_supported(e) :- (string_form(e, "str") ; string_form(e, "bytes")), !fstring(e).
fac_supported(e) :- kind(e, "identifier").
fac_supported(e) :- kind(e, "attribute").
fac_supported(e) :- kind(e, "parenthesized_expression").
fac_supported(e) :- kind(e, "list").
fac_supported(e) :- kind(e, "tuple").
fac_supported(e) :- kind(e, "expression_list").
fac_supported(e) :- kind(e, "set").
fac_supported(e) :- kind(e, "dictionary").
// A valid non-f-string concatenation is one str or bytes Constant. No decoded value is needed.
fac_supported(e) :- kind(e, "concatenated_string"), !fstring(e).
.decl fac_bad_value(root: node)
fac_bad_value(root) :- fac_value_reach(root, e), !fac_supported(e).
.decl fac_bad_assignment(a: node)
fac_bad_assignment(a) :- fac_bad_target(a).
fac_bad_assignment(a) :- fac_assignment(_, a), assign_value(a, v), fac_bad_value(v).

// Normalize identifiers from individual CST components; relative-import dots are not node.module.
.decl fac_component(n: node, i: number, s: symbol)
fac_component(n, i, @nfkc(raw)) :- kind(n, "dotted_name"), child(n, c, i), kind(c, "identifier"), text(c, raw).
.decl fac_dotted(n: node, s: symbol)
fac_dotted(n, s) :- kind(n, "dotted_name"), s = join part by i sep "." : { fac_component(n, i, part) }.
.decl fac_from(st: node, module: symbol)
fac_from(st, module) :- (kind(st, "import_from_statement") ; kind(st, "future_import_statement")),
    field(st, "module_name", n), fac_dotted(n, module).
fac_from(st, module) :- kind(st, "import_from_statement"), field(st, "module_name", r), kind(r, "relative_import"),
    child(r, n, _), fac_dotted(n, module).
.decl fac_import(st: node, alias: node, module: symbol)
fac_import(st, n, module) :- kind(st, "import_statement"), field(st, "name", n), fac_dotted(n, module).
fac_import(st, n, module) :- kind(st, "import_statement"), field(st, "name", n), kind(n, "aliased_import"),
    field(n, "name", d), fac_dotted(d, module).
.decl fac_import_statement(st: node)
fac_import_statement(st) :- kind(st, "import_statement").
fac_import_statement(st) :- kind(st, "import_from_statement").
fac_import_statement(st) :- kind(st, "future_import_statement").
.decl fac_definition(st: node, f: node)
fac_definition(st, f) :- fac_top(st, _), defstmt(f, st).
.decl fac_issue(at: node, code: symbol, ordinal: number, witness: node)
fac_issue(f, "FAC001", i, f) :- fac_definition(st, f), fac_top(st, i).
fac_issue(st, "FAC003", i, alias) :- fac_top(st, i), fac_import(st, alias, module), prefix("impl.", module).
fac_issue(st, "FAC003", i, st) :- fac_top(st, i), fac_from(st, module), prefix("impl.", module).
fac_issue(st, "FAC004", i, a) :- fac_top(st, i), fac_assignment(st, a), fac_bad_assignment(a).
fac_issue(st, "FAC002", i, st) :- fac_top(st, i), !doc_stmt(st, _), !fac_import_statement(st),
    !fac_definition(st, _), !fac_assignment(st, _).

.violation Facade(at: node, code: symbol, ordinal: number, witness: node, message: symbol) code {code} "{message}"
Facade(at, code, i, witness, message) :- fac_issue(at, code, i, witness), fac_message(code, message).
// Attempted files include every caller occurrence, including explicit non-Python files.
.violation FileCount(at: span, count: number) code "FAC_FILES" "{count}"
FileCount(@place("facade-policy", 1), n) :- option("entryPoint", "lint_targets"),
    n = count : { option_item("attemptedPaths", i, _) }.
// The original constant assertion reads the same table that selects configured files.
.violation DefaultTarget(at: span, path: symbol, ordinal: number) code "FAC_CONFIG" "{path}"
DefaultTarget(@place("facade-policy", 1), path, i) :- option("entryPoint", "default_targets"), fac_default(i, path).
'''
