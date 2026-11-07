"""Compile the shared ServerPlan surface collector into native relations."""
import importlib
import json
from data_layer_rr_text import module


def serverplan_grammar_surface(source, target, header):
    original = module(source, 'serverplan_grammar_surface')
    shared = importlib.import_module('scripts.lint_lib.serverplan_facts')
    text = '\n'.join(line for line in header('serverplan_grammar_surface', target).splitlines()
                     if not line.startswith('.scope exclude')).replace('"python/ast.rr"', '"python/ast-shape.rr"') + '\n'
    text += '.setting severity "Surface error"\n'
    text += '.decl sgp_name(group: symbol, name: symbol)\n'
    for group, values in [('helper', shared.FORBIDDEN_GRAMMAR_NAMES),
                          ('direct', shared.FORBIDDEN_DIRECT_GRAMMAR_NAMES),
                          ('ddl', shared.FORBIDDEN_DDL_ESCAPE_NAMES),
                          ('skip', shared.SERVERPLAN_FACT_SKIP_DIRS),
                          ('trigger', original.SERVERPLAN_SURFACE_EXTRACTOR.source_triggers)]:
        for value in sorted(values):
            text += f'sgp_name({json.dumps(group)}, {json.dumps(value)}).\n'
    return text + r'''
.decl sgp_hidden(d: number)
sgp_hidden(d) :- path_part(d, p),
    (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp").
.decl sgp_trigger(d: number)
sgp_trigger(d) :- source(d, sp), text(sp, source), sgp_name("trigger", t), contains(t, source).
.decl sgp_admitted(d: number)
sgp_admitted(d) :- doc(d, _, _), option("entryPoint", "scan_file").
sgp_admitted(d) :- doc(d, _, _), option("entryPoint", "collector"), sgp_trigger(d).
sgp_admitted(d) :- doc(d, _, _), !option("entryPoint", _), file_name(d, f), suffix(".py", f),
    !sgp_hidden(d), sgp_trigger(d).
.decl sgp_active(d: number)
sgp_active(d) :- sgp_admitted(d), !syntax_error(d, _, _).
.violation Materialized(at: span) code "SPG_FILE" "Surface facts materialized"
Materialized(@place(path, 1)) :- doc(d, path, _), sgp_active(d), option("entryPoint", "collector").
.decl sgp_hit(n: node, kind: symbol, name: symbol, witness: number)
sgp_hit(n, "helper_definition", @nfkc(raw), 0) :- kind(n, "function_definition"),
    def_name(n, raw), sgp_name("helper", @nfkc(raw)).
sgp_hit(n, "direct_helper_definition", @nfkc(raw), 0) :- kind(n, "function_definition"),
    def_name(n, raw), sgp_name("direct", @nfkc(raw)).
sgp_hit(n, "helper_attribute", @nfkc(raw), layer) :-
    py_ast_attribute(n, raw, layer), sgp_name("helper", @nfkc(raw)).
sgp_hit(n, "direct_helper_attribute", @nfkc(raw), layer) :-
    py_ast_attribute(n, raw, layer), sgp_name("direct", @nfkc(raw)).
sgp_hit(n, "ddl_escape", @nfkc(raw), 0) :- ast_name(n, raw), sgp_name("ddl", @nfkc(raw)).
.decl sgp_alias(a: node, raw: symbol)
sgp_alias(a, raw) :- kind(a, "dotted_name"), text(a, raw).
sgp_alias(a, raw) :- kind(a, "aliased_import"), field(a, "name", name), text(name, raw).
sgp_hit(n, "ddl_import", @nfkc(raw), i) :- (kind(n, "import_from_statement") ; kind(n, "future_import_statement")),
    field(n, "name", a), child(n, a, i), sgp_alias(a, raw), sgp_name("ddl", @nfkc(raw)).
.decl sgp_prefix(kind: symbol, value: symbol)
sgp_prefix("helper_definition", "forbidden helper definition ").
sgp_prefix("direct_helper_definition", "forbidden direct grammar helper definition ").
sgp_prefix("helper_attribute", "forbidden helper attribute .").
sgp_prefix("direct_helper_attribute", "forbidden direct grammar helper attribute .").
sgp_prefix("ddl_escape", "forbidden DDL escape hatch ").
sgp_prefix("ddl_import", "forbidden DDL escape import ").
.decl sgp_depth(n: node, kind: symbol, witness: number, depth: number)
sgp_depth(n, k, w, base + w) :- sgp_hit(n, k, _, w), py_ast_depth(n, base),
    (k = "helper_attribute" ; k = "direct_helper_attribute").
sgp_depth(n, k, w, base) :- sgp_hit(n, k, _, w), py_ast_depth(n, base),
    k != "helper_attribute", k != "direct_helper_attribute".
.violation Surface(at: node, code: symbol, message: symbol, kind: symbol, name: symbol,
    depth: number, ordinal: number, witness: number) code {code} "{message}"
Surface(n, cat("SPG-", k), cat(p, name), k, name, depth, ordinal, witness) :-
    sgp_hit(n, k, name, witness), sgp_prefix(k, p), docof(n, d), sgp_active(d),
    sgp_depth(n, k, witness, depth), visit_index(n, ordinal).
.violation ParseError(at: span, detail: symbol, filename: symbol, errorLine: number) code "SPG-SYNTAX"
    "syntax error while scanning: {detail} ({filename}, line {errorLine})"
ParseError(@place(path, 1), detail, filename, l) :- doc(d, path, _), option("entryPoint", "scan_file"),
    option("filename", filename), syntax_error(d, l, detail), detail != "source code string cannot contain null bytes".
.violation NullSource(at: span) code "SPG-SYNTAX" "syntax error while scanning: source code string cannot contain null bytes"
NullSource(@place(path, 1)) :- doc(d, path, _), option("entryPoint", "scan_file"),
    syntax_error(d, _, "source code string cannot contain null bytes").
.decl sgp_candidate(i: number, path: symbol)
sgp_candidate(i, path) :- option_item("candidates", i, path), suffix(".py", path).
.decl sgp_candidate_hidden(i: number)
sgp_candidate_hidden(i) :- sgp_candidate(i, path), sgp_name("skip", name),
    contains(cat("/", name, "/"), cat("/", path, "/")).
.violation Discovery(at: span, ordinal: number, result: symbol) code "SPG_DISCOVERY" "{result}"
Discovery(@place("inventory", 1), i, path) :- sgp_candidate(i, path), !sgp_candidate_hidden(i),
    option("entryPoint", "iter_python_files").
.decl sgp_framework_hidden(i: number)
sgp_framework_hidden(i) :- sgp_candidate(i, path),
    (contains("/.git/", cat("/", path, "/")) ; contains("/.venv/", cat("/", path, "/")) ;
     contains("/venv/", cat("/", path, "/")) ; contains("/__pycache__/", cat("/", path, "/")) ;
     contains("/tmp/", cat("/", path, "/"))), option("broad", "true").
.violation FileSelection(at: span, ordinal: number, result: symbol) code "SPG_PATH" "{result}"
FileSelection(@place("inventory", 1), i, path) :- sgp_candidate(i, path),
    option_item("regularFiles", _, path), !sgp_framework_hidden(i), option("entryPoint", "collector_paths").
'''
