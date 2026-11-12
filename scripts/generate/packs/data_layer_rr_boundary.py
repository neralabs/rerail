"""Port boundary collectors, raw scan counts, category projections, and reporting filters."""
import json,re
from data_layer_rr_text import module


def async_sync_boundary(source, target, header):
    original=module(source,'async_sync_boundary');q=json.dumps
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude "**/.git/**", "**/.venv/**", "**/venv/**", "**/__pycache__/**", "**/tmp/**"',header('async_sync_boundary', target),flags=re.M)
    for relation,values in [('bs_trigger',original.SOURCE_TRIGGERS),('bs_information_dir',original.INFORMATIONAL_DIRS)]:
        text+=f'.decl {relation}(text: symbol)\n'+''.join(f'{relation}({q(v)}).\n' for v in sorted(values))
    text+='.decl bs_static_allow(file: symbol, function: symbol)\n'+''.join(f'bs_static_allow({q(p)}, {q(f)}).\n' for p,f in sorted(original.ALLOWLIST))
    text+='.decl bs_label(category: symbol, text: symbol)\n'+''.join(f'bs_label({q(k)}, {q(v)}).\n' for k,v in sorted(original.CATEGORY_LABELS.items()))
    text += r'''
.violation Count(at: span, count: number) code "BS_COUNT" "{count}"
Count(@place("<inventory>", 1), n) :- option("entryPoint", "scan_tree"), n = count : { option_item("checkedPaths", _, _) }.
.violation Category(at: span, index: number) code "BS_CATEGORY" "category"
Category(@place("<category>", 1), i) :- option("entryPoint", "category"), option("category", c), option_item("categories", i, c).
.violation ReadFailure(at: span) code "BS_READ_ERROR" "Source read failed"
ReadFailure(@place("<inventory>", 1)) :- option("entryPoint", "scan_tree"), option("hasReadError", "true").

.decl bs_api_helper()
bs_api_helper() :- option("entryPoint", "_get_call_name").
.decl bs_call(n: node, fn: node)
bs_call(n, fn) :- kind(n, "call"), field(n, "function", fn).
.decl bs_callee(n: node)
bs_callee(n) :- !bs_api_helper(), bs_call(_, n).
.decl bs_attr(n: node, obj: node)
bs_attr(n, obj) :- !bs_api_helper(), unparen(n, e), kind(e, "attribute"), field(e, "object", obj).
// Collect the finite receiver chain, then join its identifiers from the inside out.
// A call, subscript, or literal receiver contributes no identifier.
.decl bs_part(callee: node, part: node)
bs_part(n, n) :- bs_callee(n).
bs_part(n, obj) :- bs_part(n, p), bs_attr(p, obj).
.decl bs_component(n: node, name: symbol, depth: number)
bs_component(n, @nfkc(name), depth) :- !bs_api_helper(), unparen(n, e), kind(e, "identifier"),
    text(e, name), ast_depth(n, depth).
bs_component(n, @nfkc(name), depth) :- !bs_api_helper(), unparen(n, e), kind(e, "attribute"),
    field(e, "attribute", a), text(a, name), ast_depth(n, depth).
.decl bs_name(n: node, name: symbol)
bs_name(n, name) :- bs_part(n, p), bs_component(p, _, _),
    name = join piece by -depth sep "." : { bs_part(n, part), bs_component(part, piece, depth) }.
.decl bs_context(c: node, name: symbol, async_: number)
bs_context(c, @nfkc(name), 1) :- !bs_api_helper(), bs_call(c, _), def_frame(c, f), def_name(f, name), op(f, "async").
bs_context(c, @nfkc(name), 0) :- !bs_api_helper(), bs_call(c, _), def_frame(c, f), def_name(f, name), !op(f, "async").
bs_context(c, "<module>", 0) :- !bs_api_helper(), bs_call(c, _), !def_frame(c, _).
.decl bs_document(d: number, path: symbol)
bs_document(d, p) :- doc(d, p, _), option("entryPoint", "scan_tree"), file_name(d, nm), suffix(".py", nm), !syntax_error(d, _, _).
bs_document(d, p) :- doc(d, p, _), !option("entryPoint", _), file_name(d, nm), suffix(".py", nm),
 (prefix("api/", p) ; prefix("impl/", p) ; prefix("conform/", p)), !bs_hidden(d), !syntax_error(d, _, _).
.decl bs_hidden(d: number)
bs_hidden(d) :- path_part(d, part), (part = ".git" ; part = ".venv" ; part = "venv" ; part = "__pycache__" ; part = "tmp").
.decl bs_source_enabled(d: number)
bs_source_enabled(d) :- bs_document(d, _), !option("hasReadError", "true"), source(d, span), text(span, source), bs_trigger(trigger), contains(trigger, source).
.decl bs_enabled(c:node)
bs_enabled(c) :- bs_call(c,_),docof(c,d),bs_source_enabled(d).
.decl bs_detect(c: node, category: symbol, name: symbol, async_: number, call_text: symbol)
bs_detect(c, "SYNC_WRAPS_ASYNC", fn, 0, cat(name, "()")) :- bs_call(c, f), bs_enabled(c), bs_name(f, name),
 (name = "asyncio.run" ; name = "asyncio.run_coroutine_threadsafe"), bs_context(c, fn, 0).
bs_detect(c, "ASYNC_WRAPS_SYNC", fn, 1, cat(name, "()")) :- bs_call(c, f), bs_enabled(c), bs_name(f, name),
 (name = "asyncio.to_thread" ; name = "to_thread"), bs_context(c, fn, 1).
bs_detect(c, "EXECUTOR_ESCAPE", fn, 1, cat(name, "()")) :- bs_call(c, f), bs_enabled(c), bs_name(f, name),
 suffix("run_in_executor", name), bs_context(c, fn, 1).

.decl bs_allowed_pairs(file: symbol, function: symbol)
bs_allowed_pairs(p, f) :- !option("entryPoint", "_is_allowlisted"), bs_static_allow(p, f).
bs_allowed_pairs(p, f) :- option("entryPoint", "_is_allowlisted"), option_item("allowedFiles", i, p), option_item("allowedFunctions", i, f).
.decl bs_info_pattern(pattern: symbol)
bs_info_pattern(p) :- !option("entryPoint", "_is_informational"), bs_information_dir(p).
bs_info_pattern(p) :- option("entryPoint", "_is_informational"), option_item("informationDirs", _, p).
.decl bs_target(path: symbol, function: symbol)
bs_target(p, f) :- bs_detect(c, _, f, _, _), docof(c, d), bs_document(d, p).
bs_target(p, f) :- option("entryPoint", "_is_allowlisted"), option("file_rel", p), option("function_name", f).
.decl bs_allowed(path: symbol, function: symbol)
bs_allowed(p, f) :- bs_target(p, f), bs_allowed_pairs(pattern, fun), (fun = "*" ; fun = f), !suffix("*", pattern), suffix(pattern, p).
bs_allowed(p, f) :- bs_target(p, f), bs_allowed_pairs(pattern, fun), (fun = "*" ; fun = f), suffix("*", pattern),
 prefix_ = @slice(pattern, 0, strlen(pattern) - 1), trimmed = @replace_all(prefix_, "/+$", ""),
 (suffix(trimmed, p) ; contains(cat(trimmed, "/"), p) ; suffix(prefix_, p) ; contains(prefix_, p)).
.decl bs_info_path(path: symbol)
bs_info_path(p) :- bs_document(_, p).
bs_info_path(p) :- option("entryPoint", "_is_informational"), option("file_rel", p).
.decl bs_informational(path: symbol)
bs_informational(p) :- bs_info_path(p), bs_info_pattern(part), contains(part, p).
.decl bs_emit(c: node, category: symbol, function_name: symbol, async_: number, call_text: symbol)
bs_emit(c, cat, fn, a, call_text) :- bs_detect(c, cat, fn, a, call_text), option("entryPoint", "scan_tree").
bs_emit(c, cat, fn, a, call_text) :- bs_detect(c, cat, fn, a, call_text), !option("entryPoint", _),
 docof(c, d), bs_document(d, p), !bs_allowed(p, fn), !bs_informational(p).

.violation Sync(at: node, function_name: symbol, async_: number, call_text: symbol, column: number, ordinal: number, message: symbol) code "SYNC_WRAPS_ASYNC" "{message}"
Sync(c, fn, a, t, col, ordinal, cat(label, ": ", fn, ": ", t)) :- bs_emit(c, "SYNC_WRAPS_ASYNC", fn, a, t), col(c, col), visit_index(c, ordinal), bs_label("SYNC_WRAPS_ASYNC", label).
.violation Async(at: node, function_name: symbol, async_: number, call_text: symbol, column: number, ordinal: number, message: symbol) code "ASYNC_WRAPS_SYNC" "{message}"
Async(c, fn, a, t, col, ordinal, cat(label, ": ", fn, ": ", t)) :- bs_emit(c, "ASYNC_WRAPS_SYNC", fn, a, t), col(c, col), visit_index(c, ordinal), bs_label("ASYNC_WRAPS_SYNC", label).
.violation Executor(at: node, function_name: symbol, async_: number, call_text: symbol, column: number, ordinal: number, message: symbol) code "EXECUTOR_ESCAPE" "{message}"
Executor(c, fn, a, t, col, ordinal, cat(label, ": ", fn, ": ", t)) :- bs_emit(c, "EXECUTOR_ESCAPE", fn, a, t), col(c, col), visit_index(c, ordinal), bs_label("EXECUTOR_ESCAPE", label).

.decl bs_graph_kind(n: node, kind: symbol)
bs_graph_kind(n, k) :- bs_api_helper(), option_item("astKinds", n, k).
.decl bs_graph_edge(p: node, field: symbol, n: node)
bs_graph_edge(to_number(p), field, n) :- bs_api_helper(), option_item("astParents", n, p), p != "-1", option_item("astFields", n, field).
bs_callee(f) :- bs_graph_kind(0, "Call"), bs_graph_edge(0, "func", f).
bs_attr(n, obj) :- bs_graph_kind(n, "Attribute"), bs_graph_edge(n, "value", obj).
bs_component(n, name, i) :- bs_graph_kind(n, "Name"), option_item("astIds", i, name), n = i.
bs_component(n, name, i) :- bs_graph_kind(n, "Attribute"), option_item("astAttrs", i, name), n = i.
.violation CallName(at: span, name: symbol) code "BS_CALL_NAME" "{name}"
CallName(@place("<call-name>", 1), name) :- bs_graph_kind(0, "Call"), bs_graph_edge(0, "func", f), bs_name(f, name).
.violation Allow(at: span) code "BS_ALLOW" "allowlisted"
Allow(@place("<allow>", 1)) :- option("entryPoint", "_is_allowlisted"), option("file_rel", p), option("function_name", f), bs_allowed(p, f).
.violation Info(at: span) code "BS_INFO" "informational"
Info(@place("<info>", 1)) :- option("entryPoint", "_is_informational"), option("file_rel", p), bs_informational(p).
'''
    from data_layer_rr_boundary_api import suffix
    return text+suffix(source)
