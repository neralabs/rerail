"""Translate lazy-result method eligibility and wrapper exemptions to native relations."""
import json
import re
from data_layer_rr_text import module, character_class


def lazy_result_usage(source, target, header):
    original = module(source, 'lazy_result_usage')
    text = header('lazy_result_usage', target)
    text = re.sub(r'^\.scope include .*$', '.scope include "**/*.py"', text, flags=re.M)
    text = re.sub(r'^\.scope exclude .*$', '.scope exclude', text, flags=re.M)
    text += '#include "python/print.rr"\n'
    for relation, values in [('lru_single', original.SINGLE_VALUE_METHODS),
            ('lru_sequence', original.SEQUENCE_METHODS), ('lru_class_pattern', original.TRANSACTION_CLASS_PATTERNS),
            ('lru_skip_class', original.SKIP_CLASSES), ('lru_skip_pattern', original.SKIP_PATTERNS)]:
        text += f'.decl {relation}(value: symbol)\n'
        text += ''.join(f'{relation}({json.dumps(v)}).\n' for v in sorted(values))
    text += '.decl lru_layer(path: symbol)\nlru_layer(p) :- lru_path(_, p), match('+json.dumps('(?s)l'+character_class(str.isdecimal)+r'{4}_[^/\n][^/]*/.*')+', p).\n'
    return text + projection() + semantics()


