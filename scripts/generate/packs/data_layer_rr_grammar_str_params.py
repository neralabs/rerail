"""Translate grammar string parameters, constructor prepasses, and ordered render events."""
import json
from data_layer_rr_text import module


def grammar_str_params(source,target,header):
    original=module(source,'grammar_str_params');q=json.dumps
    text=header('grammar_str_params',target)+'#include "python/print.rr"\n'
    text+='.decl gsp_default_class(name: symbol)\n'+''.join(f'gsp_default_class({q(name)}).\n' for name in sorted(original.ALLOWLISTED_STR_CLASSES))
    text+='.decl gsp_default_param(cls: symbol, name: symbol)\n'+''.join(f'gsp_default_param({q(cls)}, {q(name)}).\n' for cls,name in sorted(original.ALLOWLISTED_STR_PARAMS))
    text+='.decl gsp_baseline(order: number, code: symbol, count: number)\n'+''.join(f'gsp_baseline({i}, {q(code)}, {count}).\n' for i,(code,count) in enumerate(original.BASELINE_COUNTS.items()))
    hint='\nMechanical cleanup:\n  uv run python scripts/fix_grammar_linter_violations.py --write\n  uv run python scripts/codemod_hoist_render_raw_strings.py impl/common/l2100_sql_grammar\n'
    text+='.decl gsp_hint(value: symbol)\ngsp_hint('+q(hint)+').\n'
    return text+configuration()+detector()+outputs()


def configuration():
    return r'''
.violation GSPClass(at: span, name: symbol) code "GSP_CLASS" "{name}"
GSPClass(@place("", 0), name) :- option("entryPoint", "configuration"), gsp_default_class(name).
.violation GSPParam(at: span, cls: symbol, name: symbol) code "GSP_PARAM" "{name}"
GSPParam(@place("", 0), cls, name) :- option("entryPoint", "configuration"), gsp_default_param(cls, name).
.violation GSPBaseline(at: span, ordinal: number, code: symbol, count: number) code "GSP_BASELINE" "{code}"
GSPBaseline(@place("", 0), i, code, count) :- option("entryPoint", "configuration"), gsp_baseline(i, code, count).
.violation GSPText(at: span, value: symbol) code "GSP_TEXT" "{value}"
GSPText(@place("", 0), value) :- option("entryPoint", "print_autofix_hint"), gsp_hint(value).
.decl gsp_allow_class(name: symbol)
gsp_allow_class(name) :- option("entryPoint", _), option_item("classes", _, name).
gsp_allow_class(name) :- !option("entryPoint", _), gsp_default_class(name).
.decl gsp_allow_param(cls: symbol, name: symbol)
gsp_allow_param(cls, name) :- option("entryPoint", _), option_item("paramClasses", i, cls), option_item("paramNames", i, name).
gsp_allow_param(cls, name) :- !option("entryPoint", _), gsp_default_param(cls, name).
.decl gsp_trigger(text: symbol)
gsp_trigger("__init__"). gsp_trigger("render("). gsp_trigger("str"). gsp_trigger("Sql").
.violation GSPRead(at: span) code "GSP_READ" "read"
GSPRead(@place("", 0)) :- option("entryPoint", "source_gate"), option("providedSource", source), gsp_trigger(t), contains(t, source).
'''


