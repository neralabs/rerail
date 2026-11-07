"""Translate both backend-coupling import contracts and their shared configuration tables."""
import json
from data_layer_rr_text import module, classes, character_class


def backend_coupling(source, target, header):
    original = module(source, 'backend_coupling')
    text = header('backend_coupling', target)
    text += '.decl bc_config(key: symbol, i: number, value: symbol)\n'
    keys = ['BACKEND_PREFIXES', 'REGISTRATION_FILES', 'SQL_SPECIFIC_DIRS', 'SHARED_DESPITE_NAME', 'NON_SQL_BACKENDS']
    checker = original.BackendCouplingChecker()
    tables = {key: sorted(getattr(original, key)) for key in keys}
    tables.update(name=[checker.name], description=[checker.description], directories=checker.directories)
    for key, values in tables.items():
        for i, value in enumerate(values):
            text += f'bc_config({json.dumps(key)}, {i}, {json.dumps(value)}).\n'
    classes()
    digits = character_class(str.isdecimal)
    body = r'''
.decl bc_checker()
bc_checker() :- option("entryPoint", "BackendCouplingChecker.check_file").
.decl bc_path(d: number, raw: symbol, normalized: symbol)
bc_path(d, raw, @replace_all(raw, "\\\\", "/")) :- doc(d, _, _), bc_checker(), option("path", raw).
bc_path(d, path, path) :- doc(d, path, _), !bc_checker().
.decl bc_common(d: number)
bc_common(d) :- bc_path(d, _, path), prefix("impl/common/", path).
bc_common(d) :- bc_path(d, _, path), bc_checker(), contains("/impl/common/", path).
.decl bc_impl(d: number)
bc_impl(d) :- bc_path(d, _, path), prefix("impl/", path).
bc_impl(d) :- bc_path(d, _, path), bc_checker(), contains("/impl/", path).
.decl bc_layer(path: symbol)
bc_layer(path) :- bc_path(_, _, path), match("(?:[\\s\\S]*/)?l__DIGITS__{4}_[^/\\n][^/]*(?:/[\\s\\S]*)?", path).
.decl bc_public(d: number)
bc_public(d) :- bc_path(d, _, path), !suffix("/__init__.py", path),
    (prefix("api/", path) ; prefix("data_layer/", path) ; prefix("_internal/", path)).
bc_public(d) :- bc_path(d, _, path), !suffix("/__init__.py", path), bc_checker(),
    (contains("/api/", path) ; contains("/data_layer/", path) ; contains("/_internal/", path)).
bc_public(d) :- bc_path(d, _, path), !suffix("/__init__.py", path), !bc_impl(d), bc_layer(path).
.decl bc_nonsql(d: number)
bc_nonsql(d) :- bc_path(d, raw, _), bc_config("NON_SQL_BACKENDS", _, prefix), prefix(prefix, raw).
bc_nonsql(d) :- bc_path(d, raw, _), bc_checker(), bc_config("NON_SQL_BACKENDS", _, prefix), contains(cat("/", prefix), raw).
.decl bc_skip(d: number)
bc_skip(d) :- bc_path(d, raw, _), bc_config("REGISTRATION_FILES", _, suffix), suffix(suffix, raw).
bc_skip(d) :- bc_path(d, "_internal/dynamic_import.py", _).
bc_skip(d) :- bc_path(d, raw, _), contains("impl/common/", raw),
    file_name(d, basename), contains("test_", basename).

// The framework selects only configured production roots, then applies its directory exclusions.
.decl bc_hidden(d: number)
bc_hidden(d) :- path_part(d, part),
    (part = "__pycache__" ; part = ".git" ; part = ".venv" ; part = "venv" ;
     part = "tmp" ; part = "tests" ; part = "test").
.decl bc_framework_file(d: number)
bc_framework_file(d) :- doc(d, path, _), file_name(d, f), suffix(".py", f), !bc_hidden(d),
    (prefix("api/", path) ; prefix("data_layer/", path) ; prefix("_internal/", path) ;
     prefix("impl/common/", path) ; prefix("impl/mongodb/", path) ; prefix("impl/neo4j/", path) ;
     match("l__DIGITS__{4}_[^/\\n][^/]*/[\\s\\S]*", path)).
.decl bc_trigger(d: number)
bc_trigger(d) :- source(d, sp), text(sp, s), (contains("impl.", s) ; contains("impl/", s)).
.decl bc_active(d: number)
bc_active(d) :- doc(d, _, _), bc_checker(), !bc_skip(d), !syntax_error(d, _, _).
bc_active(d) :- bc_framework_file(d), !bc_checker(), !bc_skip(d), bc_trigger(d), !syntax_error(d, _, _).

// CPython joins normalized identifier components, ignoring whitespace and relative-import dots.
.decl bc_component(n: node, i: number, s: symbol)
bc_component(n, i, @nfkc(raw)) :- kind(n, "dotted_name"), child(n, c, i), kind(c, "identifier"), text(c, raw).
.decl bc_dotted(n: node, s: symbol)
bc_dotted(n, s) :- kind(n, "dotted_name"), s = join part by i sep "." : { bc_component(n, i, part) }.
.decl bc_alias(st: node, alias: node, module: symbol)
bc_alias(st, n, module) :- kind(st, "import_statement"), field(st, "name", n), bc_dotted(n, module).
bc_alias(st, n, module) :- kind(st, "import_statement"), field(st, "name", n), kind(n, "aliased_import"),
    field(n, "name", d), bc_dotted(d, module).
.decl bc_import(st: node, module: symbol)
bc_import(st, module) :- bc_alias(st, alias, module), node(_, alias, _, start, _),
    start = min offset : { bc_alias(st, other, _), node(_, other, _, offset, _) }.
bc_import(st, module) :- (kind(st, "import_from_statement") ; kind(st, "future_import_statement")),
    field(st, "module_name", n), bc_dotted(n, module).
bc_import(st, module) :- kind(st, "import_from_statement"), field(st, "module_name", r), kind(r, "relative_import"),
    child(r, n, _), bc_dotted(n, module).
.decl bc_tc(i: node)
bc_tc(i) :- (kind(i, "if_statement") ; kind(i, "elif_clause")), field(i, "condition", test),
    unparen(test, t), kind(t, "identifier"), text(t, raw), @nfkc(raw) = "TYPE_CHECKING".
.decl bc_tc_end(i: node, last: number)
bc_tc_end(i, last) :- bc_tc(i), kind(i, "if_statement"), end_line(i, last).
// An ast.If represented by an elif owns the remaining alternatives through the enclosing if's end.
bc_tc_end(i, last) :- bc_tc(i), kind(i, "elif_clause"), child(parent, i, _), kind(parent, "if_statement"), end_line(parent, last).
.decl bc_exempt(st: node)
// The legacy checker compares inclusive source-line ranges, including every elif and else.
bc_exempt(st) :- bc_checker(), bc_import(st, _), docof(st, d), line(st, l),
    bc_tc(i), docof(i, d), line(i, first), bc_tc_end(i, last), first <= l, l <= last.
// The collector uses actual ancestors. An elif is a nested ast.If inside its predecessor's orelse.
bc_exempt(st) :- !bc_checker(), bc_import(st, _), bc_tc(i), within(st, i).
bc_exempt(st) :- !bc_checker(), bc_import(st, _), bc_tc(i), kind(i, "elif_clause"),
    child(parent, i, _), kind(parent, "if_statement"), within(st, parent),
    node(_, st, _, start, _), node(_, i, _, begin, _), begin <= start.
.decl bc_direct(st: node, module: symbol, context: symbol)
bc_direct(st, module, "public code") :- bc_import(st, module), docof(st, d), bc_active(d), bc_public(d),
    !bc_exempt(st), bc_config("BACKEND_PREFIXES", _, prefix), prefix(prefix, module).
bc_direct(st, module, "impl/common") :- bc_import(st, module), docof(st, d), bc_active(d), !bc_public(d), bc_common(d),
    !bc_exempt(st), bc_config("BACKEND_PREFIXES", _, prefix), prefix(prefix, module).
.decl bc_issue(st: node, code: symbol, message: symbol)
bc_issue(st, "direct_backend_import", cat("Direct backend import '", module, "' in ", context, "/ — use a registry instead")) :-
    bc_direct(st, module, context).
bc_issue(st, "sql_in_nonsql", cat("Non-SQL backend imports SQL-specific module '", module, "'")) :-
    bc_import(st, module), docof(st, d), bc_active(d), bc_nonsql(d), !bc_exempt(st),
    bc_config("SQL_SPECIFIC_DIRS", _, dir),
    (contains(cat("impl.common.", dir), module) ; contains(cat("impl/common/", dir), module)),
    (bc_checker() ; !bc_direct(st, _, _)).
.violation Coupling(at: node, code: symbol, message: symbol, ordinal: number, depth: number) code {code} "{message}"
Coupling(st, code, message, i, depth) :- bc_issue(st, code, message), visit_index(st, i), ast_depth(st, depth).

// Configuration and protocol assertions read the same tables used above.
.violation Configuration(at: span, key: symbol, ordinal: number, value: symbol) code "BC_CONFIG" "{value}"
Configuration(@place("backend-policy", 1), key, i, value) :- option("entryPoint", "configuration"), bc_config(key, i, value).
.violation ShouldCheck(at: span) code "BC_CHECK" "true"
ShouldCheck(@place("backend-policy", 1)) :- option("entryPoint", "should_check_file"), option("path", path), !contains("tests", path).
.violation FrameworkFile(at: span) code "BC_FILE" "selected"
FrameworkFile(@place(path, 1)) :- option("entryPoint", "collector"), bc_framework_file(d), doc(d, path, _).
'''
    return text + body.replace('__DIGITS__', json.dumps(digits)[1:-1])
