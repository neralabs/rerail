"""Exact ordered transaction-detector events over the native tape and supplied AST columns."""
import json,re
from data_layer_rr_text import module,character_class


def transaction_closures(source,target,header):
    original=module(source,'transaction_closures');q=json.dumps
    text=header('transaction_closures',target)
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    for rel,values in [('tx_method',original.TRANSACTION_METHODS),('tx_mutator',original.MUTATING_METHODS),('tx_trigger',original.TRANSACTION_CALL_TRIGGERS)]:
        text+=f'.decl {rel}(name: symbol)\n'+''.join(f'{rel}({q(s)}).\n' for s in sorted(values))
    text+='.decl tx_layer(p: symbol)\ntx_layer(p) :- doc(_, p, _), match('+q('(?s)l'+character_class(str.isdecimal)+r'{4}_[^/\n][^/]*/.*')+', p).\n'
    text+=projection()+semantics()
    return text


def projection():
    return r'''
// Supplied ASTs are external argument transport. Production reads the native tape.
.decl tx_graph()
tx_graph() :- option_item("astKinds", 0, _).
.decl tx_gkind(n: node, k: symbol)
tx_gkind(n, k) :- option_item("astKinds", n, k).
.decl tx_gedge(p: node, field: symbol, c: node)
tx_gedge(to_number(p), f, c) :- option_item("astParents", c, p), p != "-1", option_item("astFields", c, f).
.decl tx_n(n: node, d: number, l: number, c: number, i: number)
tx_n(n, d, l, c, i) :- !tx_graph(), docof(n, d), doc_role(d, "primary"), line(n, l), col(n, c), visit_index(n, i).
tx_n(n, 0, to_number(l), to_number(c), n) :- tx_gkind(n, _), option_item("astLines", n, l), option_item("astColumns", n, c).
.decl tx_doc(d: number, p: symbol)
tx_doc(d, p) :- !tx_graph(), doc(d, p, _), doc_role(d, "primary").
tx_doc(0, p) :- tx_graph(), option("originalPath", p).
.decl tx_parent(c: node, p: node)
tx_parent(c, p) :- !tx_graph(), ast_parent(c, p).
tx_parent(c, p) :- tx_gedge(p, _, c).
.decl tx_function(f: node, name: symbol)
tx_function(f, @nfkc(name)) :- !tx_graph(), kind(f, "function_definition"), def_name(f, name).
tx_function(f, name) :- tx_gkind(f, k), (k = "FunctionDef" ; k = "AsyncFunctionDef"), option_item("astNames", f, name).
.decl tx_class(c: node, name: symbol)
tx_class(c, @nfkc(name)) :- !tx_graph(), kind(c, "class_definition"), def_name(c, name).
tx_class(c, name) :- tx_gkind(c, "ClassDef"), option_item("astNames", c, name).
.decl tx_lambda(n: node, body: node)
tx_lambda(n, b) :- !tx_graph(), kind(n, "lambda"), field(n, "body", raw), unparen(raw, b).
tx_lambda(n, b) :- tx_gkind(n, "Lambda"), tx_gedge(n, "body", b).
.decl tx_body(o: node, s: node)
tx_body(o, s) :- !tx_graph(), (kind(o, "function_definition") ; kind(o, "class_definition")), field(o, "body", b), body_stmt(b, st, _), tx_stmt(st, s).
tx_body(o, s) :- !tx_graph(), kind(o, "module"), body_stmt(o, st, _), tx_stmt(st, s).
tx_body(o, s) :- tx_gedge(o, "body", s), !tx_gkind(o, "Lambda").
.decl tx_stmt(st: node, s: node)
tx_stmt(st, s) :- defstmt(s, st).
tx_stmt(st, s) :- asgstmt(s, st).
tx_stmt(st, st) :- body_stmt(_, st, _), !kind(st, "decorated_definition"), !asgstmt(_, st), !kind(st, "function_definition"), !kind(st, "class_definition").
.decl tx_module(m: node)
tx_module(m) :- !tx_graph(), kind(m, "module").
tx_module(m) :- tx_gkind(m, "Module").
.decl tx_name(n: node, name: symbol)
tx_name(n, @nfkc(name)) :- !tx_graph(), ast_name(n, name).
tx_name(n, name) :- tx_gkind(n, "Name"), option_item("astIds", n, name).
.decl tx_attr(n: node, obj: node, name: symbol)
tx_attr(n, o, @nfkc(name)) :- !tx_graph(), kind(n, "attribute"), field(n, "object", raw), unparen(raw, o), field(n, "attribute", a), text(a, name).
tx_attr(n, o, name) :- tx_gkind(n, "Attribute"), tx_gedge(n, "value", o), option_item("astAttrs", n, name).
.decl tx_call(c: node, f: node)
tx_call(c, f) :- !tx_graph(), kind(c, "call"), field(c, "function", raw), unparen(raw, f).
tx_call(c, f) :- tx_gkind(c, "Call"), tx_gedge(c, "func", f).
.decl tx_kw(c: node, v: node, i: number)
tx_kw(c, v, i) :- !tx_graph(), kind(c, "call"), field(c, "arguments", a), child(a, k, i), kind(k, "keyword_argument"), field(k, "name", n), text(n, raw), @nfkc(raw) = "fn", field(k, "value", rawv), unparen(rawv, v).
tx_kw(c, v, to_number(i)) :- tx_gedge(c, "keywords", k), option_item("astKeywords", k, "fn"), tx_gedge(k, "value", v), option_item("astOrdinals", k, i).
.decl tx_pos(c: node, v: node, i: number)
tx_pos(c, v, i) :- !tx_graph(), kind(c, "call"), field(c, "arguments", a), child(a, raw, j), !kind(raw, "keyword_argument"), !kind(raw, "dictionary_splat"), !kind(raw, "comment"), unparen(raw, v),
 i = count : { child(a, earlier, k), k < j, !kind(earlier, "keyword_argument"), !kind(earlier, "dictionary_splat"), !kind(earlier, "comment") }.
tx_pos(c, v, to_number(i)) :- tx_gedge(c, "args", v), tx_gkind(c, "Call"), option_item("astOrdinals", v, i).
.decl tx_param(f: node, name: symbol, k: symbol)
tx_param(f, @nfkc(raw), k) :- !tx_graph(), sig_param(f, _, n, _, k, _, _), text(n, raw).
tx_param(f, name, "pos") :- tx_gedge(f, "args", a), tx_gedge(a, "args", p), option_item("astArgNames", p, name).
tx_param(f, name, "vararg") :- tx_gedge(f, "args", a), tx_gedge(a, "vararg", p), option_item("astArgNames", p, name).
tx_param(f, name, "kwarg") :- tx_gedge(f, "args", a), tx_gedge(a, "kwarg", p), option_item("astArgNames", p, name).
.decl tx_bind(a: node, name: symbol, form: symbol)
tx_bind(a, @nfkc(name), "assign") :- !tx_graph(), assign_target(a, raw), unparen(raw, n), kind(n, "identifier"), text(n, name).
tx_bind(a, @nfkc(name), "named") :- !tx_graph(), kind(a, "named_expression"), field(a, "name", n), text(n, name).
tx_bind(a, name, "assign") :- tx_gkind(a, k), (k = "Assign" ; k = "AnnAssign"), tx_gedge(a, f, n), (f = "targets" ; f = "target"), tx_name(n, name).
tx_bind(a, name, "named") :- tx_gkind(a, "NamedExpr"), tx_gedge(a, "target", n), tx_name(n, name).
.decl tx_substore(n: node, name: symbol)
tx_substore(n, name) :- !tx_graph(), kind(n, "subscript"), expr_ctx(n, "store"), field(n, "value", raw), unparen(raw, v), tx_name(v, name).
tx_substore(n, name) :- tx_gkind(n, "Subscript"), tx_gedge(n, "ctx", ctx), tx_gkind(ctx, "Store"), tx_gedge(n, "value", v), tx_name(v, name).
.decl tx_global(n: node, name: symbol, i: number, k: symbol)
tx_global(n, @nfkc(name), i, "TXN001") :- !tx_graph(), kind(n, "nonlocal_statement"), child(n, c, i), text(c, name).
tx_global(n, @nfkc(name), i, "TXN002") :- !tx_graph(), kind(n, "global_statement"), child(n, c, i), text(c, name).
tx_global(to_number(n), name, to_number(i), "TXN001") :- option_item("astListNodes", j, n), option_item("astListNames", j, name), option_item("astListOrdinals", j, i), tx_gkind(to_number(n), "Nonlocal").
tx_global(to_number(n), name, to_number(i), "TXN002") :- option_item("astListNodes", j, n), option_item("astListNames", j, name), option_item("astListOrdinals", j, i), tx_gkind(to_number(n), "Global").
.decl tx_import(n: node, name: symbol)
tx_import(n, @nfkc(x)) :- !tx_graph(), kind(n, "import_statement"), field(n, "name", a), kind(a, "dotted_name"), child(a, first, 0), text(first, x).
tx_import(n, @nfkc(x)) :- !tx_graph(), (kind(n, "import_from_statement") ; kind(n, "future_import_statement")), field(n, "name", a), kind(a, "dotted_name"), text(a, x).
tx_import(n, @nfkc(x)) :- !tx_graph(), (kind(n, "import_statement") ; kind(n, "import_from_statement") ; kind(n, "future_import_statement")), field(n, "name", a), kind(a, "aliased_import"), field(a, "alias", al), text(al, x).
tx_import(n, name) :- tx_gkind(n, k), (k = "Import" ; k = "ImportFrom"), tx_gedge(n, "names", a), option_item("astAsNames", a, name), name != "".
tx_import(n, first) :- tx_gkind(n, "Import"), tx_gedge(n, "names", a), option_item("astAsNames", a, ""), option_item("astNames", a, name), match("(?<first>[^.]+)(?:[.].*)?", name).
tx_import(n, name) :- tx_gkind(n, "ImportFrom"), tx_gedge(n, "names", a), option_item("astAsNames", a, ""), option_item("astNames", a, name).
// Containment is lazy on the production tape; supplied arguments use their flat parent columns.
.decl tx_contains(n: node, f: node)
tx_contains(n, f) :- !tx_graph(), tx_function(f, _), within(n, f).
tx_contains(n, f) :- !tx_graph(), tx_function(f, _), decorator(f, e), (n = e ; within(n, e)).
tx_contains(n, f) :- tx_graph(), tx_parent(n, f), tx_function(f, _).
tx_contains(n, f) :- tx_graph(), tx_parent(n, p), tx_contains(p, f).
.decl tx_owner(n: node, f: node)
tx_owner(n, f) :- !tx_graph(), def_frame(n, f).
tx_owner(n, f) :- tx_graph(), tx_contains(n, f), tx_n(f, _, _, _, i), i = max j : { tx_contains(n, other), tx_n(other, _, _, _, j) }.
'''


