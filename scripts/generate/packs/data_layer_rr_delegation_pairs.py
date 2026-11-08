"""Public pairs and the complete eager/lazy wrapper shapes in the pinned checker."""


def pair_relations():
    return r'''
// CPython's positional argument list includes starred arguments and excludes
// keywords, even when a starred argument follows a keyword in source order.
.decl asdw_pos_item(c: node, raw: node, order: number)
asdw_pos_item(c, raw, order) :- kind(c, "call"), field(c, "arguments", args), child(args, raw, order), !kind(raw, "comment"), !kind(raw, "keyword_argument"), !kind(raw, "dictionary_splat").
.decl asdw_arg(c: node, index: number, value: node)
asdw_arg(c, index, value) :- asdw_pos_item(c, raw, order), unparen(raw, value), index = count : { asdw_pos_item(c, _, j), j < order }.
.decl asdw_keyword(c: node, name: symbol, value: node, order: number)
asdw_keyword(c, @nfkc(name), value, order) :- kind(c, "call"), field(c, "arguments", args), child(args, kw, order), kind(kw, "keyword_argument"), field(kw, "name", nm), text(nm, name), field(kw, "value", raw), unparen(raw, value).
.decl asdw_expected(f: node, helper: symbol, boundary: symbol, terminal: symbol)
asdw_expected(f, "_resolve_legacy_result_sync", "run_in_legacy_execution_boundary", "resolve") :- asdp_function(f, _, _), !is_async(f).
asdw_expected(f, "_resolve_legacy_result_async", "run_in_legacy_execution_boundary_async", "resolve_async") :- asdp_function(f, _, _), is_async(f).
.decl asdw_call(f: node, value: node, call: node)
asdw_call(f, call, call) :- asdw_input(f, call), !is_async(f), kind(call, "call").
asdw_call(f, value, call) :- asdw_input(f, value), is_async(f), kind(value, "await"), child(value, raw, 0), unparen(raw, call), kind(call, "call").
.decl asdw_lazy_call(call: node, family: symbol)
asdw_lazy_call(call, @slice(name, 0, strlen(name)-5)) :- kind(call, "call"), field(call, "function", raw), unparen(raw, fn), asdp_callable(fn, name), suffix("_lazy", name).
.decl asdw_build_lambda(call: node, value: node)
asdw_build_lambda(call, value) :- asdw_arg(call, 2, value).
asdw_build_lambda(call, value) :- kind(call, "call"), !asdw_arg(call, 2, _), asdw_keyword(call, "build_lazy", value, i), i = min j : { asdw_keyword(call, "build_lazy", _, j) }.
.decl asdw_resolve(f: node, value: node, family: symbol)
asdw_resolve(f, value, family) :- asdw_call(f, value, call), asdw_expected(f, helper, _, _), field(call, "function", raw), unparen(raw, fn), asdp_callable(fn, helper), asdw_build_lambda(call, lam), kind(lam, "lambda"), field(lam, "body", body), unparen(body, lazy), asdw_lazy_call(lazy, family).
asdw_resolve(f, value, family) :- asdw_call(f, value, call), asdw_expected(f, _, boundary, terminal), field(call, "function", rawfn), unparen(rawfn, fn), asdp_callable(fn, boundary), field(call, "arguments", args), count : { child(args, arg, _), !kind(arg, "comment") } = 1, asdw_arg(call, 0, arg), kind(arg, "attribute"), field(arg, "attribute", nm), text(nm, name), @nfkc(name) = terminal, field(arg, "object", rawlazy), unparen(rawlazy, lazy), asdw_lazy_call(lazy, family).
asdw_resolve(f, value, family) :- asdw_call(f, value, call), asdw_expected(f, _, _, terminal), field(call, "function", rawfn), unparen(rawfn, fn), kind(fn, "attribute"), field(fn, "attribute", nm), text(nm, name), @nfkc(name) = terminal, field(fn, "object", rawlazy), unparen(rawlazy, lazy), asdw_lazy_call(lazy, family).
.decl asdw_value(stmt: node, value: node)
asdw_value(stmt, value) :- kind(stmt, k), (k = "return_statement" ; k = "expression_statement"), child(stmt, raw, 0), unparen(raw, value).
.decl asdw_order(f: node, stmt: node, index: number)
asdw_order(f, stmt, index) :- asdp_statement(f, stmt, i), index = count : { asdp_statement(f, _, j), j < i }.
.decl asdw_length(f: node, count: number)
asdw_length(f, count) :- asdp_function(f, _, _), count = count : { asdp_statement(f, _, _) }.
.decl asdw_assigned(f: node, family: symbol)
asdw_assigned(f, family) :- asdw_length(f, 2), asdw_order(f, first, 0), asgstmt(a, first), !field(a, "type", _), count : { assign_target(a, _) } = 1, assign_target(a, rawtarget), unparen(rawtarget, target), kind(target, "identifier"), text(target, name), assign_value(a, rawlazy), unparen(rawlazy, lazy), asdw_lazy_call(lazy, family), asdw_order(f, last, 1), kind(last, "return_statement"), asdw_value(last, value), asdw_call(f, value, call), asdw_expected(f, _, boundary, terminal), field(call, "function", rawfn), unparen(rawfn, fn), asdp_callable(fn, boundary), field(call, "arguments", args), count : { child(args, item, _), !kind(item, "comment") } = 1, asdw_arg(call, 0, attr), kind(attr, "attribute"), field(attr, "attribute", nm), text(nm, actual), @nfkc(actual) = terminal, field(attr, "object", rawobj), unparen(rawobj, obj), kind(obj, "identifier"), text(obj, receiver), @nfkc(receiver) = @nfkc(name).
.decl asdw_post(stmt: node, family: symbol)
asdw_post(stmt, family) :- kind(stmt, "expression_statement"), child(stmt, raw, 0), unparen(raw, call), kind(call, "call"), field(call, "function", rawfn), unparen(rawfn, fn), asdp_callable(fn, name), asdp_normalized(name, family).
.decl asdw_bad_handler(t: node)
asdw_bad_handler(t) :- kind(t, "try_statement"), child(t, h, _), kind(h, "except_clause"), op(h, "*").
asdw_bad_handler(t) :- kind(t, "try_statement"), child(t, h, _), kind(h, "except_clause"), child(h, b, _), kind(b, "block"), !body_stmt(b, _, _).
asdw_bad_handler(t) :- kind(t, "try_statement"), child(t, h, _), kind(h, "except_clause"), child(h, b, _), kind(b, "block"), last = max i : { body_stmt(b, _, i) }, body_stmt(b, s, last), !kind(s, "raise_statement").
.decl asdw_try(t: node, statement: node)
asdw_try(t, statement) :- kind(t, "try_statement"), field(t, "body", b), body_len(b, 1), body_stmt(b, statement, 0), !asdw_bad_handler(t), count : { child(t, other, _), (kind(other, "else_clause") ; kind(other, "finally_clause")) } = 0.
.decl asdw_input(f: node, value: node)
asdw_input(f, value) :- asdp_statement(f, stmt, _), asdw_value(stmt, value).
asdw_input(f, value) :- asdp_single(f, t), asdw_try(t, stmt), asdw_value(stmt, value).
.decl asdw_signature(f: node, family: symbol, post: symbol, has_post: number)
asdw_signature(f, family, "", 0) :- asdp_single(f, stmt), asdw_value(stmt, value), asdw_resolve(f, value, family).
asdw_signature(f, family, "__exception_translation__", 1) :- asdp_single(f, t), asdw_try(t, stmt), asdw_value(stmt, value), asdw_resolve(f, value, family).
asdw_signature(f, family, "", 0) :- asdw_assigned(f, family).
asdw_signature(f, family, post, 1) :- asdw_length(f, 2), !asdw_assigned(f, _), asdw_order(f, stmt, 0), asdw_value(stmt, value), asdw_resolve(f, value, family), asdw_order(f, last, 1), asdw_post(last, post).
.decl asdw_candidate(c: node, sync: node, async: node, cls: symbol, sn: symbol, an: symbol, path: symbol, first: number)
asdw_candidate(c, sync, async, @nfkc(cls), sn, an, path, first) :- asdp_method(c, async, an, first), suffix("_async", an), sn = @slice(an, 0, strlen(an)-6), sn != "", !prefix("_", sn), asdp_method(c, sync, sn, _), def_name(c, cls), asdp_function(async, path, _), !asdp_test_path(path), !asdp_browser(path), !asdp_abstract(sync), !asdp_abstract(async), !asds_unsupported(sync), !asds_unsupported(async), !option_item("allowlistedTargets", _, cat(path, "::", @nfkc(cls), ".", an)).
.decl asdw_callback_pair(c: node, async: node)
asdw_callback_pair(c, async) :- asdw_candidate(c, sync, async, _, _, _, _, _), (asds_callback(sync) ; asds_callback(async)).
.decl asdw_shared_thin(c: node, async: node)
asdw_shared_thin(c, async) :- asdw_candidate(c, sync, async, _, _, _, _, _), asdp_thin(sync, family), asdp_thin(async, family).
.decl asdw_callback_legacy(c: node, async: node)
asdw_callback_legacy(c, async) :- asdw_candidate(c, sync, async, _, _, _, _, _), (asdp_decorator(sync, _, "legacy_sync") ; asdp_decorator(async, _, "legacy_async")).
.decl asdw_participates(c: node, async: node)
asdw_participates(c, async) :- asdw_candidate(c, _, async, _, sn, _, _, _), asdp_method(c, _, cat(sn, "_lazy"), _).
asdw_participates(c, async) :- asdw_candidate(c, sync, async, _, _, _, _, _), (asdw_signature(sync, _, _, _) ; asdw_signature(async, _, _, _)).
.decl asdw_triple(c: node, async: node)
asdw_triple(c, async) :- asdw_candidate(c, sync, async, _, sn, _, _, _), asdp_method(c, lazy, cat(sn, "_lazy"), _), asdp_decorator(sync, _, "legacy_sync"), asdp_decorator(async, _, "legacy_async"), asds_execution_lazy(lazy).
.decl asdw_shared_signature(c: node, async: node)
asdw_shared_signature(c, async) :- asdw_candidate(c, sync, async, _, _, _, _, _), asdw_signature(sync, family, post, has_post), asdw_signature(async, family, post, has_post).
.decl asdw_lazy_text(f: node, text: symbol)
asdw_lazy_text(f, @python_repr(family)) :- asdw_signature(f, family, _, _).
asdw_lazy_text(f, "None") :- asdp_function(f, _, _), !asdw_signature(f, _, _, _).
.decl asdw_class_order(c: node, d: number, end: number, depth: number)
asdw_class_order(c, d, end, depth) :- asdw_candidate(c, _, _, _, _, _, _, _), node(d, c, _, _, end), ast_depth(c, depth).
.decl asdw_rank(c: node, async: node, rank: number)
asdw_rank(c, async, rank) :- asdw_candidate(c, _, async, _, _, _, _, first), asdw_class_order(c, d, end, depth), rank = count : { asdw_candidate(other, _, _, _, _, _, _, i), asdw_class_order(other, d, oe, od), (oe < end ; (oe = end, od > depth) ; (other = c, i < first)) }.
ASDAnalysis(@place(path, line), cat(cls, ".", an), "C", cat("Public sync/async pair ", cls, ".", sn, "/", an, " does not delegate to one shared helper family"), "", cat("sync_helper=", sync_text, "; async_helper=", async_text, ". Move orchestration into a shared helper family and keep both public entrypoints thin."), 0, 1, 3, rank) :- asdw_candidate(c, sync, async, cls, sn, an, path, _), asdw_callback_pair(c, async), !asdw_callback_legacy(c, async), !asdw_shared_thin(c, async), asdp_family_text(sync, sync_text), asdp_family_text(async, async_text), line(async, line), asdw_rank(c, async, rank).
ASDAnalysis(@place(path, line), cat(cls, ".", an), "C", cat("Eager txn-bound pair ", cls, ".", sn, "/", an, " does not resolve through one shared lazy family"), "", cat("sync_lazy=", sync_text, "; async_lazy=", async_text, ". Use a shared *_lazy implementation and make the eager pair resolve it."), 0, 1, 3, rank) :- asdw_candidate(c, sync, async, cls, sn, an, path, _), !asdw_callback_pair(c, async), (asds_transaction(sync) ; asds_transaction(async)), (sn != "ensure_schema" ; !asdw_shared_thin(c, async)), asdw_participates(c, async), !asdw_triple(c, async), !asdw_shared_signature(c, async), asdw_lazy_text(sync, sync_text), asdw_lazy_text(async, async_text), line(async, line), asdw_rank(c, async, rank).
'''
