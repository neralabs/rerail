"""Port callback-wrapper function contexts and completion order to native relations."""
import json,re
from data_layer_rr_text import module,character_class


def transaction_callback_wrappers(source,target,header):
    original=module(source,'transaction_callback_wrappers');q=json.dumps
    text=header('transaction_callback_wrappers',target)
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    text+='#include "python/print.rr"\n#include "project.rr"\n'
    for rel,values in [('tcw_entrypoint',original.TRANSACTION_ENTRYPOINTS),('tcw_versioned',original.REENTRANT_VERSIONED_ENTRYPOINTS),
        ('tcw_allowed_path',original.ALLOWED_RELATIVE_PATHS),('tcw_trigger',original.TRANSACTION_CALLBACK_WRAPPER_EXTRACTOR.source_triggers)]:
        text+=f'.decl {rel}(name: symbol)\n'+''.join(f'{rel}({q(v)}).\n' for v in sorted(values))
    text+='.decl tcw_layer(path: symbol)\ntcw_layer(p) :- doc(_, p, _), match('+q('(?s)l'+character_class(str.isdecimal)+r'{4}_[^/\n][^/]*/.*')+', p).\n'
    return text+projection()+semantics()


def projection():
    return r'''
.decl tcw_graph()
tcw_graph() :- option_item("astKinds", 0, _).
.decl tcw_gkind(n: node, k: symbol)
tcw_gkind(n, k) :- option_item("astKinds", n, k).
.decl tcw_gedge(p: node, field: symbol, c: node)
tcw_gedge(to_number(p), field, c) :- option_item("astParents", c, p), p != "-1", option_item("astFields", c, field).
.decl tcw_gancestor(n: node, a: node)
tcw_gancestor(n, a) :- tcw_gedge(a, _, n).
tcw_gancestor(n, a) :- tcw_gedge(p, _, n), tcw_gancestor(p, a).
.decl tcw_doc(d: number, path: symbol, relative: symbol)
tcw_doc(d, p, relative) :- !tcw_graph(), doc(d, p, _), doc_role(d, "primary"), option_item("documentPaths", i, p), option_item("relativePaths", i, relative).
tcw_doc(d, p, p) :- !tcw_graph(), doc(d, p, _), doc_role(d, "primary"), !option_item("documentPaths", _, p), !option("repositoryRoot", _).
tcw_doc(d, p, @slice(absolute, strlen(root) + 1, strlen(absolute))) :- !tcw_graph(), doc(d, p, _), doc_role(d, "primary"),
 !option_item("documentPaths", _, p), option("repositoryRoot", root), workspace_root(workspace), absolute = cat(workspace, "/", p), prefix(cat(root, "/"), absolute).
tcw_doc(d, p, absolute) :- !tcw_graph(), doc(d, p, _), doc_role(d, "primary"), !option_item("documentPaths", _, p),
 option("repositoryRoot", root), workspace_root(workspace), absolute = cat(workspace, "/", p), !prefix(cat(root, "/"), absolute).
tcw_doc(0, p, relative) :- tcw_graph(), option("originalPath", p), option("relativePath", relative).
.decl tcw_raw(d: number, source: symbol)
tcw_raw(d, text) :- !tcw_graph(), tcw_doc(d, _, _), source(d, sp), text(sp, text).
tcw_raw(0, text) :- tcw_graph(), option("providedSource", text).
.decl tcw_function(f: node, name: symbol)
tcw_function(f, @nfkc(name)) :- !tcw_graph(), kind(f, "function_definition"), def_name(f, name).
tcw_function(f, name) :- tcw_gkind(f, k), (k = "FunctionDef" ; k = "AsyncFunctionDef"), option_item("astNames", f, name).
.decl tcw_class(c: node, name: symbol)
tcw_class(c, @nfkc(name)) :- !tcw_graph(), kind(c, "class_definition"), def_name(c, name).
tcw_class(c, name) :- tcw_gkind(c, "ClassDef"), option_item("astNames", c, name).
.decl tcw_position(f: node, d: number, line: number, entry_order: number, exit_order: number)
// Only function completion order is needed: physical ends ascend, with inner functions first on ties.
tcw_position(f, d, l, i, end) :- !tcw_graph(), tcw_function(f, _), node(d, f, _, _, end), line(f, l), visit_index(f, i).
tcw_position(f, 0, to_number(l), f, to_number(end)) :- tcw_graph(), tcw_function(f, _), option_item("astLines", f, l), option_item("astExitOrder", f, end).
.decl tcw_owner(n: node, f: node)
tcw_owner(n, f) :- !tcw_graph(), def_frame(n, f).
tcw_owner(n, f) :- tcw_graph(), tcw_gancestor(n, f), tcw_function(f, _),
 f = max ancestor : { tcw_gancestor(n, ancestor), tcw_function(ancestor, _) }.
.decl tcw_current_class(n: node, name: symbol)
tcw_current_class(n, @nfkc(name)) :- !tcw_graph(), nearest(n, c, "class_definition"), def_name(c, name).
tcw_current_class(n, name) :- tcw_graph(), tcw_gancestor(n, c), tcw_class(c, name),
 c = max ancestor : { tcw_gancestor(n, ancestor), tcw_class(ancestor, _) }.
.decl tcw_qualname(f: node, name: symbol)
tcw_qualname(f, cat(@nfkc(classes), ".", name)) :- !tcw_graph(), tcw_function(f, name), class_path(f, classes).
tcw_qualname(f, name) :- !tcw_graph(), tcw_function(f, name), !class_path(f, _).
tcw_qualname(f, cat(classes, ".", name)) :- tcw_graph(), tcw_function(f, name), tcw_current_class(f, _),
 classes = join cname by c sep "." : { tcw_gancestor(f, c), tcw_class(c, cname) }.
tcw_qualname(f, name) :- tcw_graph(), tcw_function(f, name), !tcw_current_class(f, _).
.decl tcw_name(n: node, name: symbol)
tcw_name(n, @nfkc(name)) :- !tcw_graph(), unparen(n, p), ast_name(p, name).
tcw_name(n, name) :- tcw_gkind(n, "Name"), option_item("astIds", n, name).
.decl tcw_attr(n: node, name: symbol)
tcw_attr(n, @nfkc(name)) :- !tcw_graph(), unparen(n, a), kind(a, "attribute"), field(a, "attribute", nm), text(nm, name).
tcw_attr(n, name) :- tcw_gkind(n, "Attribute"), option_item("astAttrs", n, name).
.decl tcw_call(c: node, fn: node)
tcw_call(c, fn) :- !tcw_graph(), kind(c, "call"), field(c, "function", raw), unparen(raw, fn).
tcw_call(c, fn) :- tcw_gkind(c, "Call"), tcw_gedge(c, "func", fn).
.decl tcw_posarg(c: node, n: node, i: number)
tcw_posarg(c, n, i) :- !tcw_graph(), tcw_call(c, _), field(c, "arguments", args), child(args, raw, j),
 !kind(raw, "keyword_argument"), !kind(raw, "dictionary_splat"), !kind(raw, "comment"), unparen(raw, n),
 i = count : { child(args, earlier, k), k < j, !kind(earlier, "keyword_argument"), !kind(earlier, "dictionary_splat"), !kind(earlier, "comment") }.
tcw_posarg(c, n, to_number(i)) :- tcw_gkind(c, "Call"), tcw_gedge(c, "args", n), option_item("astOrdinals", n, i).
.decl tcw_keyword(c: node, n: node, i: number)
tcw_keyword(c, n, i) :- !tcw_graph(), tcw_call(c, _), field(c, "arguments", args), child(args, k, i), kind(k, "keyword_argument"),
 field(k, "name", nm), text(nm, name), @nfkc(name) = "fn", field(k, "value", raw), unparen(raw, n).
tcw_keyword(c, n, to_number(i)) :- tcw_gkind(c, "Call"), tcw_gedge(c, "keywords", kw), option_item("astKeywords", kw, "fn"),
 tcw_gedge(kw, "value", n), option_item("astOrdinals", kw, i).
.decl tcw_lambda(n: node, identity: number)
tcw_lambda(n, n) :- !tcw_graph(), kind(n, "lambda").
tcw_lambda(n, to_number(identity)) :- tcw_gkind(n, "Lambda"), option_item("astIdentity", n, identity).
.decl tcw_lambda_ancestor(n: node, identity: number)
tcw_lambda_ancestor(n, identity) :- !tcw_graph(), tcw_lambda(lam, identity), within(n, lam).
tcw_lambda_ancestor(n, identity) :- tcw_graph(), tcw_gancestor(n, lam), tcw_lambda(lam, identity).
.decl tcw_string(n: node, value: symbol)
tcw_string(n, value) :- !tcw_graph(), unparen(n, p), plain_str(p, value).
tcw_string(n, value) :- tcw_gkind(n, "Constant"), option_item("astStringJson", n, encoded), tcw_versioned(value), encoded = @json_str(value).
.decl tcw_parameter_annotation(f: node, annotation: symbol)
tcw_parameter_annotation(f, annotation) :- !tcw_graph(), sig_param(f, _, _, _, _, ann, _), !unset(ann), annotation_text(ann, annotation).
.decl tcw_print(n: node, annotation: symbol)
tcw_print(to_number(@slice(name, 1, strlen(name))), printed) :- tcw_graph(), sig_param(_, _, nm, _, _, ann, _), text(nm, name), prefix("e", name), annotation_text(ann, printed).
tcw_parameter_annotation(f, printed) :- tcw_function(f, _), tcw_gedge(f, "args", args), tcw_gedge(args, kind, p),
 (kind = "args" ; kind = "posonlyargs" ; kind = "kwonlyargs" ; kind = "vararg" ; kind = "kwarg"), tcw_gedge(p, "annotation", ann), tcw_print(ann, printed).
'''


