"""Author AST rules against native Python views, preserving source collector contracts."""


def classvar_check(source, target, header):
    return header('classvar_check', target) + r'''
.decl cv_source(d: number, t: symbol)
cv_source(d, t) :- source(d, sp), text(sp, t).
.decl cv_trigger(d: number)
cv_trigger(d) :- cv_source(d, t), (contains("get_origin", t) ; contains("ClassVar", t)).
.decl cv_api_gate(d: number)
cv_api_gate(d) :- doc(d, _, _), !option("entryPoint", _).
cv_api_gate(d) :- cv_source(d, t), option("entryPoint", "lint_file"), contains("get_origin", t), contains("ClassVar", t).
cv_api_gate(d) :- cv_source(d, t), option("entryPoint", "lint_directory"), contains("get_origin", t), contains("ClassVar", t).
// lint_file_content uses its source argument only as a gate; the framework still reads the named file.
cv_api_gate(d) :- doc(d, _, _), option("entryPoint", "lint_file_content"), option("callSource", t),
    contains("get_origin", t), contains("ClassVar", t).
.decl cv_leaf(e: node, x: symbol)
cv_leaf(e, @nfkc(t)) :- unparen(e, n), kind(n, "identifier"), text(n, t).
cv_leaf(e, @nfkc(t)) :- unparen(e, n), kind(n, "attribute"), field(n, "attribute", a), text(a, t).
// Comments are CST children but are absent from the AST's comparator sequence.
.decl cv_operand(c: node, i: number, n: node)
cv_operand(c, i, n) :- kind(c, "comparison_operator"), child(c, n, j),
    !kind(n, "comment"), !kind(n, "line_continuation"),
    i = count : { child(c, b, k), k < j, !kind(b, "comment"), !kind(b, "line_continuation") }.
.decl cv_candidate(c: node, ordinal: number, style: symbol)
cv_candidate(c, i, style) :- cv_operand(c, 0, raw), unparen(raw, call), kind(call, "call"),
    field(call, "function", f), cv_leaf(f, "get_origin"), op_at(c, i, op), cv_operand(c, i + 1, other),
    cv_leaf(other, "ClassVar"), (op = "is", style = "is/==" ; op = "==", style = "is/==" ;
    op = "is not", style = "is not/!=" ; op = "!=", style = "is not/!="),
    docof(c, d), cv_trigger(d), cv_api_gate(d), !syntax_error(d, _, _).
.decl cv_helper(f: node)
cv_helper(f) :- kind(f, "function_definition"), def_name(f, name), @nfkc(name) = "is_classvar_annotation", !op(f, "async").
.decl cv_exempt(c: node)
cv_exempt(c) :- cv_candidate(c, _, _), cv_helper(f), within(c, f).
cv_exempt(c) :- cv_candidate(c, _, _), cv_helper(f), decorator(f, e), c = e.
cv_exempt(c) :- cv_candidate(c, _, _), cv_helper(f), decorator(f, e), within(c, e).
.violation ClassVarComparison(at: node, ordinal: number, style: symbol, column: number) code "CLASSVAR001"
    "Direct get_origin(...) {style} ClassVar check. This fails for string annotations (PEP 563). Use is_classvar_annotation() from l1000_schema.type_utils instead."
// Keep the comparator ordinal: repeated links produce repeated public DTOs, as the source collector does.
ClassVarComparison(c, ordinal, style, column) :- cv_candidate(c, ordinal, style), !cv_exempt(c), col(c, column).
'''