def detector():
    return r'''
.decl gsp_result(path: symbol, line: number, cls: symbol, name: symbol, annotation: symbol, code: symbol, detail: symbol, ordinal: number, first: number, second: number)
.decl gsp_path(d: number, path: symbol)
gsp_path(d, original) :- doc(d, logical, _), doc_role(d, "primary"), option_item("documentPaths", i, logical), option_item("originalPaths", i, original).
gsp_path(d, path) :- doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
.decl gsp_test_path(d: number)
gsp_test_path(d) :- gsp_path(d, path), match_all("(?:\\A|/)(?:test|tests)(?:/|\\z)", path, _, _, _).
.decl gsp_hidden(d: number)
gsp_hidden(d) :- path_part(d, p), (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp").
.decl gsp_source_trigger(d: number)
gsp_source_trigger(d) :- source(d, sp), text(sp, source), gsp_trigger(t), contains(t, source).
.decl gsp_selected(d: number)
gsp_selected(d) :- option("entryPoint", _), gsp_path(d, path), suffix(".py", path), !gsp_test_path(d), !syntax_error(d, _, _), gsp_source_trigger(d).
gsp_selected(d) :- !option("entryPoint", _), gsp_path(d, path), prefix("impl/common/l2100_sql_grammar/", path), suffix(".py", path), !gsp_test_path(d), !gsp_hidden(d), !syntax_error(d, _, _), gsp_source_trigger(d).
.decl gsp_ancestor(n: node, a: node)
gsp_ancestor(n, a) :- ast_parent(n, a).
gsp_ancestor(n, a) :- ast_parent(n, p), gsp_ancestor(p, a).
.decl gsp_class(c: node, name: symbol)
gsp_class(c, @nfkc(name)) :- kind(c, "class_definition"), def_name(c, name).
.decl gsp_base(c: node, name: symbol)
gsp_base(c, @nfkc(name)) :- gsp_class(c, _), field(c, "superclasses", al), child(al, raw, _), unparen(raw, n), kind(n, "identifier"), text(n, name).
gsp_base(c, @nfkc(name)) :- gsp_class(c, _), field(c, "superclasses", al), child(al, raw, _), unparen(raw, n), kind(n, "attribute"), field(n, "attribute", a), text(a, name).
.decl gsp_sync(f: node, name: symbol)
gsp_sync(f, @nfkc(name)) :- kind(f, "function_definition"), !is_async(f), def_name(f, name).
.decl gsp_direct_method(c: node, f: node, index: number, name: symbol)
gsp_direct_method(c, f, i, name) :- gsp_class(c, _), field(c, "body", body), body_stmt(body, stmt, i), defstmt(f, stmt), gsp_sync(f, name).
.decl gsp_grammar(c: node)
gsp_grammar(c) :- gsp_base(c, name), (prefix("Sql", name) ; prefix("_Sql", name)).
gsp_grammar(c) :- gsp_direct_method(c, _, _, "render").
.decl gsp_enabled(c: node, name: symbol)
gsp_enabled(c, name) :- gsp_class(c, name), gsp_grammar(c), !gsp_allow_class(name).
.decl gsp_init(c: node, f: node, index: number)
gsp_init(c, f, i) :- gsp_enabled(c, _), gsp_direct_method(c, f, i, "__init__").
.decl gsp_contains_str(text: symbol)
gsp_contains_str(text) :- annotation_text(_, text), match_all("(?:\\A|\\|) *str *(?:\\||\\z)", text, _, _, _).
.decl gsp_str_parameter(c: node, nm: node, name: symbol, annotation: symbol, method_index: number, param_index: number)
gsp_str_parameter(c, nm, @nfkc(name), annotation, mi, pi) :- gsp_enabled(c, cls), gsp_init(c, f, mi), sig_param(f, _, nm, pi, "pos", ann, _),
 text(nm, name), @nfkc(name) != "self", !gsp_allow_param(cls, @nfkc(name)), !unset(ann), annotation_text(ann, annotation), gsp_contains_str(annotation).
.decl gsp_slot(c: node, name: symbol)
gsp_slot(c, cat("_", name)) :- gsp_str_parameter(c, _, name, _, _, _).
.decl gsp_current_class(f: node, c: node)
gsp_current_class(f, c) :- gsp_sync(f, _), gsp_ancestor(f, c), gsp_class(c, _), visit_index(c, i),
 i = max j : { gsp_ancestor(f, a), gsp_class(a, _), visit_index(a, j) }.
.decl gsp_render(f: node, c: node)
gsp_render(f, c) :- gsp_sync(f, "render"), gsp_current_class(f, c), gsp_enabled(c, _).
.decl gsp_context(n: node, c: node)
gsp_context(n, c) :- gsp_ancestor(n, f), gsp_render(f, c), visit_index(f, i),
 i = max j : { gsp_ancestor(n, other), gsp_render(other, _), visit_index(other, j) }.
.decl gsp_slot_allowed(c: node, slot: symbol)
gsp_slot_allowed(c, slot) :- gsp_slot(c, slot).
gsp_slot_allowed(c, slot) :- gsp_context(n, c), gsp_call_slot(n, _, slot), !gsp_init(c, _, _).
gsp_slot_allowed(c, slot) :- gsp_context(n, c), gsp_concat_slot(n, _, _, slot), !gsp_init(c, _, _).
.decl gsp_str_call(c: node)
gsp_str_call(c) :- kind(c, "call"), field(c, "function", raw), unparen(raw, f), kind(f, "identifier"), text(f, name), @nfkc(name) = "str".
.decl gsp_self_slot(n: node, name: symbol)
gsp_self_slot(n, @nfkc(name)) :- kind(n, "attribute"), field(n, "attribute", a), text(a, name), field(n, "object", raw), unparen(raw, obj), kind(obj, "identifier"), text(obj, value), @nfkc(value) = "self".
.decl gsp_call_slot(c: node, index: number, slot: symbol)
gsp_call_slot(c, i, slot) :- gsp_str_call(c), field(c, "arguments", al), child(al, raw, i), !kind(raw, "keyword_argument"), !kind(raw, "dictionary_splat"), !kind(raw, "list_splat"), !kind(raw, "comment"), unparen(raw, a), gsp_self_slot(a, slot).
.decl gsp_side(n: node, index: number, call: node)
gsp_side(n, 0, c) :- kind(n, "binary_operator"), op(n, "+"), field(n, "left", raw), unparen(raw, c), kind(c, "call").
gsp_side(n, 1, c) :- kind(n, "binary_operator"), op(n, "+"), field(n, "right", raw), unparen(raw, c), kind(c, "call").
.decl gsp_concat_slot(n: node, side: number, index: number, slot: symbol)
gsp_concat_slot(n, side, i, slot) :- gsp_side(n, side, c), gsp_call_slot(c, i, slot).
gsp_result(path, line, cls, name, annotation, "STR_PARAM", cat("__init__ parameter '", name, "' typed as '", annotation, "' - should be a branded type or enum"), i, mi, pi) :-
 gsp_str_parameter(c, nm, name, annotation, mi, pi), gsp_enabled(c, cls), docof(c, d), gsp_selected(d), gsp_path(d, path), visit_index(c, i), line(nm, line).
gsp_result(path, line, cls, slot, "", "STR_CALL_IN_RENDER", cat("str(self.", slot, ") in render() - slot stores a branded type but is unwrapped to str"), i, 0, ai) :-
 gsp_context(n, c), gsp_enabled(c, cls), gsp_call_slot(n, ai, slot), gsp_slot_allowed(c, slot), docof(n, d), gsp_selected(d), gsp_path(d, path), visit_index(n, i), line(n, line).
gsp_result(path, line, cls, slot, "", "STR_CONCAT_IN_RENDER", cat("str(self.", slot, ") used in + concatenation in render()"), i, side, ai) :-
 gsp_context(n, c), gsp_enabled(c, cls), gsp_concat_slot(n, side, ai, slot), gsp_slot_allowed(c, slot), docof(n, d), gsp_selected(d), gsp_path(d, path), visit_index(n, i), line(n, line).
'''


