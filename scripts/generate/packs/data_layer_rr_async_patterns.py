"""Port async collector scope, traversal-order alias state, diagnostics, and fallback context."""
import json,re
from data_layer_rr_text import module
from data_layer_rr_async_graph import supplied_ast


def async_patterns(source,target,header):
    original=module(source,'async_patterns');q=json.dumps
    text=header('async_patterns',target)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude "**/.git/**", "**/.venv/**", "**/venv/**", "**/__pycache__/**", "**/tmp/**"',text,flags=re.M)
    text+='#include "python/print.rr"\n'
    for rel,values in [('ap_write',original.SYNC_WRITE_METHODS),('ap_read',original.SYNC_READ_METHODS),('ap_internal',original.SYNC_INTERNAL_IO_METHODS),('ap_connection',original.SYNC_CONNECTION_METHODS),('ap_metadata',original.SYNC_POLYMORPHIC_METADATA_METHODS),('ap_transaction',original.ALL_SYNC_TRANSACTION_METHODS)]:
        text+=f'.decl {rel}(name: symbol, async_name: symbol)\n'+''.join(f'{rel}({q(k)}, {q(v)}).\n' for k,v in sorted(values.items()))
    for rel,values in [('ap_param_name',original.TRANSACTION_PARAM_NAMES),('ap_param_type',original.TRANSACTION_TYPE_NAMES),('ap_query_method',original.QUERY_SOURCE_METHODS),('ap_exclude_pattern',original.EXCLUDE_PATTERNS),('ap_scan_root',original.SCAN_PATHS),('ap_trigger',original.SOURCE_TRIGGERS)]:
        text+=f'.decl {rel}(value: symbol)\n'+''.join(f'{rel}({q(v)}).\n' for v in sorted(values))
    text+=''.join(f'.decl ap_fixed_{code}(message: symbol)\nap_fixed_{code}({q(original.VIOLATIONS[code])}).\n' for code in ['ASYNC001','ASYNC002'])
    text+=r'''
.decl ap_native_enabled()
ap_native_enabled() :- !option("entryPoint", "analyze_source").
.decl ap_parent(n: node, p: node)
ap_parent(n, p) :- ap_native_enabled(), ast_parent(n, p).
.decl ap_frame_node(n: node)
ap_frame_node(n) :- ap_native_enabled(), kind(n, k), (k = "function_definition" ; k = "lambda" ; k = "module").
.decl ap_scope(n: node, f: node)
ap_scope(n, n) :- ap_frame_node(n).
ap_scope(n, f) :- ap_parent(n, p), ap_scope(p, f), !ap_frame_node(n).
.decl ap_outer_frame(f: node, p: node)
ap_outer_frame(f, p) :- ap_frame_node(f), ap_parent(f, n), ap_scope(n, p).
.decl ap_function(f: node, name: symbol, async_: number)
ap_function(f, @nfkc(name), 1) :- ap_native_enabled(), kind(f, "function_definition"), def_name(f, name), op(f, "async").
ap_function(f, @nfkc(name), 0) :- ap_native_enabled(), kind(f, "function_definition"), def_name(f, name), !op(f, "async").
.decl ap_sync_frame(f: node)
ap_sync_frame(f) :- ap_function(f, _, 0).
ap_sync_frame(f) :- ap_native_enabled(), kind(f, "lambda").
.decl ap_root_frame(f: node)
ap_root_frame(f) :- ap_native_enabled(), kind(f, "module").
.decl ap_class(n: node, name: symbol)
ap_class(n, @nfkc(name)) :- ap_native_enabled(), kind(n, "class_definition"), def_name(n, name).
.decl ap_class_base(c: node, b: node)
ap_class_base(c, b) :- ap_native_enabled(), ap_class(c, _), field(c, "superclasses", args), child(args, raw, _), unparen(raw, b).
.decl ap_in_class(n: node, c: node)
ap_in_class(n, p) :- ap_parent(n, p), ap_class(p, _).
ap_in_class(n, c) :- ap_parent(n, p), !ap_class(p, _), ap_in_class(p, c).
.decl ap_decorator(f: node, e: node)
ap_decorator(f, e) :- ap_native_enabled(), decorator(f, e).
.decl ap_name(n: node, name: symbol)
ap_name(n, @nfkc(name)) :- ap_native_enabled(), unparen(n, e), ast_name(e, name).
.decl ap_attr(n: node, obj: node, name: symbol)
ap_attr(n, obj, @nfkc(name)) :- ap_native_enabled(), unparen(n, e), kind(e, "attribute"), field(e, "object", obj), field(e, "attribute", nm), text(nm, name).
.decl ap_call(n: node, fn: node)
ap_call(n, fn) :- ap_native_enabled(), kind(n, "call"), field(n, "function", fn).
.decl ap_subscript_value(n: node, obj: node)
ap_subscript_value(n, obj) :- ap_native_enabled(), unparen(n, e), kind(e, "subscript"), field(e, "value", obj).
.decl ap_parameter(f: node, name: symbol)
ap_parameter(f, @nfkc(name)) :- ap_native_enabled(), sig_param(f, _, nm, _, _, _, _), text(nm, name).
.decl ap_parameter_annotation(f: node, ann: node)
ap_parameter_annotation(f, ann) :- ap_native_enabled(), sig_param(f, _, _, _, _, ann, _), !unset(ann).
.decl ap_typed_parameter(f: node, name: symbol, ann: node)
ap_typed_parameter(f, @nfkc(name), ann) :- ap_native_enabled(), sig_param(f, _, nm, _, _, ann, _), !unset(ann), text(nm, name).
.decl ap_annotation_edge(e: node, c: node)
ap_annotation_edge(e, c) :- ap_native_enabled(), kind(e, k), (k = "type" ; k = "parenthesized_expression"), child(e, c, _), !kind(c, "comment").
ap_annotation_edge(e, c) :- ap_native_enabled(), kind(e, "generic_type"), child(e, c, 0).
ap_annotation_edge(e, c) :- ap_native_enabled(), kind(e, "generic_type"), child(e, tp, 1), child(tp, c, _).
ap_annotation_edge(e, c) :- ap_native_enabled(), kind(e, "subscript"), (field(e, "value", c) ; field(e, "subscript", c)).
ap_annotation_edge(e, c) :- ap_native_enabled(), kind(e, "tuple"), child(e, c, _).
ap_annotation_edge(e, c) :- ap_native_enabled(), kind(e, "binary_operator"), op(e, "|"), (field(e, "left", c) ; field(e, "right", c)).
ap_annotation_edge(e, c) :- ap_native_enabled(), kind(e, "union_type"), child(e, c, _).
.decl ap_annotation_string(e: node, text: symbol)
ap_annotation_string(e, text) :- ap_native_enabled(), plain_str(e, text).
.decl ap_assignment(a: node, target: symbol, value: node)
ap_assignment(a, target, value) :- ap_native_enabled(), assign_target(a, t), ap_name(t, target), assign_value(a, value).
.decl ap_order(n: node, ordinal: number)
ap_order(n, i) :- ap_native_enabled(), visit_index(n, i).
.decl ap_call_line(n: node, line: number)
ap_call_line(n, l) :- ap_native_enabled(), ap_call(n, _), line(n, l).
.decl ap_receiver_print(c: node, printed: symbol)
ap_receiver_print(c, printed) :- ap_native_enabled(), call_receiver_text(c, printed).

.decl ap_document(d: number, path: symbol)
ap_document(d, p) :- doc(d, p, _), option("entryPoint", "lint_file"), file_name(d, n), suffix(".py", n), !syntax_error(d, _, _).
ap_document(d, p) :- doc(d, p, _), !option("entryPoint", _), ap_scan_root(root), prefix(cat(root, "/"), p), file_name(d, n), suffix(".py", n), !ap_hidden(d), !syntax_error(d, _, _).
.decl ap_hidden(d: number)
ap_hidden(d) :- path_part(d, p), (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp").
.decl ap_source_triggered(d: number)
ap_source_triggered(d) :- source(d, sp), text(sp, text), ap_trigger(trigger), contains(trigger, text).
.decl ap_path(d: number, path: symbol)
ap_path(d, path) :- ap_document(d, _), option("originalPath", path).
ap_path(d, path) :- ap_document(d, path), !option("originalPath", _).
.decl ap_excluded_path(path: symbol)
ap_excluded_path(path) :- ap_path(_, path), ap_exclude_pattern(pattern), contains(pattern, path).
ap_excluded_path(path) :- option("originalPath", path), ap_exclude_pattern(pattern), contains(pattern, path).
.decl ap_in_scope(d: number)
ap_in_scope(d) :- ap_path(d, p), !ap_excluded_path(p),
    (contains("impl/", p) ; contains("impl\\", p) ; contains("api/", p) ; contains("api\\", p) ; contains("conform/", p) ; contains("conform\\", p)).
.decl ap_api(d: number)
ap_api(d) :- ap_path(d, p), (contains("api/", p) ; contains("api\\", p)).
.decl ap_impl(d: number)
ap_impl(d) :- ap_path(d, p), (contains("impl/", p) ; contains("impl\\", p)).
.decl ap_line(d: number, line: number, text: symbol)
ap_line(d, l, text) :- ap_native_enabled(), ap_document(d, _), split_line(d, l, text).
.decl ap_call_doc(c: node, d: number, path: symbol)
ap_call_doc(c, d, path) :- ap_call(c, _), ap_native_enabled(), docof(c, d), ap_document(d, path), ap_in_scope(d), ap_source_triggered(d).
// The supplied tree and original source are independent inputs. Its printer buffer is never a detector input.
ap_document(0, path) :- option("entryPoint", "analyze_source"), option("analysisPath", path), option("providedTree", "true").
ap_line(0, i + 1, text) :- option("entryPoint", "analyze_source"), option_item("sourceLines", i, text).
ap_source_triggered(0) :- option("entryPoint", "analyze_source"), option("providedSource", text), ap_trigger(trigger), contains(trigger, text).
ap_call_doc(c, 0, path) :- option("entryPoint", "analyze_source"), ap_call(c, _), ap_document(0, path), ap_in_scope(0), ap_source_triggered(0).

.decl ap_write_decorator(f: node)
ap_write_decorator(f) :- ap_decorator(f, e), ap_call(e, fn), ap_attr(fn, _, "write_async").
.decl ap_read_decorator(f: node)
ap_read_decorator(f) :- ap_decorator(f, e), ap_call(e, fn), ap_attr(fn, _, "read_async").
.decl ap_flags(f: node, write_: number, read_: number)
ap_flags(f, 0, 0) :- ap_root_frame(f).
ap_flags(f, 0, 0) :- ap_sync_frame(f).
ap_flags(f, w, r) :- ap_function(f, _, 1), (ap_write_decorator(f) ; ap_read_decorator(f)),
    w = count : { ap_write_decorator(f) }, r = count : { ap_read_decorator(f) }.
ap_flags(f, w, r) :- ap_function(f, _, 1), !ap_write_decorator(f), !ap_read_decorator(f), ap_outer_frame(f, p), ap_flags(p, w, r).
ap_flags(f, 0, 0) :- ap_function(f, _, 1), !ap_write_decorator(f), !ap_read_decorator(f), !ap_outer_frame(f, _).
.decl ap_async_call(c: node, f: node, name: symbol)
ap_async_call(c, f, name) :- ap_call(c, _), ap_scope(c, f), ap_function(f, name, 1).
.decl ap_annotation_contains(e: node)
ap_annotation_contains(e) :- ap_name(e, name), ap_param_type(name).
ap_annotation_contains(e) :- ap_attr(e, _, name), ap_param_type(name).
ap_annotation_contains(e) :- ap_annotation_string(e, text), ap_param_type(name), contains(name, text).
ap_annotation_contains(e) :- ap_annotation_edge(e, c), ap_annotation_contains(c).
.decl ap_txn_name(f: node, name: symbol)
ap_txn_name(f, name) :- ap_function(f, _, 1), ap_parameter(f, name), ap_param_name(name).
ap_txn_name(f, name) :- ap_function(f, _, 1), ap_typed_parameter(f, name, ann), ap_annotation_contains(ann).
.decl ap_receiver_root(e: node, name: symbol)
ap_receiver_root(e, name) :- ap_name(e, name).
ap_receiver_root(e, name) :- ap_attr(e, obj, _), ap_receiver_root(obj, name).
ap_receiver_root(e, name) :- ap_call(e, fn), ap_receiver_root(fn, name).
ap_receiver_root(e, name) :- ap_subscript_value(e, obj), ap_receiver_root(obj, name).
.decl ap_query_chain(f: node, e: node)
ap_query_chain(f, e) :- ap_function(f, _, 1), ap_call(e, fn), ap_attr(fn, obj, method), ap_query_method(method), ap_receiver_root(obj, root), ap_txn_name(f, root).
ap_query_chain(f, e) :- ap_call(e, fn), ap_attr(fn, obj, _), ap_query_chain(f, obj).
.decl ap_alias(f: node, name: symbol, kind: symbol, at: number)
ap_alias(f, name, "session", i) :- ap_assignment(a, name, v), ap_scope(a, f), ap_function(f, _, 1), ap_order(a, i), ap_call(v, fn), ap_attr(fn, _, "session").
ap_alias(f, name, "query", i) :- ap_assignment(a, name, v), ap_scope(a, f), ap_function(f, _, 1), ap_order(a, i), ap_query_chain(f, v).
ap_alias(f, name, kind, i) :- ap_assignment(a, name, v), ap_scope(a, f), ap_function(f, _, 1), ap_order(a, i), ap_name(v, other), ap_alias(f, other, kind, j), j <= i.
.decl ap_tracked_query(c: node, obj: node)
ap_tracked_query(c, obj) :- ap_async_call(c, f, _), ap_receiver_root(obj, name), ap_alias(f, name, "query", at), ap_order(c, i), at <= i.
ap_tracked_query(c, obj) :- ap_async_call(c, f, _), ap_query_chain(f, obj).
.decl ap_transaction_receiver(c: node, obj: node)
ap_transaction_receiver(c, obj) :- ap_async_call(c, _, _), ap_name(obj, "self").
ap_transaction_receiver(c, obj) :- ap_async_call(c, f, _), ap_receiver_root(obj, name), ap_txn_name(f, name).
ap_transaction_receiver(c, obj) :- ap_tracked_query(c, obj).
.decl ap_connection_receiver(c: node, obj: node)
ap_connection_receiver(c, obj) :- ap_async_call(c, _, _), ap_attr(obj, base, a), ap_name(base, "self"), (a = "_conn" ; a = "conn" ; a = "connection" ; a = "_connection").
ap_connection_receiver(c, obj) :- ap_async_call(c, f, _), ap_receiver_root(obj, name), ap_alias(f, name, "session", at), ap_order(c, i), at <= i.

.decl ap_abstract(c: node, d: number)
ap_abstract(c, d) :- ap_class_base(c, b), ap_name(b, "ABC"), docof(c, d), ap_native_enabled().
ap_abstract(c, d) :- ap_class_base(c, b), ap_attr(b, _, "ABC"), docof(c, d), ap_native_enabled().
ap_abstract(c, 0) :- ap_class_base(c, b), (ap_name(b, "ABC") ; ap_attr(b, _, "ABC")), option("entryPoint", "analyze_source").
ap_abstract(c, d) :- ap_class(c, name), ap_api(d),
    (contains("Transaction", name) ; contains("Connection", name) ; contains("Query", name) ; contains("Handle", name) ; contains("Version", name)).
.decl ap_window_query(radius: number, fragment: symbol)
ap_window_query(5, "ASYNC_FALLBACK:"). ap_window_query(5, "TODO: Implement").
ap_window_query(5, "native async"). ap_window_query(15, "_wrap_for_executor").
.decl ap_window_contains(c: node, radius: number, fragment: symbol)
ap_window_contains(c, radius, fragment) :- ap_window_query(radius, fragment), ap_call_doc(c, d, _), ap_call_line(c, line), ap_line(d, l, text), l >= line - radius, l <= line + radius, contains(fragment, text).
.decl ap_fallback(c: node)
ap_fallback(c) :- ap_call_doc(c, d, _), ap_api(d), ap_in_class(c, cls), ap_abstract(cls, d).
ap_fallback(c) :- ap_window_contains(c, 5, "ASYNC_FALLBACK:").
ap_fallback(c) :- ap_window_contains(c, 5, "TODO: Implement"), ap_window_contains(c, 5, "native async").
.decl ap_fallback_value(c: node, flag: number)
ap_fallback_value(c, 1) :- ap_call(c, _), ap_fallback(c).
ap_fallback_value(c, 0) :- ap_call(c, _), !ap_fallback(c).
.decl ap_executor_fallback(c: node, flag: number)
ap_executor_fallback(c, 1) :- ap_fallback(c).
ap_executor_fallback(c, 1) :- ap_call_doc(c, d, _), !ap_impl(d), ap_window_contains(c, 15, "_wrap_for_executor").
ap_executor_fallback(c, 0) :- ap_call_doc(c, d, _), !ap_fallback(c), (ap_impl(d) ; !ap_window_contains(c, 15, "_wrap_for_executor")).
.decl ap_context_line(d: number, l: number, text: symbol)
ap_context_line(d, l, cat("  ", @slice("    ", 0, 4 - strlen(to_string(l))), to_string(l), " | ", text)) :- ap_line(d, l, text), l < 1000.
ap_context_line(d, l, cat("  ", to_string(l), " | ", text)) :- ap_line(d, l, text), l >= 1000.
.decl ap_context(c: node, context: symbol)
ap_context(c, context) :- ap_call_doc(c, d, _), ap_call_line(c, line),
    context = join text by l sep "\n" : { ap_context_line(d, l, text), l >= line - 2, l <= line + 2 }.
.decl ap_issue(c: node, code: symbol, message: symbol, fallback: number, rank: number)
ap_issue(c, "ASYNC001", message, flag, 1) :- ap_call_doc(c, _, _), ap_call(c, fn), ap_attr(fn, _, "to_thread"), ap_fixed_ASYNC001(message), ap_fallback_value(c, flag).
ap_issue(c, "ASYNC002", message, flag, 2) :- ap_call_doc(c, _, _), ap_call(c, fn), ap_attr(fn, _, "run_in_executor"), ap_fixed_ASYNC002(message), ap_executor_fallback(c, flag).
ap_issue(c, "ASYNC004", cat("Use ", async_method, "() instead of ", method, "() in async transaction"), 0, 4) :- ap_call_doc(c, _, _), ap_call(c, fn), ap_attr(fn, _, method), ap_scope(c, f), ap_flags(f, 1, _), ap_write(method, async_method).
ap_issue(c, "ASYNC004", cat("Use ", async_method, "() instead of ", method, "() in async transaction"), 0, 4) :- ap_call_doc(c, _, _), ap_call(c, fn), ap_attr(fn, _, method), ap_scope(c, f), ap_flags(f, w, r), (w = 1 ; r = 1), ap_read(method, async_method).
ap_issue(c, "ASYNC005", cat("Sync I/O '", method, "()' in async function '", name, "' - use ", async_method, "()"), flag, 5) :- ap_call_doc(c, _, _), ap_async_call(c, _, name), ap_call(c, fn), ap_attr(fn, _, method), ap_internal(method, async_method), ap_fallback_value(c, flag).
ap_issue(c, "ASYNC006", cat("Sync method '", printed, ".", method, "()' in async function - use ", async_method, "()"), flag, 6) :- ap_call_doc(c, _, _), ap_async_call(c, f, _), ap_flags(f, 0, 0), ap_call(c, fn), ap_attr(fn, obj, method), ap_transaction(method, async_method), ap_transaction_receiver(c, obj), ap_receiver_print(c, printed), ap_fallback_value(c, flag).
ap_issue(c, "ASYNC007", cat("Sync '", printed, ".", method, "()' in async function - use await ", printed, ".", async_method, "()"), 0, 7) :- ap_call_doc(c, _, _), ap_async_call(c, _, _), ap_call(c, fn), ap_attr(fn, obj, method), ap_connection(method, async_method), ap_connection_receiver(c, obj), ap_receiver_print(c, printed).
ap_issue(c, "ASYNC008", cat("Sync polymorphic metadata helper '", printed, ".", method, "()' in async function - use await ", printed, ".", async_method, "()"), 0, 8) :- ap_call_doc(c, _, _), ap_async_call(c, _, _), ap_call(c, fn), ap_attr(fn, _, method), ap_metadata(method, async_method), ap_receiver_print(c, printed).
.decl ap_visible(c: node, code: symbol, message: symbol, fallback: number, rank: number)
ap_visible(c, code, message, fallback, rank) :- ap_issue(c, code, message, fallback, rank), option("entryPoint", e), (e = "lint_file" ; e = "analyze_source").
ap_visible(c, code, message, 0, rank) :- ap_issue(c, code, message, 0, rank), !option("entryPoint", _).
'''
    for code in ['ASYNC001','ASYNC002','ASYNC004','ASYNC005','ASYNC006','ASYNC007','ASYNC008']:
        text+=f'.violation Async{code}(at: span, context: symbol, fallback: number, ordinal: number, rank: number) code {q(code)} "{{message}}" parameters(message: symbol)\n'
        # Message parameters belong in the finding's typed head, as the modern reader requires.
        text=text.replace(f'.violation Async{code}(at: span, context: symbol, fallback: number, ordinal: number, rank: number) code {q(code)} "{{message}}" parameters(message: symbol)',f'.violation Async{code}(at: span, message: symbol, context: symbol, fallback: number, ordinal: number, rank: number) code {q(code)} "{{message}}"')
        text+=f'Async{code}(@place(path, line), message, context, fallback, ordinal, rank) :- ap_visible(c, {q(code)}, message, fallback, rank), ap_call_doc(c, _, path), ap_call_line(c, line), ap_context(c, context), ap_order(c, ordinal).\n'
    text+=r'''
 .violation Excluded(at: span) code "AP_EXCLUDED" "excluded"
Excluded(@place("input.py", 1)) :- option("entryPoint", "should_exclude"), option("originalPath", path), ap_excluded_path(path).
.violation ReadFailure(at: span) code "AP_READ_ERROR" "Source read failed"
ReadFailure(@place("input.py", 1)) :- option("entryPoint", "lint_file"), option("originalPath", path), !ap_excluded_path(path), option("ioError", "true").
.violation ParseWarning(at: span) code "AP_PARSE_WARNING" "Source decoding failed"
ParseWarning(@place("input.py", 1)) :- option("entryPoint", "lint_file"), option("originalPath", path), !ap_excluded_path(path), option("decodeError", "true").
.violation MissingTree(at: span) code "AP_AST_REQUIRED" "AST tree is required; parsing is owned by the fact framework"
MissingTree(@place("input.py", 1)) :- option("entryPoint", "analyze_source"), option("providedTree", "false"), ap_source_triggered(0).
'''
    graph=supplied_ast()
    graph=graph.replace('ap_scope(n, n) :- ap_frame_node(n), ap_graph_kind(n, _).\nap_scope(n, f) :- ap_graph_edge(p, _, n), ap_scope(p, f), !ap_frame_node(n).\nap_outer_frame(f, p) :- ap_graph_callable(f), ap_graph_edge(parent, _, f), ap_scope(parent, p).\n','')
    graph=graph.replace('ap_in_class(n, c) :- ap_graph_edge(p, _, n), ap_class(p, _), c = p.\nap_in_class(n, c) :- ap_graph_edge(p, _, n), !ap_class(p, _), ap_in_class(p, c).\n','')
    graph+='ap_parent(n, p) :- ap_graph_edge(p, _, n).\n'
    return text+graph