def forwardref(source, target, header):
    return header('forwardref', target) + r'''
.decl fr_trigger(d: number)
fr_trigger(d) :- source(d, sp), text(sp, t), (contains(".annotation", t) ; contains("ForwardRef", t)).
.decl fr_sync(f: node)
fr_sync(f) :- kind(f, "function_definition"), !op(f, "async").
// Sync functions reset and restore this collector's state. Async functions do neither.
.decl fr_in(f: node, n: node)
fr_in(f, n) :- fr_sync(f), within(n, f).
fr_in(f, n) :- fr_sync(f), decorator(f, e), (n = e ; within(n, e)).
.decl fr_outer(f: node, n: node)
fr_outer(f, n) :- fr_in(f, n), fr_in(g, n), fr_in(f, g), f != g.
.decl fr_scope(n: node, f: node)
fr_scope(n, f) :- fr_in(f, n), !fr_outer(f, n).
fr_scope(n, m) :- kind(n, _), docof(n, d), modnode(d, m), !fr_in(_, n).
.decl fr_name(e: node, x: symbol)
fr_name(e, @nfkc(t)) :- unparen(e, n), ast_name(n, t).
.decl fr_attr(e: node, x: symbol)
fr_attr(e, @nfkc(t)) :- unparen(e, n), kind(n, "attribute"), field(n, "attribute", a), text(a, t).
.decl fr_nested_assign(a: node)
fr_nested_assign(a) :- kind(p, "assignment"), field(p, "right", a).
.decl fr_assign(a: node, f: node)
fr_assign(a, f) :- kind(a, "assignment"), !field(a, "type", _), !fr_nested_assign(a),
    field(a, "left", t), fr_name(t, "annotation"), field(a, "right", v), fr_attr(v, "annotation"), fr_scope(a, f).
.decl fr_arg(c: node, i: number, n: node)
fr_arg(c, i, n) :- kind(c, "call"), field(c, "arguments", al), child(al, n, j),
    !kind(n, "keyword_argument"), !kind(n, "dictionary_splat"), !kind(n, "comment"), !kind(n, "line_continuation"),
    i = count : { child(al, a, k), k < j, !kind(a, "keyword_argument"), !kind(a, "dictionary_splat"),
        !kind(a, "comment"), !kind(a, "line_continuation") }.
.decl fr_callable(c: node, name: symbol)
fr_callable(c, name) :- kind(c, "call"), field(c, "function", fn), fr_name(fn, name), fr_arg(c, 0, a), fr_name(a, "annotation").
.decl fr_contains_ref(e: node)
fr_contains_ref(e) :- fr_name(e, "ForwardRef").
fr_contains_ref(e) :- fr_attr(e, "ForwardRef").
fr_contains_ref(e) :- unparen(e, t), kind(t, "tuple"), child(t, child, _), fr_contains_ref(child).
.decl fr_good(c: node)
fr_good(c) :- fr_callable(c, "isinstance"), fr_arg(c, 1, a), fr_contains_ref(a).
.decl fr_candidate(c: node, f: node, which: symbol)
fr_candidate(c, f, "isinstance") :- fr_callable(c, "isinstance"), fr_arg(c, 1, a), fr_name(a, "type"), fr_scope(c, f).
fr_candidate(c, f, "issubclass") :- fr_callable(c, "issubclass"), fr_scope(c, f).
fr_candidate(c, f, "callable") :- fr_callable(c, "callable"), fr_scope(c, f).
.decl fr_newer(c: node, a: node)
fr_newer(c, a) :- fr_candidate(c, f, _), fr_assign(a, f), fr_assign(b, f), earlier(a, b), earlier(b, c).
.decl fr_latest(c: node, a: node)
fr_latest(c, a) :- fr_candidate(c, f, _), fr_assign(a, f), earlier(a, c), !fr_newer(c, a).
.decl fr_checked(c: node)
fr_checked(c) :- fr_latest(c, a), fr_scope(c, f), fr_good(g), fr_scope(g, f), earlier(a, g), earlier(g, c).
.decl fr_bad(c: node, which: symbol, column: number, ordinal: number)
fr_bad(c, which, column, ordinal) :- fr_candidate(c, _, which), fr_latest(c, _), !fr_checked(c),
    docof(c, d), fr_file(d), fr_trigger(d), fr_api_gate(d), !syntax_error(d, _, _), col(c, column), visit_index(c, ordinal).
.decl fr_file(d: number)
fr_file(d) :- file_name(d, n), suffix(".py", n), !path_part(d, "tests"), !path_part(d, "test"), !fr_excluded(d).
.decl fr_excluded(d: number)
fr_excluded(d) :- doc(d, p, _), option_item("logicalPaths", i, p), option_item("originalPaths", i, original),
    option_item("excludePatterns", _, pattern), contains(pattern, original).
.decl fr_api_gate(d: number)
fr_api_gate(d) :- doc(d, _, _), !option("callSource", _).
fr_api_gate(d) :- doc(d, _, _), option("callSource", t), (contains(".annotation", t) ; contains("ForwardRef", t)).
.violation ForwardRefSourceGate(at: span) code "FWDREF_GATE" "source trigger"
ForwardRefSourceGate(@place("input.py", 1)) :- option("entryPoint", "source_gate"), option("callSource", t),
    (contains(".annotation", t) ; contains("ForwardRef", t)).
.violation ForwardRefFileGate(at: span) code "FWDREF_FILE_GATE" "file scope"
ForwardRefFileGate(@place(p, 1)) :- option("entryPoint", "file_gate"), doc(d, p, _), fr_file(d).
.violation ForwardRefInstance(at: node, column: number, ordinal: number) code "FWDREF001"
    "isinstance(annotation, type) without ForwardRef handling. ForwardRef annotations will fail this check. Consider using coerce_sys_id_value() or checking for ForwardRef first."
ForwardRefInstance(c, column, ordinal) :- fr_bad(c, "isinstance", column, ordinal).
.violation ForwardRefSubclass(at: node, column: number, ordinal: number) code "FWDREF002"
    "issubclass(annotation, ...) without ForwardRef handling. ForwardRef annotations will raise TypeError. Consider checking for ForwardRef first."
ForwardRefSubclass(c, column, ordinal) :- fr_bad(c, "issubclass", column, ordinal).
.violation ForwardRefCallable(at: node, column: number, ordinal: number) code "FWDREF003"
    "callable(annotation) without ForwardRef handling. ForwardRef is callable but calling it produces wrong results. Consider checking for ForwardRef first."
ForwardRefCallable(c, column, ordinal) :- fr_bad(c, "callable", column, ordinal).
'''