def semantics():
    helper='forbidden transaction callback wrapper helper: accept transactions directly or use existing connection/versioned dispatch infrastructure'
    inline='forbidden inline transaction callback wrapper: nested callback re-enters versioned transaction APIs instead of using read_version*/write_version*'
    text=r'''
.decl tcw_triggered(d: number)
tcw_triggered(d) :- tcw_raw(d, source), tcw_trigger(trigger), contains(trigger, source).
.decl tcw_selected(d: number)
tcw_selected(0) :- tcw_graph().
tcw_selected(d) :- !tcw_graph(), tcw_doc(d, _, _), file_name(d, name), suffix(".py", name), option("entryPoint", _), tcw_api_gate(), tcw_triggered(d), !syntax_error(d, _, _), !tcw_test(d).
tcw_selected(d) :- !tcw_graph(), tcw_doc(d, p, _), !option("entryPoint", _), tcw_triggered(d), !syntax_error(d, _, _), !tcw_test(d), !tcw_hidden(d),
 (prefix("data_layer/", p) ; prefix("_internal/", p) ; tcw_layer(p)).
.decl tcw_test(d: number)
tcw_test(d) :- doc(d, p, _), !option_item("documentPaths", _, p), path_part(d, part), (part = "tests" ; part = "test").
tcw_test(d) :- doc(d, p, _), option_item("documentPaths", i, p), option_item("scopePaths", i, scope), match("(?s)(?:.*/)?(?:tests|test)(?:/.*)?", scope).
.decl tcw_api_gate()
tcw_api_gate() :- option("entryPoint", entry), entry != "scan_file", entry != "scan_source".
tcw_api_gate() :- option("entryPoint", entry), (entry = "scan_file" ; entry = "scan_source"), option("providedSource", source), tcw_trigger(trigger), contains(trigger, source).
.decl tcw_hidden(d: number)
tcw_hidden(d) :- path_part(d, p), (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp").
.decl tcw_inline_enabled(d: number)
tcw_inline_enabled(d) :- tcw_raw(d, source), contains("getattr", source), !option("inlineOverride", _).
tcw_inline_enabled(d) :- tcw_raw(d, source), tcw_versioned(name), contains(name, source), !option("inlineOverride", _).
tcw_inline_enabled(d) :- tcw_doc(d, _, _), option("inlineOverride", "true").
.decl tcw_has_callback_annotation(f: node)
tcw_has_callback_annotation(f) :- tcw_parameter_annotation(f, text), contains("Callable", text), contains("Transaction", text).
.decl tcw_boundary(f: node)
tcw_boundary(f) :- tcw_function(f, name), tcw_entrypoint(name), tcw_current_class(f, cls),
 (cls = "SyncDatabaseConnection" ; cls = "SyncVersionedDatabaseConnection").
.decl tcw_entry_call(c: node)
tcw_entry_call(c) :- tcw_call(c, fn), tcw_attr(fn, method), tcw_entrypoint(method).
.decl tcw_callback(c: node, n: node)
tcw_callback(c, n) :- tcw_entry_call(c), tcw_keyword(c, n, i), i = min j : { tcw_keyword(c, _, j) }.
tcw_callback(c, n) :- tcw_entry_call(c), !tcw_keyword(c, _, _), tcw_posarg(c, n, 1).
.decl tcw_reentrant_call(c: node)
tcw_reentrant_call(c) :- tcw_call(c, fn), tcw_attr(fn, method), tcw_versioned(method).
tcw_reentrant_call(c) :- tcw_call(c, fn), tcw_name(fn, "getattr"), tcw_posarg(c, n, i), i >= 1, tcw_string(n, name), tcw_versioned(name).
.decl tcw_calls_entry(f: node)
tcw_calls_entry(f) :- tcw_entry_call(c), tcw_owner(c, f).
.decl tcw_has_reentrant(f: node)
tcw_has_reentrant(f) :- tcw_reentrant_call(c), tcw_owner(c, f).
.decl tcw_callback_name(f: node, name: symbol)
tcw_callback_name(f, name) :- tcw_callback(c, expr), tcw_owner(c, f), tcw_name(expr, name).
.decl tcw_callback_lambda(f: node, identity: number)
tcw_callback_lambda(f, identity) :- tcw_callback(c, expr), tcw_owner(c, f), tcw_lambda(expr, identity).
.decl tcw_nested_reentrant(f: node, name: symbol)
tcw_nested_reentrant(f, name) :- tcw_function(child, name), tcw_owner(child, f), tcw_has_reentrant(child).
.decl tcw_lambda_reentrant(f: node)
tcw_lambda_reentrant(f) :- tcw_callback_lambda(f, identity), tcw_reentrant_call(c), tcw_owner(c, _), tcw_lambda_ancestor(c, identity).
.decl tcw_inline_reentrant(f: node)
tcw_inline_reentrant(f) :- tcw_callback_name(f, name), tcw_nested_reentrant(f, name).
tcw_inline_reentrant(f) :- tcw_lambda_reentrant(f).
.decl tcw_permitted(f: node)
tcw_permitted(f) :- tcw_position(f, d, _, _, _), tcw_doc(d, _, relative), tcw_allowed_path(relative).
tcw_permitted(f) :- tcw_boundary(f).
.decl tcw_issue(f: node, reason: number)
tcw_issue(f, 0) :- tcw_function(f, _), tcw_position(f, d, _, _, _), tcw_selected(d), !tcw_permitted(f), tcw_has_callback_annotation(f), tcw_calls_entry(f).
tcw_issue(f, 1) :- tcw_function(f, _), tcw_position(f, d, _, _, _), tcw_selected(d), !tcw_permitted(f), tcw_inline_enabled(d), tcw_inline_reentrant(f).
'''
    for reason,message in [(0,helper),(1,inline)]:
        name='Helper' if reason==0 else 'Inline'
        text+=f'''
.violation Tcw{name}API(at: span, qualname: symbol, exit_order: number, entry_order: number, reason: number) code "TCW00{reason+1}" {json.dumps(message)}
Tcw{name}API(@place(path, l), qual, end, entry, {reason}) :- tcw_issue(f, {reason}), tcw_position(f, d, l, entry, end), tcw_doc(d, path, _), tcw_qualname(f, qual), option("entryPoint", _).
.violation Tcw{name}(at: span, qualname: symbol, exit_order: number, entry_order: number, reason: number) code "transaction_callback_wrapper" "{{qualname}}: {message}"
.severity Tcw{name} error
Tcw{name}(@place(path, l), qual, end, entry, {reason}) :- tcw_issue(f, {reason}), tcw_position(f, d, l, entry, end), tcw_doc(d, path, _), tcw_qualname(f, qual), !option("entryPoint", _).
'''
    return text
