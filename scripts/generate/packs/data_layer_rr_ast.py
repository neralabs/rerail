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
