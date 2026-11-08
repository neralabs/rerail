"""Source relations for delegation checks on the current engine's Python tape."""


def source_relations():
    return r'''
#include "python/calls.rr"
.decl asdp_path(d: number, path: symbol)
asdp_path(d, original) :- doc(d, _, _), option("originalPath", original).
asdp_path(d, path) :- doc(d, path, _), !option("originalPath", _).
.decl asdp_function(f: node, path: symbol, name: symbol)
asdp_function(f, path, @nfkc(name)) :- kind(f, "function_definition"), docof(f, d), asdp_path(d, path), !syntax_error(d, _, _), def_name(f, name).
.decl asdp_test_path(path: symbol)
asdp_test_path(path) :- asdp_path(_, path), (contains("/tests/", path) ; contains("/conform/", path) ; prefix("tests/", path) ; prefix("conform/", path)).
.decl asdp_browser(path: symbol)
asdp_browser(path) :- asdp_path(_, path), (contains("/impl/browser/", path) ; prefix("impl/browser/", path)).
.decl asdp_candidate(path: symbol, name: symbol)
asdp_candidate(path, name) :- asdp_function(_, path, name).
.decl asdp_statement(f: node, s: node, i: number)
asdp_statement(f, s, i) :- body_of(f, b), body_stmt(b, s, i), i > 0.
asdp_statement(f, s, 0) :- body_of(f, b), body_stmt(b, s, 0), !doc_stmt(s, _).
.decl asdp_single(f: node, s: node)
asdp_single(f, s) :- asdp_function(f, _, _), asdp_statement(f, s, _), count : { asdp_statement(f, _, _) } = 1.
// Callable spelling follows _extract_callable_name, including the attribute tail
// after a non-name receiver, such as factory().helper.
.decl asdp_callable(e: node, name: symbol)
asdp_callable(e, @nfkc(name)) :- kind(e, "identifier"), text(e, name).
asdp_callable(e, cat(parent, ".", @nfkc(attr))) :- kind(e, "attribute"), field(e, "object", raw), unparen(raw, object), asdp_callable(object, parent), e < object, field(e, "attribute", a), text(a, attr).
asdp_callable(e, @nfkc(attr)) :- kind(e, "attribute"), field(e, "object", raw), unparen(raw, object), kind(object, k), k != "identifier", k != "attribute", field(e, "attribute", a), text(a, attr).
.decl asdp_normalized(name: symbol, family: symbol)
asdp_normalized(name, @slice(name, 0, strlen(name)-6)) :- asdp_callable(_, name), suffix("_async", name).
asdp_normalized(name, @slice(name, 0, strlen(name)-5)) :- asdp_callable(_, name), !suffix("_async", name), suffix("_sync", name).
asdp_normalized(name, name) :- asdp_callable(_, name), !suffix("_async", name), !suffix("_sync", name).
.decl asdp_dec_leaf(d: node, name: symbol)
asdp_dec_leaf(d, @nfkc(name)) :- kind(d, "identifier"), text(d, name).
asdp_dec_leaf(d, @nfkc(name)) :- kind(d, "attribute"), field(d, "attribute", a), text(a, name).
asdp_dec_leaf(d, name) :- kind(d, "call"), field(d, "function", raw), unparen(raw, fn), asdp_dec_leaf(fn, name).
.decl asdp_decorator(f: node, d: node, name: symbol)
asdp_decorator(f, d, name) :- decorator(f, raw), unparen(raw, d), asdp_dec_leaf(d, name).
.decl asdp_abstract(f: node)
asdp_abstract(f) :- asdp_decorator(f, d, "abstractmethod"), !kind(d, "call").
asdp_abstract(f) :- asdp_single(f, s), kind(s, "pass_statement").
asdp_abstract(f) :- asdp_single(f, s), kind(s, "expression_statement"), child(s, raw, 0), unparen(raw, v), kind(v, "ellipsis").
asdp_abstract(f) :- asdp_single(f, s), kind(s, "raise_statement"), child(s, raw, 0), unparen(raw, call), kind(call, "call"), field(call, "function", rawfn), unparen(rawfn, fn), kind(fn, "identifier"), text(fn, name), @nfkc(name) = "NotImplementedError".
.decl asdp_single_value(f: node, value: node)
asdp_single_value(f, value) :- asdp_single(f, s), (kind(s, "return_statement") ; kind(s, "expression_statement")), child(s, raw, 0), unparen(raw, value).
.decl asdp_single_call(f: node, call: node)
asdp_single_call(f, call) :- asdp_single_value(f, call), !is_async(f), kind(call, "call").
asdp_single_call(f, call) :- asdp_single_value(f, value), is_async(f), kind(value, "await"), child(value, raw, 0), unparen(raw, call), kind(call, "call").
.decl asdp_boundary_family(f: node, family: symbol)
asdp_boundary_family(f, family) :- asdp_single(f, stmt), kind(stmt, "return_statement"), asdp_single_call(f, call), field(call, "function", raw), unparen(raw, fn), asdp_callable(fn, boundary),
 ((is_async(f), boundary = "run_in_legacy_execution_boundary_async") ; (!is_async(f), boundary = "run_in_legacy_execution_boundary")),
 field(call, "arguments", args), count : { child(args, item, _), !kind(item, "comment") } = 1, call_arg(call, 0, "", arg), unparen(arg, lam), kind(lam, "lambda"), field(lam, "body", body), unparen(body, inner), kind(inner, "call"), field(inner, "function", rawhelper), unparen(rawhelper, helper), asdp_callable(helper, name), asdp_normalized(name, family).
.decl asdp_thin(f: node, family: symbol)
asdp_thin(f, family) :- asdp_boundary_family(f, family).
asdp_thin(f, family) :- asdp_single_call(f, call), !asdp_boundary_family(f, _), field(call, "function", raw), unparen(raw, fn), asdp_callable(fn, name), asdp_normalized(name, family).
// Class method dictionaries retain the last definition and the first insertion
// position of each name. Decorated definitions are one class-body statement.
.decl asdp_method_candidate(c: node, f: node, name: symbol, i: number)
asdp_method_candidate(c, f, name, i) :- kind(c, "class_definition"), field(c, "body", b), body_stmt(b, stmt, i), defstmt(f, stmt), asdp_function(f, _, name).
.decl asdp_method(c: node, f: node, name: symbol, first: number)
asdp_method(c, f, name, first) :- asdp_method_candidate(c, f, name, i), i = max j : { asdp_method_candidate(c, _, name, j) }, first = min j : { asdp_method_candidate(c, _, name, j) }.
.decl asdp_family_text(f: node, value: symbol)
asdp_family_text(f, @python_repr(family)) :- asdp_thin(f, family).
asdp_family_text(f, "None") :- asdp_function(f, _, _), !asdp_thin(f, _).
.decl asdp_context_order(c: node, d: number, end: number, depth: number)
asdp_context_order(c, d, end, depth) :- asdp_context_candidate(c, _, _, _, _, _, _, _), node(d, c, _, _, end), ast_depth(c, depth).
.decl asdp_exit_rank(c: node, rank: number)
asdp_exit_rank(c, rank) :- asdp_context_order(c, d, end, depth),
 rank = count : { asdp_context_order(_, d, oe, od), (oe < end ; (oe = end, od > depth)) }.
.decl asdp_context_pair(sync: symbol, async: symbol, rank: number)
asdp_context_pair("__enter__", "__aenter__", 0).
asdp_context_pair("__exit__", "__aexit__", 1).
.decl asdp_context_candidate(c: node, f: node, sync: node, cls: symbol, sn: symbol, an: symbol, path: symbol, rank: number)
asdp_context_candidate(c, f, sync, @nfkc(cls), sn, an, path, rank) :- asdp_context_pair(sn, an, rank), asdp_method(c, sync, sn, _), asdp_method(c, f, an, _), def_name(c, cls), asdp_function(f, path, _), !asdp_test_path(path), !asdp_browser(path), !option_item("allowlistedTargets", _, cat(path, "::", @nfkc(cls), ".", an)).
.decl asdp_context_shared(c: node, f: node)
asdp_context_shared(c, f) :- asdp_context_candidate(c, f, sync, _, _, _, _, _), asdp_thin(sync, family), asdp_thin(f, family).
ASDAnalysis(@place(path, line), cat(cls, ".", an), "C", cat("Context-manager pair ", cls, ".", sn, "/", an, " is not a thin wrapper over one shared helper family"), "", cat("sync_helper=", sync_text, "; async_helper=", async_text, ". Move wrapper logic into _..._sync/_..._async helpers and keep the dunder pair thin."), 0, 1, 2, exit*2+rank) :- asdp_context_candidate(c, f, sync, cls, sn, an, path, rank), !asdp_context_shared(c, f), asdp_family_text(sync, sync_text), asdp_family_text(f, async_text), line(f, line), asdp_exit_rank(c, exit).
ASDAnalysis(@place(path, line), name, "C", "Name mixes lazy with sync/async execution mode", "", cat(@python_repr(name), " mixes 'lazy' with 'sync' or 'async'. Lazy surfaces must stay execution-mode neutral."), 0, 1, 1, i) :- asdp_function(f, path, name), !asdp_test_path(path), normalized = @lower(name), contains("lazy", normalized), (contains("sync", normalized) ; contains("async", normalized)), line(f, line), visit_index(f, i).
'''