def semantics():
    return r'''
.decl tx_triggered(d: number)
tx_triggered(d) :- source(d, sp), text(sp, t), tx_trigger(trigger), contains(trigger, t).
.decl tx_selected(d: number)
tx_selected(d) :- !tx_graph(), tx_doc(d, _), option("entryPoint", _), !syntax_error(d, _, _).
tx_selected(0) :- tx_graph(), !option("entryPoint", "source").
tx_selected(0) :- tx_graph(), option("entryPoint", "source"), option("providedSource", s), tx_trigger(t), contains(t, s).
tx_selected(d) :- tx_doc(d, p), !option("entryPoint", _), tx_triggered(d), !syntax_error(d, _, _),
 (prefix("impl/", p) ; prefix("data_layer/", p) ; prefix("_internal/", p) ; tx_layer(p)),
 !tx_hidden(d), !tx_test(d).
.decl tx_hidden(d: number)
tx_hidden(d) :- path_part(d, p), (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp").
.decl tx_test(d: number)
tx_test(d) :- path_part(d, p), (p = "tests" ; p = "test").
.decl tx_callback(c: node, f: node, method: symbol)
tx_callback(c, fn, method) :- tx_call(c, callee), tx_attr(callee, _, method), tx_method(method), tx_kw(c, fn, i), i = min j : { tx_kw(c, _, j) }.
tx_callback(c, fn, method) :- tx_call(c, callee), tx_attr(callee, _, method), tx_method(method), !tx_kw(c, _, _), tx_pos(c, fn, 1).
.decl tx_module_bind(m: node, name: symbol)
tx_module_bind(m, name) :- tx_module(m), tx_body(m, a), tx_bind(a, name, "assign").
tx_module_bind(m, name) :- tx_module(m), tx_body(m, f), tx_function(f, name).
tx_module_bind(m, name) :- tx_module(m), tx_body(m, c), tx_class(c, name).
tx_module_bind(m, name) :- tx_module(m), tx_body(m, i), tx_import(i, name).
.decl tx_func_bind(f: node, name: symbol)
tx_func_bind(f, name) :- tx_function(f, _), tx_param(f, name, k), (k = "pos" ; k = "vararg" ; k = "kwarg").
tx_func_bind(f, name) :- tx_function(f, _), tx_contains(a, f), tx_bind(a, name, _).
.decl tx_direct_outer(f: node, name: symbol)
tx_direct_outer(f, name) :- tx_function(f, _), tx_param(f, name, "pos").
tx_direct_outer(f, name) :- tx_function(f, _), tx_body(f, a), tx_bind(a, name, "assign").
.decl tx_last_inner(f: node, name: symbol, inner: node)
tx_last_inner(f, name, inner) :- tx_function(f, _), tx_body(f, inner), tx_function(inner, name), tx_n(inner, _, _, _, i),
 i = max j : { tx_body(f, other), tx_function(other, name), tx_n(other, _, _, _, j) }.
// One job per call occurrence. Named definitions can generate the same violation for several calls.
.decl tx_job(call: node, root: node, prefix: symbol)
tx_job(c, body, cat("In lambda passed to ", method, "(): ")) :- tx_callback(c, fn, method), tx_lambda(fn, body),
 tx_n(c, d, _, _, _), tx_selected(d), !option("checkInline", "false"), !option("entryPoint", "detector").
tx_job(c, fn, cat("In function passed to ", method, "(): ")) :- tx_callback(c, fn, method), tx_function(fn, _),
 tx_n(c, d, _, _, _), tx_selected(d), !option("checkInline", "false"), !option("entryPoint", "detector").
tx_job(c, inner, cat("In '", name, "()' passed to ", method, "(): ")) :- tx_callback(c, fn, method), tx_name(fn, name),
 tx_owner(c, f), tx_last_inner(f, name, inner), tx_n(inner, _, dl, _, _), tx_n(c, d, cl, _, _), dl < cl,
 tx_selected(d), !option("checkNamed", "false"), !option("entryPoint", "detector").
tx_job(0, 0, "") :- tx_graph(), option("entryPoint", "detector").
.decl tx_outer(call: node, name: symbol)
tx_outer(0, name) :- option("entryPoint", "detector"), option_item("outerVars", _, name).
tx_outer(c, name) :- tx_job(c, _, _), tx_callback(c, fn, _), (tx_lambda(fn, _) ; tx_function(fn, _)), tx_n(c, d, _, _, _), tx_n(m, d, _, _, _), tx_module_bind(m, name).
tx_outer(c, name) :- tx_job(c, _, _), tx_callback(c, fn, _), (tx_lambda(fn, _) ; tx_function(fn, _)), tx_contains(c, f), tx_func_bind(f, name).
tx_outer(c, name) :- tx_job(c, _, _), tx_callback(c, fn, _), tx_name(fn, _), tx_owner(c, f), tx_direct_outer(f, name).
// Detector function visits skip decorators, annotations and defaults; lambdas stop entirely.
.decl tx_step(p: node, c: node)
tx_step(p, c) :- tx_parent(c, p), !tx_function(p, _), !tx_lambda(p, _).
tx_step(p, c) :- tx_function(p, _), tx_body(p, c).
.decl tx_walk(call: node, n: node)
tx_walk(c, r) :- tx_job(c, r, _).
tx_walk(c, n) :- tx_walk(c, p), tx_step(p, n).
.decl tx_local_event(call: node, name: symbol, i: number)
tx_local_event(c, name, i) :- tx_walk(c, a), tx_bind(a, name, _), tx_n(a, _, _, _, i).
tx_local_event(c, name, i) :- tx_walk(c, f), tx_function(f, _), tx_param(f, name, k), (k = "pos" ; k = "vararg" ; k = "kwarg"), tx_n(f, _, _, _, i).
tx_local_event(c, name, -1) :- tx_job(c, _, _), tx_callback(c, fn, _), tx_lambda(fn, _), tx_param(fn, name, k), (k = "pos" ; k = "vararg" ; k = "kwarg").
tx_local_event(0, name, -1) :- option("entryPoint", "detector"), option_item("initialLocals", _, name).
.decl tx_local_before(call: node, name: symbol, n: node)
tx_local_before(c, name, n) :- tx_walk(c, n), tx_n(n, _, _, _, i), tx_local_event(c, name, j), j <= i.
.decl tx_hit(call: node, n: node, rank: number, code: symbol, message: symbol)
tx_hit(c, n, i, "TXN001", cat("TXN001: Transaction closure uses 'nonlocal ", name, "' - state will be corrupted on retry. Return results instead.")) :-
 tx_walk(c, n), tx_global(n, name, i, "TXN001").
tx_hit(c, n, i, "TXN002", cat("TXN002: Transaction closure uses 'global ", name, "' - state will be corrupted on retry. Return results instead.")) :-
 tx_walk(c, n), tx_global(n, name, i, "TXN002").
tx_hit(c, n, 0, "TXN003", cat("TXN003: Transaction closure calls '", name, ".", method, "()' on outer scope variable - state will be corrupted on retry. Return results instead.")) :-
 tx_walk(c, n), tx_call(n, f), tx_attr(f, obj, method), tx_name(obj, name), tx_mutator(method), tx_outer(c, name), !tx_local_before(c, name, n).
tx_hit(c, n, 0, "TXN004", cat("TXN004: Transaction closure assigns to '", name, "[...]' - outer scope variable will be corrupted on retry. Return results instead.")) :-
 tx_walk(c, n), tx_substore(n, name), tx_outer(c, name), !tx_local_before(c, name, n).
.decl tx_issue(p: symbol, l: number, column: number, call: number, ordinal: number, rank: number, code: symbol, message: symbol)
tx_issue(p, l, col, ci, ni, rank, code, cat(prefix, message)) :- tx_hit(c, n, rank, code, message), tx_job(c, _, prefix),
 tx_n(n, d, l, col, ni), tx_doc(d, p), tx_n(c, _, _, _, ci).
'''+''.join(f'''
.severity Tx{i} error
.violation Tx{i}(at: span, column: number, call: number, ordinal: number, rank: number, message: symbol) code "TXN00{i}" "{{message}}"
Tx{i}(@place(p, l), col, call, ordinal, rank, message) :- tx_issue(p, l, col, call, ordinal, rank, "TXN00{i}", message).
''' for i in range(1,5))+r'''
.violation Local(at: span, name: symbol) code "TX_LOCAL" "{name}"
Local(@place("input.py", 1), name) :- option("entryPoint", "detector"), tx_local_event(0, name, _).
.violation TxScope(at: span, name: symbol) code "TX_SCOPE" "{name}"
TxScope(@place("input.py", 1), name) :- option("entryPoint", "visitor"), tx_module_bind(_, name).
TxScope(@place("input.py", 1), name) :- option("entryPoint", "scope"), option_item("scopeNames", _, name).
.severity SyntaxFailure error
.violation SyntaxFailure(at: span, column: number, call: number, ordinal: number, rank: number, message: symbol) code "TXN000" "{message}"
SyntaxFailure(@place(p, 1), 0, -1, -1, 0, cat("Syntax error: ", message, " (", base, ", line ", to_string(l), ")")) :- tx_doc(d, p), syntax_error(d, l, message), file_name(d, base), !tx_graph(), option("entryPoint", e), (e = "lint_file" ; e = "lint_paths"), !option("readError", "true"), !contains("null bytes", message).
SyntaxFailure(@place(p, 1), 0, -1, -1, 0, cat("Syntax error: ", message)) :- tx_doc(d, p), syntax_error(d, _, message), contains("null bytes", message), !tx_graph(), option("entryPoint", e), (e = "lint_file" ; e = "lint_paths").
.violation ReadFailure(at: span, column: number, call: number, ordinal: number, rank: number, message: symbol) code "TXN000" "{message}"
ReadFailure(@place(p, 1), 0, -1, -1, 0, cat("Failed to parse: ", error)) :- tx_doc(_, p), option("entryPoint", "lint_file"), option("readError", error), error != "".
ReadFailure(@place(p, 1), 0, -1, -1, 0, cat("Failed to parse: ", error)) :- option("entryPoint", "lint_paths"), option_item("errorPaths", i, p), option_item("errorMessages", i, error).
.violation MissingTree(at: span) code "TX_TREE" "AST tree is required; parsing is owned by the fact framework"
MissingTree(@place("input.py", 1)) :- option("entryPoint", "source_missing"), option("providedSource", s), tx_trigger(t), contains(t, s).
.decl tx_inventory_selected(i: number)
tx_inventory_selected(i) :- option("entryPoint", "selection"), option_item("inventoryPath", i, p), suffix(".py", p), option_item("inventoryKind", i, "direct").
tx_inventory_selected(i) :- option("entryPoint", "selection"), option_item("inventoryPath", i, p), suffix(".py", p), option_item("inventoryKind", i, "nested"),
 option_item("inventoryTraversal", i, t), !match("(?s)(?:.*/)?(?:__pycache__|[.]venv|tmp)(?:/.*)?", t), !tx_inventory_test(i).
.decl tx_inventory_test(i: number)
tx_inventory_test(i) :- option("excludeTests", "true"), option_item("inventoryPath", i, p),
 (contains("/test_", p) ; contains("/tests/", p) ; suffix("/conftest.py", p) ; p = "conftest.py").
.violation Selected(at: span, index: number) code "TX_FILE" "selected"
Selected(@place("input.py", 1), i) :- tx_inventory_selected(i).
.violation TxConstant(at: span, table: symbol, value: symbol) code "TX_CONSTANT" "{value}"
TxConstant(@place("constants.py", 1), "transaction", name) :- option("entryPoint", "constants"), tx_method(name).
TxConstant(@place("constants.py", 1), "mutating", name) :- option("entryPoint", "constants"), tx_mutator(name).
'''