def projection():
    return r'''
.decl lru_graph()
lru_graph() :- option_item("astKinds", 0, _).
.decl lru_gkind(n: node, k: symbol)
lru_gkind(n, k) :- option_item("astKinds", n, k).
.decl lru_edge(p: node, field: symbol, c: node)
lru_edge(to_number(p), field, c) :- option_item("astParents", c, p), p != "-1", option_item("astFields", c, field).
.decl lru_ancestor(n: node, a: node)
lru_ancestor(n, a) :- lru_edge(a, _, n).
lru_ancestor(n, a) :- lru_edge(p, _, n), lru_ancestor(p, a).
.decl lru_function(f: node, name: symbol)
lru_function(f, @nfkc(name)) :- !lru_graph(), kind(f, "function_definition"), def_name(f, name).
lru_function(f, name) :- lru_gkind(f, k), (k = "FunctionDef" ; k = "AsyncFunctionDef"), option_item("astNames", f, name).
.decl lru_class(c: node, name: symbol)
lru_class(c, @nfkc(name)) :- !lru_graph(), kind(c, "class_definition"), def_name(c, name).
lru_class(c, name) :- lru_gkind(c, "ClassDef"), option_item("astNames", c, name).
.decl lru_current_class(f: node, c: node)
lru_current_class(f, c) :- !lru_graph(), lru_function(f, _), nearest(f, c, "class_definition").
lru_current_class(f, c) :- lru_graph(), lru_function(f, _), lru_ancestor(f, c), lru_class(c, _),
 c = max a : { lru_ancestor(f, a), lru_class(a, _) }.
.decl lru_direct_method(c: node, name: symbol)
lru_direct_method(c, name) :- !lru_graph(), lru_class(c, _), field(c, "body", b), body_stmt(b, st, _), defstmt(f, st), lru_function(f, name).
lru_direct_method(c, name) :- lru_class(c, _), lru_edge(c, "body", f), lru_function(f, name).
.decl lru_position(f: node, d: number, line: number, entry_order: number, exit_order: number)
lru_position(f, d, l, i, e) :- !lru_graph(), lru_function(f, _), node(d, f, _, _, e), line(f, l), visit_index(f, i).
lru_position(f, 0, to_number(l), f, to_number(e)) :- lru_gkind(f, _), lru_function(f, _), option_item("astLines", f, l), option_item("astExitOrder", f, e).
.decl lru_path(d: number, path: symbol)
lru_path(d, path) :- !lru_graph(), doc(d, physical, _), doc_role(d, "primary"), option_item("documentPaths", i, physical), option_item("scopePaths", i, path).
lru_path(d, path) :- !lru_graph(), doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
lru_path(0, path) :- lru_graph(), option("originalPath", path).
.decl lru_name(n: node, name: symbol)
lru_name(n, @nfkc(name)) :- !lru_graph(), unparen(n, p), ast_name(p, name).
lru_name(n, name) :- lru_gkind(n, "Name"), option_item("astIds", n, name).
.decl lru_attr(n: node, name: symbol)
lru_attr(n, @nfkc(name)) :- !lru_graph(), unparen(n, a), attr_of(a, _, name).
lru_attr(n, name) :- lru_gkind(n, "Attribute"), option_item("astAttrs", n, name).
.decl lru_call(n: node, fn: node)
lru_call(n, fn) :- !lru_graph(), unparen(n, c), kind(c, "call"), field(c, "function", raw), unparen(raw, fn).
lru_call(n, fn) :- lru_gkind(n, "Call"), lru_edge(n, "func", fn).
.decl lru_base_edge(n: node, value: node)
lru_base_edge(n, v) :- !lru_graph(), kind(n, "subscript"), field(n, "value", raw), unparen(raw, v).
lru_base_edge(n, v) :- !lru_graph(), kind(n, "generic_type"), child(n, v, 0).
lru_base_edge(n, v) :- !lru_graph(), kind(n, "type"), child(n, v, 0).
lru_base_edge(n, v) :- lru_gkind(n, "Subscript"), lru_edge(n, "value", v).
.decl lru_base_name(n: node, name: symbol)
lru_base_name(n, name) :- lru_name(n, name).
lru_base_name(n, name) :- lru_attr(n, name).
lru_base_name(n, name) :- lru_base_edge(n, v), lru_base_name(v, name).
.decl lru_protocol(c: node)
lru_protocol(c) :- !lru_graph(), lru_class(c, _), field(c, "superclasses", args), child(args, b, _), !kind(b, "keyword_argument"), lru_base_name(b, name), (name = "Protocol" ; name = "ABC").
lru_protocol(c) :- lru_class(c, _), lru_edge(c, "bases", b), lru_base_name(b, name), (name = "Protocol" ; name = "ABC").
.decl lru_decorator(f: node, e: node)
lru_decorator(f, e) :- !lru_graph(), decorator(f, e).
lru_decorator(f, e) :- lru_function(f, _), lru_edge(f, "decorator_list", e).
.decl lru_decorator_name(n: node, name: symbol)
lru_decorator_name(n, name) :- lru_name(n, name).
lru_decorator_name(n, name) :- lru_attr(n, name).
lru_decorator_name(n, name) :- lru_call(n, fn), lru_decorator_name(fn, name).
.decl lru_descendant(n: node, f: node)
lru_descendant(n, f) :- !lru_graph(), lru_function(f, _), defstmt(f, st), within(n, st).
lru_descendant(n, f) :- lru_ancestor(n, f), lru_function(f, _).
.decl lru_reference(f: node, name: symbol)
lru_reference(f, name) :- lru_descendant(n, f), lru_name(n, name).
lru_reference(f, name) :- lru_descendant(n, f), lru_attr(n, name).
.decl lru_return(f: node, text: symbol)
lru_return(f, text) :- !lru_graph(), lru_function(f, _), field(f, "return_type", ann), annotation_text(ann, text).
lru_return(f, "None") :- !lru_graph(), lru_function(f, _), !field(f, "return_type", _).
lru_return(f, text) :- lru_edge(f, "returns", ann), sig_param(_, _, nm, _, _, a, _), text(nm, key), key = cat("e", to_string(ann)), annotation_text(a, text).
lru_return(f, "None") :- lru_gkind(f, _), lru_function(f, _), !lru_edge(f, "returns", _).
.decl lru_placeholder(f: node)
lru_placeholder(f) :- !lru_graph(), lru_function(f, _), field(f, "body", b), body_len(b, 1), body_stmt(b, st, 0), kind(st, "pass_statement").
lru_placeholder(f) :- !lru_graph(), lru_function(f, _), field(f, "body", b), body_len(b, 1), body_stmt(b, st, 0), kind(st, "expression_statement"), child(st, raw, 0), unparen(raw, e), kind(e, "ellipsis").
lru_placeholder(f) :- lru_gkind(f, _), lru_function(f, _), 1 = count : { lru_edge(f, "body", _) }, lru_edge(f, "body", st), lru_gkind(st, "Pass").
lru_placeholder(f) :- lru_gkind(f, _), lru_function(f, _), 1 = count : { lru_edge(f, "body", _) }, lru_edge(f, "body", st), lru_gkind(st, "Expr"), lru_edge(st, "value", e), option_item("astConstantTypes", e, "ellipsis").
'''


