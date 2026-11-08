"""No-await and sync-delegation clauses, including the source's structural exemptions."""


def async_relations():
    return r'''
.decl asda_block_stmt(b: node, s: node, i: number)
asda_block_stmt(b, s, i) :- body_stmt(b, s, i), i > 0.
asda_block_stmt(b, s, 0) :- body_stmt(b, s, 0), !doc_stmt(s, _).
.decl asda_raise(s: node)
asda_raise(s) :- kind(s, "raise_statement"), child(s, raw, 0), unparen(raw, e), kind(e, "identifier"), asdp_callable(e, name), (name = "RuntimeError" ; name = "NotImplementedError").
asda_raise(s) :- kind(s, "raise_statement"), child(s, raw, 0), unparen(raw, e), kind(e, "call"), field(e, "function", rawfn), unparen(rawfn, fn), asdp_callable(fn, name), (name = "RuntimeError" ; name = "NotImplementedError").
.decl asda_setup_leaf(s: node)
asda_setup_leaf(s) :- kind(s, k), (k = "pass_statement" ; k = "delete_statement").
asda_setup_leaf(s) :- asgstmt(_, s).
asda_setup_leaf(s) :- kind(s, "expression_statement"), child(s, raw, 0), unparen(raw, e), kind(e, "ellipsis").
.decl asda_if_child(s: node, c: node)
asda_if_child(s, c) :- py_if(s, _, body), asda_block_stmt(body, c, _).
asda_if_child(s, c) :- py_orelse(s, c), kind(c, "elif_clause").
asda_if_child(s, c) :- py_orelse(s, other), kind(other, "else_clause"), field(other, "body", body), asda_block_stmt(body, c, _).
.decl asda_setup_part(s: node, c: node)
asda_setup_part(s, s) :- body_stmt(_, s, _).
asda_setup_part(s, c) :- asda_setup_part(s, p), asda_if_child(p, c).
.decl asda_setup_bad(s: node)
asda_setup_bad(s) :- asda_setup_part(s, c), !py_if(c, _, _), !asda_setup_leaf(c), !asda_raise(c).
asda_setup_bad(s) :- asda_setup_part(s, c), py_if(c, _, body), !asda_block_stmt(body, _, _).
.decl asda_setup(s: node)
asda_setup(s) :- asda_setup_part(s, s), !asda_setup_bad(s), !asda_raise(s).
.decl asda_stub(f: node)
asda_stub(f) :- asdp_function(f, _, _), !asdp_statement(f, _, _).
asda_stub(f) :- asdp_function(f, _, _), last = max i : { asdp_statement(f, _, i) }, asdp_statement(f, s, last), asda_raise(s), count : { asdp_statement(f, prev, i), i < last, !asda_setup(prev) } = 0.
.decl asda_local_edge(root: node, item: node)
asda_local_edge(root, item) :- (kind(root, "tuple") ; kind(root, "list") ; kind(root, "pattern_list") ; kind(root, "tuple_pattern") ; kind(root, "list_pattern")), child(root, raw, _), !kind(raw, "comment"), unparen(raw, item).
.decl asda_local_part(root: node, item: node)
asda_local_part(root, root) :- kind(root, _).
asda_local_part(root, item) :- asda_local_part(root, p), asda_local_edge(p, item).
.decl asda_self_attr(n: node)
asda_self_attr(n) :- kind(n, "attribute"), field(n, "object", raw), unparen(raw, receiver), kind(receiver, "identifier"), text(receiver, name), @nfkc(name) = "self".
.decl asda_local_leaf(n: node)
asda_local_leaf(n) :- kind(n, "identifier").
asda_local_leaf(n) :- asda_self_attr(n).
asda_local_leaf(n) :- kind(n, k), (k = "tuple" ; k = "list" ; k = "pattern_list" ; k = "tuple_pattern" ; k = "list_pattern").
.decl asda_local_bad(n: node)
asda_local_bad(n) :- asda_local_part(n, part), !asda_local_leaf(part).
.decl asda_static_edge(root: node, item: node)
asda_static_edge(root, item) :- kind(root, k), (k = "tuple" ; k = "list" ; k = "set" ; k = "expression_list"), child(root, raw, _), !kind(raw, "comment"), unparen(raw, item).
asda_static_edge(root, item) :- kind(root, "dictionary"), child(root, pair, _), kind(pair, "pair"), (field(pair, "key", raw) ; field(pair, "value", raw)), unparen(raw, item).
asda_static_edge(root, item) :- kind(root, "dictionary"), child(root, splat, _), kind(splat, "dictionary_splat"), child(splat, raw, 0), unparen(raw, item).
.decl asda_static_part(root: node, item: node)
asda_static_part(root, root) :- kind(root, _).
asda_static_part(root, item) :- asda_static_part(root, p), asda_static_edge(p, item).
.decl asda_static_leaf(n: node)
asda_static_leaf(n) :- ast_constant(n).
asda_static_leaf(n) :- kind(n, "identifier").
asda_static_leaf(n) :- asda_self_attr(n).
asda_static_leaf(n) :- kind(n, k), (k = "tuple" ; k = "list" ; k = "set" ; k = "dictionary" ; k = "expression_list").
.decl asda_static_bad(n: node)
asda_static_bad(n) :- asda_static_part(n, part), !asda_static_leaf(part).
.decl asda_lifecycle_stmt(s: node)
asda_lifecycle_stmt(s) :- kind(s, "pass_statement").
asda_lifecycle_stmt(s) :- asgstmt(a, s), count : { assign_target(a, raw), unparen(raw, target), asda_local_bad(target) } = 0.
asda_lifecycle_stmt(s) :- kind(s, "expression_statement"), child(s, a, 0), kind(a, "augmented_assignment"), field(a, "left", raw), unparen(raw, target), !asda_local_bad(target).
asda_lifecycle_stmt(s) :- kind(s, "return_statement"), !child(s, _, _).
asda_lifecycle_stmt(s) :- kind(s, "return_statement"), child(s, raw, 0), unparen(raw, value), !asda_static_bad(value).
.decl asda_lifecycle(f: node)
asda_lifecycle(f) :- asdp_function(f, _, name), (name = "start" ; name = "stop" ; name = "_on_last_unsubscribe"), count : { asdp_statement(f, s, _), !asda_lifecycle_stmt(s) } = 0.
// Descendants include nested definitions, their defaults and decorators. The
// current function's decorators precede its tree-sitter function range.
.decl asda_desc_call(f: node, call: node)
asda_desc_call(f, call) :- asdp_function(f, _, _), kind(call, "call"), within(call, f).
asda_desc_call(f, call) :- asdp_function(f, _, _), decorator(f, raw), kind(call, "call"), (call = raw ; within(call, raw)).
.decl asda_safe_call(call: node)
asda_safe_call(call) :- kind(call, "call"), field(call, "function", raw), unparen(raw, fn), asdp_callable(fn, name), (suffix("_lazy", name) ; name = "cast" ; name = "JSONResponse" ; name = "_attach_auto_api_hook_registry").
.decl asda_lazy_builder(f: node)
asda_lazy_builder(f) :- asda_desc_call(f, call), field(call, "function", raw), unparen(raw, fn), asdp_callable(fn, name), suffix("_lazy", name), count : { asda_desc_call(f, c), !asda_safe_call(c) } = 0.
.decl asda_await(f: node)
asda_await(f) :- kind(n, "await"), def_frame(n, f).
asda_await(f) :- kind(n, k), (k = "for_statement" ; k = "with_statement"), op(n, "async"), def_frame(n, f).
.decl asda_sync(f: node, name: symbol)
asda_sync(f, sync) :- asdp_function(f, _, name), is_async(f), suffix("_async", name), sync = @slice(name, 0, strlen(name)-6), kind(call, "call"), def_frame(call, f), field(call, "function", raw), unparen(raw, fn), ((kind(fn, "identifier"), text(fn, leaf)) ; (kind(fn, "attribute"), field(fn, "attribute", a), text(a, leaf))), @nfkc(leaf) = sync.
.decl asda_candidate(f: node, path: symbol, name: symbol)
asda_candidate(f, path, name) :- asdp_function(f, path, name), is_async(f), !asdp_ab_skip(path, name), !asda_await(f), !asdp_abstract(f), !asda_stub(f), !asda_lazy_builder(f), !asda_lifecycle(f), !asdp_decorator(f, _, "legacy_async"), !asdp_decorator(f, _, "legacy_module_async").
.decl asda_event(f: node, d: number, end: number, depth: number)
asda_event(f, d, end, depth) :- asda_candidate(f, _, _), node(d, f, _, _, end), ast_depth(f, depth).
.decl asda_rank(f: node, rank: number)
asda_rank(f, rank) :- asda_event(f, d, end, depth), rank = count : { asda_event(_, d, oe, od), (oe < end ; (oe = end, od > depth)) }.
ASDAnalysis(@place(path, line), name, "A", cat("Async function delegates to sync '", sync, "' without await"), sync, "", 1, 0, 0, rank) :- asda_candidate(f, path, name), asda_sync(f, sync), line(f, line), asda_rank(f, rank).
ASDAnalysis(@place(path, line), name, "B", "Async function has no await expressions", "", "", 0, 0, 0, rank) :- asda_candidate(f, path, name), !asda_sync(f, _), line(f, line), asda_rank(f, rank).
'''
