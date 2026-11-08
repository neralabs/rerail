"""Translate async/sync delegation using shared native syntax and typed AST arguments."""
import re
from data_layer_rr_delegation_source import source_relations


def async_sync_delegation(source,target,header):
    text=header('async_sync_delegation',target)+'#include "python/print.rr"\n'
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    return text+configuration()+helpers()+analysis()+source_relations()


def configuration():
    return r'''
.violation ASDConfig(at: span, name: symbol, value: symbol) code "ASD_CONFIG" "{value}"
ASDConfig(@place("", 0), "LINTER_NAME", "async_sync_delegation_category_c") :- option("entryPoint", "configuration").
ASDConfig(@place("", 0), "ALLOWLIST_SIGNATURE", "") :- option("entryPoint", "configuration").
'''


def helpers():
    return r'''
.decl asdh_kind(n: node, kind: symbol)
asdh_kind(n, k) :- option_item("astKinds", n, k).
.decl asdh_edge(n: node, field: symbol, c: node)
asdh_edge(to_number(parent), field, c) :- option_item("astParents", c, parent), parent != "-1", option_item("astFields", c, field).
.violation ASDTrue(at: span) code "ASD_TRUE" "true"
ASDTrue(@place("", 0)) :- option("entryPoint", _), 1 = 0.
.violation ASDValue(at: span, value: symbol) code "ASD_VALUE" "{value}"
ASDValue(@place("", 0), "") :- option("entryPoint", _), 1 = 0.
.violation ASDNode(at: span, n: node, ordinal: number) code "ASD_NODE" "node"
ASDNode(@place("", 0), n, 0) :- asdh_kind(n, _), 1 = 0.
.violation ASDRequirement(at: span, base: symbol, lazy: symbol, counterpart: symbol, decorator: symbol) code "ASD_REQUIREMENT" "requirements"
ASDRequirement(@place("", 0), "", "", "", "") :- option("entryPoint", _), 1 = 0.
.decl asdh_body(f: node, n: node, ordinal: number)
asdh_body(f, n, to_number(i)) :- asdh_edge(f, "body", n), option_item("astOrdinals", n, i).
.decl asdh_docstring(f: node)
asdh_docstring(f) :- asdh_body(f, expr, 0), asdh_kind(expr, "Expr"), asdh_edge(expr, "value", value), asdh_kind(value, "Constant"), option_item("astValueTypes", value, "str").
.decl asdh_statement(f: node, n: node, ordinal: number)
asdh_statement(f, n, i) :- asdh_body(f, n, i), !asdh_docstring(f).
asdh_statement(f, n, i) :- asdh_body(f, n, i), asdh_docstring(f), i > 0.
.decl asdh_callable(n: node, value: symbol)
asdh_callable(n, value) :- asdh_kind(n, "Name"), option_item("astIds", n, value).
asdh_callable(n, cat(parent, ".", attr)) :- asdh_kind(n, "Attribute"), asdh_edge(n, "value", v), asdh_callable(v, parent), n < v, option_item("astAttrs", n, attr).
asdh_callable(n, attr) :- asdh_kind(n, "Attribute"), asdh_edge(n, "value", v), asdh_kind(v, k), k != "Name", k != "Attribute", option_item("astAttrs", n, attr).
.decl asdh_decorator_leaf(n: node, value: symbol)
asdh_decorator_leaf(n, value) :- asdh_kind(n, "Name"), option_item("astIds", n, value).
asdh_decorator_leaf(n, value) :- asdh_kind(n, "Attribute"), option_item("astAttrs", n, value).
asdh_decorator_leaf(n, value) :- asdh_kind(n, "Call"), asdh_edge(n, "func", fn), asdh_decorator_leaf(fn, value).
.decl asdh_decorator(f: node, d: node, name: symbol, ordinal: number)
asdh_decorator(f, d, name, to_number(i)) :- asdh_edge(f, "decorator_list", d), asdh_decorator_leaf(d, name), option_item("astOrdinals", d, i).
.decl asdh_bare(f: node, name: symbol)
asdh_bare(f, name) :- asdh_decorator(f, d, name, _), !asdh_kind(d, "Call").
.decl asdh_abstract(f: node)
asdh_abstract(f) :- asdh_decorator(f, d, "abstractmethod", _), !asdh_kind(d, "Call").
asdh_abstract(f) :- asdh_kind(f, _), count : { asdh_statement(f, _, _) } = 1, asdh_statement(f, n, _), asdh_kind(n, "Pass").
asdh_abstract(f) :- asdh_kind(f, _), count : { asdh_statement(f, _, _) } = 1, asdh_statement(f, n, _), asdh_kind(n, "Expr"), asdh_edge(n, "value", value), asdh_kind(value, "Constant"), option_item("astValueTypes", value, "ellipsis").
asdh_abstract(f) :- asdh_kind(f, _), count : { asdh_statement(f, _, _) } = 1, asdh_statement(f, n, _), asdh_kind(n, "Raise"), asdh_edge(n, "exc", call), asdh_kind(call, "Call"), asdh_edge(call, "func", fn), asdh_kind(fn, "Name"), option_item("astIds", fn, "NotImplementedError").
.decl asdh_desc(n: node, a: node)
asdh_desc(n, a) :- asdh_edge(a, _, n).
asdh_desc(n, a) :- asdh_edge(p, _, n), asdh_desc(p, a).
.decl asdh_parameter(f: node, p: node)
asdh_parameter(f, p) :- asdh_edge(f, "args", args), asdh_edge(args, field, p), (field = "posonlyargs" ; field = "args" ; field = "kwonlyargs").
.decl asdh_annotation_part(p: node, n: node)
asdh_annotation_part(p, a) :- asdh_edge(p, "annotation", a).
asdh_annotation_part(p, n) :- asdh_edge(p, "annotation", a), asdh_desc(n, a).
.decl asdh_annotation_word(p: node, word: symbol)
asdh_annotation_word(p, word) :- asdh_annotation_part(p, n), (option_item("astIds", n, text) ; option_item("astAttrs", n, text) ; option_item("astArgNames", n, text) ; option_item("astKeywords", n, text) ; option_item("astConstantText", n, text)), (word = "Callable" ; word = "Transaction"), contains(word, text).
.decl asdh_callback(f: node)
asdh_callback(f) :- asdh_parameter(f, p), option_item("astArgNames", p, name), (name = "fn" ; name = "callback").
asdh_callback(f) :- asdh_parameter(f, p), asdh_annotation_word(p, "Callable").
.decl asdh_txn(f: node)
asdh_txn(f) :- asdh_parameter(f, p), !asdh_callback(f), option_item("astArgNames", p, name), (name = "txn" ; name = "tx" ; name = "vtxn" ; name = "read_txn" ; name = "write_txn" ; name = "base_txn").
asdh_txn(f) :- asdh_parameter(f, p), !asdh_callback(f), asdh_annotation_word(p, "Transaction").
.decl asdh_normalize(name: symbol, value: symbol)
asdh_normalize(name, @slice(name, 0, strlen(name) - 6)) :- option("helperName", name), suffix("_async", name).
asdh_normalize(name, @slice(name, 0, strlen(name) - 5)) :- option("helperName", name), !suffix("_async", name), suffix("_sync", name).
asdh_normalize(name, name) :- option("helperName", name), !suffix("_async", name), !suffix("_sync", name).
.decl asdh_requirement_kind(f: node, decorator: symbol, priority: number)
asdh_requirement_kind(f, "legacy_sync", 0) :- asdh_bare(f, "legacy_sync").
asdh_requirement_kind(f, "legacy_async", 1) :- asdh_bare(f, "legacy_async").
asdh_requirement_kind(f, "legacy_module_sync", 2) :- asdh_bare(f, "legacy_module_sync").
asdh_requirement_kind(f, "legacy_module_async", 3) :- asdh_bare(f, "legacy_module_async").
.decl asdh_requirement_selected(f: node, decorator: symbol)
asdh_requirement_selected(f, decorator) :- asdh_requirement_kind(f, decorator, priority), priority = min i : { asdh_requirement_kind(f, _, i) }.
.decl asdh_base(f: node, decorator: symbol, base: symbol)
asdh_base(f, "legacy_sync", @replace_all(name, "_sync\\z", "")) :- asdh_requirement_selected(f, "legacy_sync"), option_item("astNames", f, name).
asdh_base(f, "legacy_async", @replace_all(name, "_async\\z", "")) :- asdh_requirement_selected(f, "legacy_async"), option_item("astNames", f, name).
asdh_base(f, "legacy_module_sync", name) :- asdh_requirement_selected(f, "legacy_module_sync"), option_item("astNames", f, name).
asdh_base(f, "legacy_module_async", @replace_all(name, "_async\\z", "")) :- asdh_requirement_selected(f, "legacy_module_async"), option_item("astNames", f, name).
.decl asdh_lazy_keyword(d: node, value: symbol)
asdh_lazy_keyword(d, value) :- asdh_kind(d, "Call"), asdh_edge(d, "keywords", kw), option_item("astKeywords", kw, "lazy_name"), asdh_edge(kw, "value", n), asdh_kind(n, "Constant"), option_item("astValueTypes", n, "str"), option_item("astConstantText", n, value), option_item("astOrdinals", kw, rank), index = to_number(rank), index = min i : { asdh_edge(d, "keywords", other), option_item("astKeywords", other, "lazy_name"), asdh_edge(other, "value", v), asdh_kind(v, "Constant"), option_item("astValueTypes", v, "str"), option_item("astOrdinals", other, r), i = to_number(r) }.
.decl asdh_lazy_override(f: node, decorator: symbol, value: symbol)
asdh_lazy_override(f, decorator, value) :- asdh_requirement_selected(f, decorator), asdh_decorator(f, d, decorator, i), i = min j : { asdh_decorator(f, _, decorator, j) }, asdh_lazy_keyword(d, value), value != "".
.decl asdh_lazy_name(f: node, decorator: symbol, lazy: symbol)
asdh_lazy_name(f, decorator, lazy) :- asdh_lazy_override(f, decorator, lazy).
asdh_lazy_name(f, decorator, cat(base, "_lazy")) :- asdh_base(f, decorator, base), !asdh_lazy_override(f, decorator, _).
.decl asdh_requirement(f: node, base: symbol, lazy: symbol, counterpart: symbol, decorator: symbol)
asdh_requirement(f, base, lazy, cat(base, "_async"), "legacy_async") :- asdh_base(f, "legacy_sync", base), asdh_lazy_name(f, "legacy_sync", lazy).
asdh_requirement(f, base, lazy, base, "legacy_sync") :- asdh_base(f, "legacy_async", base), asdh_lazy_name(f, "legacy_async", lazy).
asdh_requirement(f, base, lazy, cat(base, "_async"), "legacy_module_async") :- asdh_base(f, "legacy_module_sync", base), asdh_lazy_name(f, "legacy_module_sync", lazy).
asdh_requirement(f, base, lazy, base, "legacy_module_sync") :- asdh_base(f, "legacy_module_async", base), asdh_lazy_name(f, "legacy_module_async", lazy).
ASDTrue(@place("", 0)) :- option("entryPoint", "_has_mixed_lazy_execution_name"), option("helperName", name), normalized = @lower(name), contains("lazy", normalized), (contains("async", normalized) ; contains("sync", normalized)).
ASDTrue(@place("", 0)) :- option("entryPoint", "_is_abstract_like"), asdh_abstract(0).
ASDTrue(@place("", 0)) :- option("entryPoint", "_has_decorator_named"), asdh_decorator(0, _, name, _), option_item("decoratorNames", _, name).
ASDTrue(@place("", 0)) :- option("entryPoint", "_is_lazy_decorated"), asdh_decorator(0, _, name, _), (name = "lazy" ; name = "pure_lazy").
ASDTrue(@place("", 0)) :- option("entryPoint", "_looks_like_callback_entrypoint"), asdh_callback(0).
ASDTrue(@place("", 0)) :- option("entryPoint", "_looks_like_transaction_bound_eager_entrypoint"), asdh_txn(0).
ASDValue(@place("", 0), value) :- option("entryPoint", "_normalize_helper_family"), asdh_normalize(_, value).
ASDValue(@place("", 0), value) :- option("entryPoint", "_extract_callable_name"), asdh_callable(0, value).
ASDValue(@place("", 0), value) :- option("entryPoint", "_decorator_names"), asdh_bare(0, value).
ASDNode(@place("", 0), n, i) :- option("entryPoint", "_strip_method_docstring"), asdh_statement(0, n, i).
ASDRequirement(@place("", 0), base, lazy, counterpart, decorator) :- option("entryPoint", "_legacy_wrapper_requirements"), asdh_requirement(0, base, lazy, counterpart, decorator).
.decl asds_candidate(i: number, path: symbol, count: number)
asds_candidate(i, path, to_number(count)) :- option("entryPoint", "ViolationStats.to_dict"), option_item("offenderPaths", i, path), option_item("offenderCounts", i, count), !option("hasRoot", "true").
asds_candidate(i, path, to_number(count)) :- option("entryPoint", "ViolationStats.to_dict"), option_item("offenderPaths", i, path), option_item("offenderCounts", i, count), option("hasRoot", "true"), option("root", root), !prefix(root, path).
asds_candidate(i, @replace_all(@slice(path, strlen(root), strlen(path)), "^/+", ""), to_number(count)) :- option("entryPoint", "ViolationStats.to_dict"), option_item("offenderPaths", i, path), option_item("offenderCounts", i, count), option("hasRoot", "true"), option("root", root), prefix(root, path).
.decl asds_offender(path: symbol, count: number, first: number)
asds_offender(path, count, first) :- asds_candidate(_, path, _), last = max i : { asds_candidate(i, path, _) }, first = min i : { asds_candidate(i, path, _) }, asds_candidate(last, path, count).
.decl asds_before(path: symbol, other: symbol)
asds_before(path, other) :- asds_offender(path, count, _), asds_offender(other, larger, _), larger > count.
asds_before(path, other) :- asds_offender(path, count, first), asds_offender(other, count, before), before < first.
.violation ASDOffender(at: span, path: symbol, count: number, ordinal: number) code "ASD_OFFENDER" "offender"
ASDOffender(@place("", 0), path, count, ordinal) :- asds_offender(path, count, _), ordinal = count : { asds_before(path, _) }, ordinal < 15.
.violation ASDStats(at: span, a: number, b: number, c: number, total: number, files: number, a_description: symbol, b_description: symbol, c_description: symbol) code "ASD_STATS" "statistics"
ASDStats(@place("", 0), to_number(a), to_number(b), to_number(c), to_number(total), files, "Direct delegation to sync counterpart without await", "Async function with no await expressions", "Architectural wrapper or escape-hatch violations") :- option("entryPoint", "ViolationStats.to_dict"), option("categoryA", a), option("categoryB", b), option("categoryC", c), option("total", total), files = count : { option_item("files", _, _) }.
'''


def analysis():
    return r'''
.violation ASDParse(at: span, error: symbol) code "ASD_PARSE" "{error}"
ASDParse(@place(path, 1), cat("SyntaxError: ", message, " (", name, ", line ", to_string(l), ")")) :- option("entryPoint", "analyze_source"), doc(d, _, _), syntax_error(d, l, message), option("originalPath", path), option("originalName", name).
.violation ASDAnalysis(at: span, function_name: symbol, category: symbol, description: symbol, sync: symbol, details: symbol, has_sync: number, has_details: number, phase: number, ordinal: number) code "ASD_ANALYSIS" "{description}"
ASDAnalysis(@place(path, 1), "", "", "", "", "", 0, 0, 0, 0) :- option("originalPath", path), 1 = 0.
'''