def semantics():
    return r'''
.decl lru_skipped(path: symbol)
lru_skipped(path) :- lru_path(_, path), lru_skip_pattern(pattern), contains(pattern, path).
lru_skipped(path) :- option("pathProbe", path), lru_skip_pattern(pattern), contains(pattern, path).
.decl lru_root(path: symbol)
lru_root(path) :- lru_path(_, path), (prefix("impl/", path) ; prefix("data_layer/", path) ; prefix("_internal/", path)).
lru_root(path) :- lru_layer(path).
.decl lru_test(path: symbol)
lru_test(path) :- lru_path(_, path), match("(?s)(?:.*/)?(?:tests|test)(?:/.*)?", path).
.decl lru_hidden(path: symbol)
lru_hidden(path) :- lru_path(_, path), match("(?s)(?:.*/)?(?:\\.git|\\.venv|venv|__pycache__|tmp)(?:/.*)?", path).
.decl lru_selected(d: number)
lru_selected(0) :- lru_graph(), option("disabled", "false").
lru_selected(d) :- !lru_graph(), lru_path(d, path), suffix(".py", path), !lru_skipped(path), !lru_test(path), !syntax_error(d, _, _), option("entryPoint", entry), entry != "find_violations", entry != "collector".
lru_selected(d) :- !lru_graph(), lru_path(d, path), suffix(".py", path), lru_root(path), !lru_skipped(path), !lru_test(path), !syntax_error(d, _, _), option("entryPoint", "find_violations").
lru_selected(d) :- !lru_graph(), lru_path(d, path), suffix(".py", path), lru_root(path), !lru_skipped(path), !lru_test(path), !lru_hidden(path), !syntax_error(d, _, _), !option("entryPoint", _).
.decl lru_lazy_authority(method: symbol, name: symbol)
lru_lazy_authority(method, cat(method, "_lazy")) :- (lru_single(method) ; lru_sequence(method)).
lru_lazy_authority(method, "execute_lazy") :- lru_single(method), (method = "first" ; method = "one" ; method = "one_or_none" ; method = "select_one").
.decl lru_exempt(f: node)
lru_exempt(f) :- lru_function(f, method), lru_current_class(f, c), lru_lazy_authority(method, name), lru_direct_method(c, name).
lru_exempt(f) :- lru_function(f, method), lru_lazy_authority(method, name), lru_reference(f, name).
lru_exempt(f) :- lru_function(f, _), lru_current_class(f, c), lru_protocol(c).
lru_exempt(f) :- lru_decorator(f, e), lru_decorator_name(e, name), (name = "legacy_sync" ; name = "legacy_async" ; name = "abstractmethod").
lru_exempt(f) :- lru_placeholder(f).
.decl lru_eligible(f: node, class_name: symbol, method: symbol, return_type: symbol)
lru_eligible(f, cls, method, text) :- lru_function(f, method), (lru_single(method) ; lru_sequence(method)),
 lru_current_class(f, c), lru_class(c, cls), !lru_skip_class(cls), lru_class_pattern(pattern), contains(pattern, cls),
 !match("_*(?:Async|Lazy).*", cls), !lru_exempt(f), lru_return(f, text),
 !contains("LazyResult", text), !contains("LazySeq", text), !contains("DeferredFuture", text).
.violation LRUFact(at: span, class_name: symbol, method_name: symbol, return_type: symbol, entry_order: number, exit_order: number) code "LRU_FACT" "{class_name}.{method_name} returns {return_type}"
LRUFact(@place(path, l), cls, method, ret, i, e) :- option("entryPoint", _), lru_position(f, d, l, i, e), lru_path(d, path), lru_selected(d), lru_eligible(f, cls, method, ret).
.decl lru_baseline(total: symbol)
.input lru_baseline(IO=json, filename="scripts/lazy_result_baseline.json", path="/total")
.decl lru_budget(value: number)
lru_budget(to_number(total)) :- lru_baseline(total).
lru_budget(0) :- !lru_baseline(_).
.decl lru_violation(f: node, d: number, line: number, cls: symbol, method: symbol, ret: symbol, i: number, e: number)
lru_violation(f, d, l, cls, method, ret, i, e) :- lru_position(f, d, l, i, e), lru_selected(d), lru_eligible(f, cls, method, ret).
.violation LazyResultUsage(at: span, class_name: symbol, method_name: symbol, return_type: symbol, entry_order: number, exit_order: number) code "lazy_result_usage" "{class_name}.{method_name} returns {return_type}; expected LazyResult/LazySeq"
LazyResultUsage(@place(path, l), cls, method, ret, i, e) :- !option("entryPoint", _), lru_violation(_, d, l, cls, method, ret, i, e), lru_path(d, path),
 total = count : { lru_violation(_, _, _, _, _, _, _, _) }, lru_budget(budget), total > budget.
.severity LazyResultUsage error
.violation LRUSkip(at: span) code "LRU_SKIP" "skip"
LRUSkip(@place("", 0)) :- option("entryPoint", "should_skip"), option("pathProbe", path), lru_skipped(path).
'''
