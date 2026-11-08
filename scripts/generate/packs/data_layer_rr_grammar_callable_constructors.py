"""Translate callable grammar constructors onto shared native syntax relations."""
import json,re


def grammar_callable_constructors(source,target,header):
    text=header('grammar_callable_constructors',target)+'#include "project.rr"\n'
    text=re.sub(r'^\.scope include .*$', '.scope include "impl/common/l2100_sql_grammar/**/*.py", "scripts/lint_grammar_callable_constructors.py"',text,flags=re.M)
    return text+configuration()+detector()+outputs()


def configuration():
    return r'''
.violation GCCFormatted(at: span, key: symbol, formatted: symbol) code "GCC_FORMAT" "{formatted}"
GCCFormatted(@place("", 0), cat(path, ":", line, ":", code, ":", detail), cat(path, ":", line, ": ", code, ": ", detail)) :-
 option("entryPoint", "format"), option("formatPath", path), option("formatLine", line), option("formatCode", code), option("formatDetail", detail).
'''


def detector():
    return r'''
.decl gcc_result(d: number, line: number, code: symbol, detail: symbol, origin: node, tag: symbol)
.decl gcc_path(d: number, path: symbol)
gcc_path(d, original) :- doc(d, logical, _), doc_role(d, "primary"), option_item("documentPaths", i, logical), option_item("originalPaths", i, original).
gcc_path(d, path) :- doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
.decl gcc_root_prefix(root: symbol)
gcc_root_prefix(root) :- workspace_root(root), suffix("/", root).
gcc_root_prefix(cat(root, "/")) :- workspace_root(root), !suffix("/", root).
.decl gcc_full_path(d: number, path: symbol)
gcc_full_path(d, path) :- gcc_path(d, path), option_item("originalPaths", _, path).
gcc_full_path(d, cat(root, path)) :- gcc_path(d, path), !option_item("originalPaths", _, path), gcc_root_prefix(root).
.decl gcc_excluded(d: number)
gcc_excluded(d) :- gcc_full_path(d, path), match_all("(?:\\A|/)tests(?:/|\\z)", path, _, _, _).
gcc_excluded(d) :- gcc_full_path(d, path), (path = "fragments.py" ; suffix("/fragments.py", path)).
.decl gcc_marker()
gcc_marker() :- doc(d, "scripts/lint_grammar_callable_constructors.py", _), doc_role(d, "primary"), !syntax_error(d, _, _).
.decl gcc_file(d: number)
gcc_file(d) :- option("entryPoint", "collect_violations"), gcc_path(d, path), suffix(".py", path), !gcc_excluded(d), !syntax_error(d, _, _).
gcc_file(d) :- !option("entryPoint", _), gcc_marker(), gcc_path(d, path), prefix("impl/common/l2100_sql_grammar/", path), suffix(".py", path), !gcc_excluded(d), !syntax_error(d, _, _).
.decl gcc_ancestor(n: node, a: node)
gcc_ancestor(n, a) :- ast_parent(n, a).
gcc_ancestor(n, a) :- ast_parent(n, p), gcc_ancestor(p, a).
.decl gcc_name(n: node, name: symbol)
gcc_name(n, @nfkc(name)) :- kind(n, "identifier"), text(n, name).
gcc_name(n, @nfkc(name)) :- kind(n, "attribute"), field(n, "attribute", a), text(a, name).
gcc_name(n, name) :- kind(n, "subscript"), field(n, "value", raw), unparen(raw, v), gcc_name(v, name).
gcc_name(n, name) :- kind(n, "call"), field(n, "function", raw), unparen(raw, f), gcc_name(f, name).
gcc_name(n, name) :- kind(n, k), (k = "type" ; k = "generic_type"), child(n, raw, 0), unparen(raw, first), gcc_name(first, name).
.decl gcc_sync(f: node, name: symbol)
gcc_sync(f, @nfkc(name)) :- kind(f, "function_definition"), !is_async(f), def_name(f, name).
.decl gcc_module(d: number, m: node)
gcc_module(d, m) :- gcc_file(d), kind(m, "module"), docof(m, d), doc_role(d, "primary").
.decl gcc_class(c: node, d: number, name: symbol)
gcc_class(c, d, @nfkc(name)) :- gcc_file(d), kind(c, "class_definition"), docof(c, d), def_name(c, name).
.decl gcc_method(c: node, f: node, name: symbol, index: number)
gcc_method(c, f, name, i) :- gcc_class(c, _, _), field(c, "body", body), body_stmt(body, stmt, i), defstmt(f, stmt), gcc_sync(f, name).
.decl gcc_first_method(c: node, name: symbol, f: node)
gcc_first_method(c, name, f) :- gcc_method(c, f, name, i), i = min j : { gcc_method(c, _, name, j) }.
.decl gcc_renderable(c: node)
gcc_renderable(c) :- gcc_method(c, _, "render", _).
.decl gcc_registry(name: symbol)
gcc_registry(name) :- gcc_module(d, m), body_stmt(m, stmt, _), defstmt(c, stmt), gcc_class(c, d, name), gcc_renderable(c).
.decl gcc_module_function(d: number, name: symbol)
gcc_module_function(d, name) :- gcc_module(d, m), body_stmt(m, stmt, _), defstmt(f, stmt), gcc_sync(f, name).
.decl gcc_nested_function(f: node, name: symbol)
gcc_nested_function(f, name) :- gcc_sync(f, _), field(f, "body", body), body_stmt(body, stmt, _), defstmt(child, stmt), gcc_sync(child, name).
.decl gcc_target_name(n: node, name: symbol)
gcc_target_name(n, @nfkc(name)) :- kind(n, "identifier"), text(n, name).
gcc_target_name(n, name) :- kind(n, k), (k = "tuple" ; k = "list" ; k = "tuple_pattern" ; k = "list_pattern" ; k = "pattern_list" ; k = "expression_list" ; k = "as_pattern_target" ; k = "parenthesized_expression"), child(n, c, _), gcc_target_name(c, name).
.decl gcc_binding_target(n: node, target: node)
gcc_binding_target(n, target) :- kind(n, k), (k = "assignment" ; k = "augmented_assignment" ; k = "for_statement"), field(n, "left", target).
gcc_binding_target(w, target) :- kind(w, "with_statement"), !op(w, "async"), child(w, clause, _), kind(clause, "with_clause"), within(item, clause), kind(item, "with_item"), field(item, "value", pattern), kind(pattern, "as_pattern"), field(pattern, "alias", target).
.decl gcc_raw_binding(f: node, name: symbol)
gcc_raw_binding(f, @nfkc(name)) :- gcc_sync(f, _), sig_param(f, _, nm, _, _, _, _), text(nm, name).
gcc_raw_binding(f, name) :- gcc_sync(f, _), gcc_ancestor(n, f), gcc_binding_target(n, target), gcc_target_name(target, name).
.decl gcc_binding(f: node, name: symbol)
gcc_binding(f, name) :- gcc_raw_binding(f, name), !gcc_nested_function(f, name).
.decl gcc_visible(call: node, name: symbol, rank: number, answer: number)
gcc_visible(call, name, i, 1) :- kind(call, "call"), gcc_ancestor(call, f), gcc_nested_function(f, name), visit_index(f, i).
gcc_visible(call, name, i, 0) :- kind(call, "call"), gcc_ancestor(call, f), gcc_binding(f, name), visit_index(f, i).
gcc_visible(call, name, -1, 1) :- kind(call, "call"), docof(call, d), gcc_module_function(d, name).
.decl gcc_function_symbol(call: node, name: symbol)
gcc_function_symbol(call, name) :- gcc_visible(call, name, i, 1), i = max k : { gcc_visible(call, name, k, _) }.
.decl gcc_annotation_root(a: node)
gcc_annotation_root(a) :- gcc_parameter(_, _, _, a, _), !unset(a).
.decl gcc_annotation_callable(a: node)
gcc_annotation_callable(a) :- gcc_annotation_root(a), ast_name(a, name), @nfkc(name) = "Callable".
gcc_annotation_callable(a) :- gcc_annotation_root(a), kind(a, "attribute"), field(a, "attribute", attr), text(attr, name), @nfkc(name) = "Callable".
gcc_annotation_callable(a) :- gcc_annotation_root(a), within(n, a), ast_name(n, name), @nfkc(name) = "Callable".
gcc_annotation_callable(a) :- gcc_annotation_root(a), within(n, a), kind(n, "attribute"), field(n, "attribute", attr), text(attr, name), @nfkc(name) = "Callable".
.decl gcc_parameter(c: node, nm: node, name: symbol, annotation: loc, index: number)
gcc_parameter(c, nm, @nfkc(name), a, i) :- gcc_renderable(c), gcc_first_method(c, "__init__", f), sig_param(f, _, nm, i, k, a, _), (k = "pos" ; k = "kwonly"), text(nm, name), @nfkc(name) != "self".
.decl gcc_callable_param(c: node, name: symbol)
gcc_callable_param(c, name) :- gcc_parameter(c, _, name, a, _), !unset(a), gcc_annotation_callable(a).
.decl gcc_current_annotation(c: node, name: symbol, annotation: loc)
gcc_current_annotation(c, name, a) :- gcc_parameter(c, _, name, a, i), i = max j : { gcc_parameter(c, _, name, _, j) }.
.decl gcc_self_field(n: node, name: symbol)
gcc_self_field(n, @nfkc(name)) :- kind(n, "attribute"), field(n, "attribute", a), text(a, name), field(n, "object", raw), unparen(raw, obj), kind(obj, "identifier"), text(obj, receiver), @nfkc(receiver) = "self".
.decl gcc_field_candidate(c: node, field: symbol, parameter: symbol, depth: number, ordinal: number)
gcc_field_candidate(c, field, @nfkc(name), depth, ordinal) :- gcc_renderable(c), gcc_first_method(c, "__init__", f), asgstmt(a, _), !field(a, "type", _), gcc_ancestor(a, f), assign_value(a, raw), unparen(raw, value), kind(value, "identifier"), text(value, name), assign_target(a, target_raw), unparen(target_raw, target), gcc_self_field(target, field), ast_depth(a, depth), visit_index(a, ordinal).
.decl gcc_field_last_depth(c: node, field: symbol, depth: number)
gcc_field_last_depth(c, field, depth) :- gcc_field_candidate(c, field, _, _, _), depth = max d : { gcc_field_candidate(c, field, _, d, _) }.
.decl gcc_field(c: node, field: symbol, parameter: symbol)
gcc_field(c, field, parameter) :- gcc_field_last_depth(c, field, depth), i = max j : { gcc_field_candidate(c, field, _, depth, j) }, gcc_field_candidate(c, field, parameter, depth, i).
.decl gcc_called_field(c: node, field: symbol)
gcc_called_field(c, field) :- gcc_first_method(c, "render", f), kind(call, "call"), gcc_ancestor(call, f), field(call, "function", raw), unparen(raw, value), gcc_self_field(value, field).
.decl gcc_untyped(c: node, parameter: symbol)
gcc_untyped(c, parameter) :- gcc_field(c, _, parameter), !gcc_current_annotation(c, parameter, _).
gcc_untyped(c, parameter) :- gcc_current_annotation(c, parameter, a), unset(a).
.decl gcc_constructor_arg(call: node, arg: node)
gcc_constructor_arg(call, arg) :- kind(call, "call"), field(call, "arguments", al), child(al, raw, _), !kind(raw, "keyword_argument"), !kind(raw, "dictionary_splat"), !kind(raw, "list_splat"), !kind(raw, "comment"), unparen(raw, arg).
gcc_constructor_arg(call, arg) :- kind(call, "call"), field(call, "arguments", al), child(al, kw, _), kind(kw, "keyword_argument"), field(kw, "value", raw), unparen(raw, arg).
gcc_constructor_arg(call, arg) :- kind(call, "call"), field(call, "arguments", al), child(al, kw, _), kind(kw, "dictionary_splat"), child(kw, raw, 0), unparen(raw, arg).
gcc_result(d, line, "CALLABLE_INIT_PARAM", cat(cls, ".__init__ parameter ", @python_repr(name), " is callable"), nm, "") :- gcc_class(c, d, cls), gcc_parameter(c, nm, name, a, _), !unset(a), gcc_annotation_callable(a), line(nm, line).
gcc_result(d, line, "CALLABLE_FIELD", cat(cls, " stores callable constructor parameter ", @python_repr(parameter)), c, field) :- gcc_class(c, d, cls), gcc_field(c, field, parameter), gcc_callable_param(c, parameter), gcc_first_method(c, "__init__", f), line(f, line).
gcc_result(d, line, "CONSTRUCTOR_FIELD_CALLED_IN_RENDER", cat(cls, ".render() calls constructor-derived field ", @python_repr(field)), c, field) :- gcc_class(c, d, cls), gcc_field(c, field, _), gcc_called_field(c, field), gcc_first_method(c, "render", f), line(f, line).
gcc_result(d, line, "UNTYPED_CALLED_CONSTRUCTOR_FIELD", cat(cls, " constructor parameter ", @python_repr(parameter), " is untyped and called in render()"), c, field) :- gcc_class(c, d, cls), gcc_field(c, field, parameter), gcc_called_field(c, field), gcc_untyped(c, parameter), gcc_first_method(c, "__init__", f), line(f, line).
gcc_result(d, line, "ANY_CALLED_CONSTRUCTOR_FIELD", cat(cls, " constructor parameter ", @python_repr(parameter), " is Any and called in render()"), c, field) :- gcc_class(c, d, cls), gcc_field(c, field, parameter), gcc_called_field(c, field), gcc_current_annotation(c, parameter, raw), !unset(raw), unparen(raw, a), gcc_name(a, "Any"), gcc_first_method(c, "__init__", f), line(f, line).
gcc_result(d, line, "CALLABLE_CONSTRUCTOR_ARG", "lambda passed to grammar constructor", arg, "") :- gcc_file(d), kind(call, "call"), docof(call, d), field(call, "function", raw), unparen(raw, f), gcc_name(f, name), gcc_registry(name), gcc_constructor_arg(call, arg), kind(arg, "lambda"), line(call, line).
gcc_result(d, line, "CALLABLE_CONSTRUCTOR_ARG", cat("function object ", @python_repr(name), " passed to grammar constructor"), arg, "") :- gcc_file(d), kind(call, "call"), docof(call, d), field(call, "function", raw), unparen(raw, f), gcc_name(f, cls), gcc_registry(cls), gcc_constructor_arg(call, arg), kind(arg, "identifier"), text(arg, raw_name), name = @nfkc(raw_name), gcc_function_symbol(call, name), line(call, line).
gcc_result(d, line, "SIMPLE_NAMESPACE_RENDER_HOOK", "SimpleNamespace(render=...) hides grammar structure behind a callable hook", kw, "") :- gcc_file(d), kind(call, "call"), docof(call, d), field(call, "function", raw), unparen(raw, f), gcc_name(f, "SimpleNamespace"), field(call, "arguments", al), child(al, kw, _), kind(kw, "keyword_argument"), field(kw, "name", k), text(k, name), @nfkc(name) = "render", line(call, line).
gcc_result(d, line, "DYNAMIC_RENDER_ASSIGNMENT", "assignment to .render creates a callable grammar render hook", target, "") :- gcc_file(d), asgstmt(a, _), docof(a, d), assign_target(a, raw), unparen(raw, target), kind(target, "attribute"), field(target, "attribute", attr), text(attr, name), @nfkc(name) = "render", line(a, line).
'''