def outputs():
    text=r'''
.violation GSPFact(at: span, class_name: symbol, param_name: symbol, annotation: symbol, violation_type: symbol, detail: symbol, ordinal: number, first: number, second: number) code "GSP_FACT" "{detail}"
GSPFact(@place(path, line), cls, name, annotation, code, detail, i, a, b) :- option("entryPoint", _), gsp_result(path, line, cls, name, annotation, code, detail, i, a, b).
.decl gsp_exceeded(code: symbol)
gsp_exceeded(code) :- gsp_baseline(_, code, baseline), n = count : { gsp_result(_, _, _, _, _, code, _, _, _, _) }, n > baseline.
'''
    for name,code in [('GSPStringParam','STR_PARAM'),('GSPStringCall','STR_CALL_IN_RENDER'),('GSPStringConcat','STR_CONCAT_IN_RENDER')]:
        text+=f'.severity {name} error\n.violation {name}(at: span, cls: symbol, name: symbol, detail: symbol, ordinal: number, first: number, second: number) code {json.dumps(code)} "{{cls}}.{{name}}: {{detail}}"\n'
        text+=f'{name}(@place(path, line), cls, name, detail, i, a, b) :- !option("entryPoint", _), gsp_exceeded({json.dumps(code)}), gsp_result(path, line, cls, name, _, {json.dumps(code)}, detail, i, a, b).\n'
    return text
