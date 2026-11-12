"""Port FieldInfo inspection, function-local resolution discovery, and predicate contracts."""
import json
from dataclasses import asdict
from pathlib import Path
from data_layer_rr_text import module, character_class
from data_layer_rr_unresolved_graph import supplied_ast


def unresolved_forward_refs(source, target, header):
    original = module(source, 'unresolved_forward_refs')
    text = header('unresolved_forward_refs', target)
    from data_layer_rr_cli import api_scope
    text = api_scope(text)
    text += '.decl uf_config(key: symbol, value: symbol)\n'
    for key in ['TYPE_INSPECTION_FUNCTIONS', 'RESOLVED_PATTERNS']:
        for value in sorted(getattr(original, key)):
            text += f'uf_config({json.dumps(key)}, {json.dumps(value)}).\n'
    plugin = original.unresolved_forward_refs_plugin(Path('.'))
    metadata = dict(extractor=asdict(original.UNRESOLVED_FORWARD_REFS_EXTRACTOR),
                    plugin={key:getattr(plugin,key) for key in ['name','enforcement','description']},
                    scope=asdict(plugin.file_scope))
    metadata['scope']['directories'] = [str(p) for p in plugin.file_scope.directories]
    metadata['scope']['explicit_files'] = [str(p) for p in plugin.file_scope.explicit_files]
    for key,value in metadata.items():
        text += f'uf_config({json.dumps(key)}, {json.dumps(json.dumps(value))}).\n'
    uppercase = '(?s)' + character_class(str.isupper) + '.*'
    text += f'.decl uf_upper(name: symbol)\nuf_upper(name) :- uf_name(_, name), match({json.dumps(uppercase)}, name).\n'
    space = character_class(str.isspace)
    text += '.decl uf_trim(pattern: symbol)\nuf_trim(' + json.dumps('^' + space + '+|' + space + '+$') + ').\n'
    return text + r'''
.decl uf_name(e: node, name: symbol)
uf_name(e, @nfkc(raw)) :- unparen(e, n), ast_name(n, raw).
.decl uf_attr(e: node, object: node, name: symbol)
uf_attr(e, obj, @nfkc(raw)) :- unparen(e, n), kind(n, "attribute"), field(n, "object", o),
    unparen(o, obj), field(n, "attribute", a), text(a, raw).
.decl uf_leaf(e: node, name: symbol)
uf_leaf(e, name) :- uf_name(e, name).
uf_leaf(e, name) :- uf_attr(e, _, name).
.decl uf_call(c: node, name: symbol)
.decl uf_call_function(c: node, f: node)
uf_call_function(c, f) :- kind(c, "call"), field(c, "function", f).
uf_call(c, name) :- uf_call_function(c, f), uf_leaf(f, name).
.decl uf_simple_call(c: node, name: symbol)
uf_simple_call(c, name) :- uf_call_function(c, f), uf_name(f, name).
.decl uf_arg(c: node, ordinal: number, value: node)
uf_arg(c, i, n) :- kind(c, "call"), field(c, "arguments", a), child(a, raw, j),
    !kind(raw, "keyword_argument"), !kind(raw, "dictionary_splat"), !kind(raw, "comment"),
    !kind(raw, "line_continuation"), unparen(raw, n),
    i = count : { child(a, prev, k), k < j, !kind(prev, "keyword_argument"),
        !kind(prev, "dictionary_splat"), !kind(prev, "comment"), !kind(prev, "line_continuation") }.
.decl uf_field(n: node)
.decl uf_attribute_node(n: node)
uf_attribute_node(n) :- kind(n, "attribute").
.decl uf_subscript_value(n: node, value: node)
uf_subscript_value(n, owner) :- kind(n, "subscript"), field(n, "value", owner).
uf_field(n) :- uf_attribute_node(n), uf_attr(n, obj, "annotation"), uf_name(obj, name),
    (contains("field_info", @lower(name)) ; suffix("_info", @lower(name)) ; name = "f").
uf_field(n) :- uf_attribute_node(n), uf_attr(n, obj, "annotation"),
    uf_subscript_value(obj, owner), uf_attr(owner, _, "model_fields").
.decl uf_none(n: node)
uf_none(n) :- unparen(n, v), kind(v, "none").
.decl uf_operand(c: node, ordinal: number, n: node)
uf_operand(c, i, n) :- kind(c, "comparison_operator"), child(c, raw, j), unparen(raw, n),
    !kind(raw, "comment"), !kind(raw, "line_continuation"),
    i = count : { child(c, prev, k), k < j, !kind(prev, "comment"), !kind(prev, "line_continuation") }.
.decl uf_safe_str(p: node)
uf_safe_str(p) :- uf_simple_call(p, "str").
.decl uf_safe_none(p: node)
uf_safe_none(p) :- uf_operand(p, _, n), uf_none(n).
.decl uf_detection_type(n: node)
uf_detection_type(n) :- uf_name(n, name), (name = "str" ; name = "ForwardRef" ; name = "TypeVar").
.decl uf_tuple_item(t: node, e: node)
uf_tuple_item(t, e) :- kind(t, "tuple"), child(t, e, _).
uf_detection_type(n) :- uf_tuple_item(n, e), uf_name(e, name),
    (name = "str" ; name = "ForwardRef" ; name = "TypeVar").
.decl uf_safe_detection(p: node)
uf_safe_detection(p) :- uf_simple_call(p, "isinstance"),
    uf_arg(p, 1, t), uf_detection_type(t).
.decl uf_scope(n: node, f: node)
uf_scope(n, f) :- def_frame(n, f).
uf_scope(n, m) :- kind(n, _), !def_frame(n, _), docof(n, d), modnode(d, m).
.decl uf_function_member(f: node, n: node)
uf_function_member(f, n) :- kind(f, "function_definition"), within(n, f).
uf_function_member(f, n) :- kind(f, "function_definition"), decorator(f, e), (n = e ; within(n, e)).
.decl uf_resolved(f: node, name: symbol)
.decl uf_assign_value(a: node, v: node)
uf_assign_value(a, v) :- kind(a, "assignment"), !field(a, "type", _), assign_value(a, raw), unparen(raw, v).
.decl uf_assign_target(a: node, t: node)
uf_assign_target(a, t) :- assign_target(a, t).
uf_resolved(f, name) :- uf_function_member(f, a), uf_assign_value(a, c),
    uf_call(c, "get_type_hints"), uf_assign_target(a, t), uf_name(t, name).
.decl uf_contains(p: node, n: node)
uf_contains(p, n) :- within(n, p).
uf_contains(n, n) :- kind(n, _).
.decl uf_fallback(n: node)
uf_fallback(n) :- uf_field(n), uf_scope(n, f), uf_call_function(c, fn),
    uf_attr(fn, obj, "get"), uf_name(obj, name),
    (uf_config("RESOLVED_PATTERNS", name) ; uf_resolved(f, name)), uf_arg(c, 1, fallback),
    uf_contains(fallback, n).
.decl uf_op(c: node, i: number, op: symbol)
uf_op(c, i, op) :- op_at(c, i, op).
.decl uf_is_problem(c: node, ordinal: number, problem: symbol)
uf_is_problem(c, i, cat("compared with 'is' to ", name)) :- uf_op(c, i, op),
    (op = "is" ; op = "is not"), uf_operand(c, i + 1, rhs), !uf_none(rhs), uf_name(rhs, name).
uf_is_problem(c, i, "compared with 'is' to a type") :- uf_op(c, i, op),
    (op = "is" ; op = "is not"), uf_operand(c, i + 1, rhs), !uf_none(rhs), !uf_name(rhs, _).
.decl uf_eq_problem(c: node, ordinal: number, problem: symbol)
uf_eq_problem(c, i, cat("compared with '==' to ", name)) :- uf_op(c, i, op),
    (op = "==" ; op = "!="), uf_operand(c, i + 1, rhs), uf_name(rhs, name), uf_upper(name).
.decl uf_problem(p: node, problem: symbol)
uf_problem(p, cat(cat("passed to ", name), "() without resolving ForwardRef")) :-
    uf_call(p, name), uf_config("TYPE_INSPECTION_FUNCTIONS", name).
uf_problem(p, problem) :- uf_is_problem(p, i, problem), i = min j : { uf_is_problem(p, j, _) }.
uf_problem(p, problem) :- !uf_is_problem(p, _, _), uf_eq_problem(p, i, problem),
    i = min j : { uf_eq_problem(p, j, _) }.
.decl uf_active(d: number)
uf_active(d) :- doc(d, _, _), option("entryPoint", "analyze_file"), option("suffix", ".py"),
    option("isFile", "true"), !option("relativePath", "true").
uf_active(d) :- doc(d, buffer, _), option("entryPoint", "cli_scan"),
    option_item("logicalFiles", i, buffer), option_item("fileSuffixes", i, ".py"), !option_item("relativeFiles", _, buffer).
uf_active(d) :- doc(d, _, _), !option("entryPoint", _), file_name(d, name), suffix(".py", name),
    doc(d, path, _), (prefix("api/", path) ; prefix("impl/", path)), !uf_hidden(d).
.decl uf_hidden(d: number)
uf_hidden(d) :- path_part(d, p),
    (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp" ; p = "tests" ; p = "test").
.decl uf_context(n: node, name: symbol)
.decl uf_function_name(f: node, name: symbol)
uf_function_name(f, @nfkc(raw)) :- def_name(f, raw).
.decl uf_module(m: node)
uf_module(m) :- kind(m, "module").
uf_context(n, name) :- uf_scope(n, f), uf_function_name(f, name).
uf_context(n, "<module>") :- uf_scope(n, m), uf_module(m).
.decl uf_parent(n: node, p: node)
uf_parent(n, p) :- ast_parent(n, p).
.decl uf_bad(n: node, function: symbol, problem: symbol)
uf_bad(n, function, problem) :- uf_field(n), uf_parent(n, parent),
    !uf_safe_str(parent), !uf_safe_none(parent), !uf_safe_detection(parent), !uf_fallback(n),
    uf_problem(parent, problem), uf_context(n, function).
.violation Unresolved(at: node, function: symbol, problem: symbol, source: symbol, ordinal: number)
    code "unresolved_forward_ref" "{function}(): {problem}"
Unresolved(n, function, problem, snippet, ordinal) :- uf_bad(n, function, problem), visit_index(n, ordinal),
    docof(n, d), uf_active(d), !syntax_error(d, _, _), source(d, sp), text(sp, full), contains(".annotation", full),
    line(n, l), split_line(d, l, raw), uf_trim(trim), snippet = @replace_all(raw, trim, "").
.violation ReadFailure(at: span) code "UF_READ_ERROR" "Source file could not be read"
ReadFailure(@place("input.py", 1)) :- option("entryPoint", entry),
    (entry = "analyze_file" ; entry = "AnnotationUsageFinder.analyze"), option("readError", "true").
.violation Configuration(at: span, key: symbol, value: symbol) code "UF_CONFIG" "{value}"
Configuration(@place("unresolved-policy", 1), key, value) :- uf_config(key, value), option("entryPoint", "configuration").
.decl uf_helper_root(n: node)
uf_helper_root(n) :- kind(a, "assignment"), field(a, "left", t), text(t, "_rr_input"),
    field(a, "right", v), unparen(v, n).
.decl uf_helper_true()
uf_helper_true() :- option("entryPoint", "AnnotationUsageFinder._is_field_annotation_access"), uf_helper_root(n), uf_field(n).
uf_helper_true() :- option("entryPoint", "AnnotationUsageFinder._is_safe_str_conversion"), uf_helper_root(p), uf_safe_str(p).
uf_helper_true() :- option("entryPoint", "AnnotationUsageFinder._is_none_check"), uf_helper_root(p), uf_safe_none(p).
uf_helper_true() :- option("entryPoint", "AnnotationUsageFinder._is_isinstance_none_check"), uf_helper_root(p), uf_safe_detection(p).
// The two source placeholder predicates are constantly false; they deliberately have no true clause.
.decl uf_helper_edge(p: symbol, n: symbol)
uf_helper_edge(p, n) :- option_item("graphParents", i, p), option_item("graphChildren", i, n).
.decl uf_helper_reach(p: symbol, n: symbol)
uf_helper_reach(p, p) :- option("containerId", p).
uf_helper_reach(p, n) :- uf_helper_reach(p, q), uf_helper_edge(q, n).
uf_helper_true() :- option("entryPoint", "AnnotationUsageFinder._node_contains"),
    option("containerId", container), option("targetId", target), uf_helper_reach(container, target).
.violation HelperTrue(at: span) code "UF_HELPER_TRUE" "true"
HelperTrue(@place("unresolved-helper", 1)) :- uf_helper_true().
.decl uf_cli_hidden(i: number)
uf_cli_hidden(i) :- option_item("cliNames", i, name), contains("test_", name).
uf_cli_hidden(i) :- option_item("cliRelativePaths", i, path),
    (contains("/tests/", cat("/", path, "/")) ; contains("/__pycache__/", cat("/", path, "/"))).
.violation CliFile(at: span, ordinal: number) code "UF_CLI_FILE" "File selected"
CliFile(@place("cli", 1), i) :- option("entryPoint", "cli_selection"), option_item("cliModes", i, "direct").
CliFile(@place("cli", 1), i) :- option("entryPoint", "cli_selection"), option_item("cliModes", i, "recursive"),
    option_item("cliNames", i, name), suffix(".py", name), !uf_cli_hidden(i).
''' + supplied_ast() + '\n#include "data-layer/unresolved-api.rr"\n'