def outputs():
    text=r'''
.violation GCCFact(at: span, violation_type: symbol, detail: symbol, key: symbol, formatted: symbol, origin: node, tag: symbol) code "GCC_FACT" "{detail}"
GCCFact(@place(path, line), code, detail, cat(path, ":", to_string(line), ":", code, ":", detail), cat(path, ":", to_string(line), ": ", code, ": ", detail), n, tag) :-
 option("entryPoint", _), gcc_result(d, line, code, detail, n, tag), gcc_path(d, path).
'''
    for i,code in enumerate(['CALLABLE_INIT_PARAM','CALLABLE_FIELD','CONSTRUCTOR_FIELD_CALLED_IN_RENDER','UNTYPED_CALLED_CONSTRUCTOR_FIELD','ANY_CALLED_CONSTRUCTOR_FIELD','CALLABLE_CONSTRUCTOR_ARG','SIMPLE_NAMESPACE_RENDER_HOOK','DYNAMIC_RENDER_ASSIGNMENT']):
        name='GCCFinding'+str(i)
        text+=f'.severity {name} error\n.violation {name}(at: span, origin: node, tag: symbol, detail: symbol) code {json.dumps(code)} "{{detail}}"\n'
        text+=f'{name}(@place(path, line), n, tag, detail) :- !option("entryPoint", _), gcc_result(d, line, {json.dumps(code)}, detail, n, tag), gcc_path(d, path).\n'
    return text
