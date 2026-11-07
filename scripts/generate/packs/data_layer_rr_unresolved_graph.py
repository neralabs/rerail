"""Native predicates over a caller-supplied AST's typed input columns."""


def supplied_ast():
    return r'''
.decl uf_graph_kind(n: node, kind: symbol)
uf_graph_kind(n, kind) :- option_item("astKinds", n, kind).
.decl uf_graph_edge(p: node, field: symbol, ordinal: number, n: node)
uf_graph_edge(to_number(parent), field, to_number(ordinal), n) :- option_item("astParents", n, parent),
    parent != "-1", option_item("astFields", n, field), option_item("astOrdinals", n, ordinal).
.decl uf_graph_ancestor(p: node, n: node)
uf_graph_ancestor(p, n) :- uf_graph_edge(p, _, _, n).
uf_graph_ancestor(p, n) :- uf_graph_ancestor(p, q), uf_graph_edge(q, _, _, n).
.decl uf_graph_function(n: node)
uf_graph_function(n) :- uf_graph_kind(n, k), (k = "FunctionDef" ; k = "AsyncFunctionDef").
.decl uf_graph_scope(n: node, f: node)
uf_graph_scope(n, n) :- uf_graph_kind(n, "Module").
uf_graph_scope(n, n) :- uf_graph_function(n).
uf_graph_scope(n, f) :- uf_graph_scope(p, f), uf_graph_edge(p, _, _, n), !uf_graph_function(n).
uf_scope(n, f) :- uf_graph_scope(n, f).
uf_name(n, name) :- uf_graph_kind(n, "Name"), option_item("astIds", n, name).
uf_attr(n, obj, name) :- uf_graph_kind(n, "Attribute"), uf_graph_edge(n, "value", _, obj), option_item("astAttrs", n, name).
uf_attribute_node(n) :- uf_graph_kind(n, "Attribute").
uf_subscript_value(n, obj) :- uf_graph_kind(n, "Subscript"), uf_graph_edge(n, "value", _, obj).
uf_call_function(c, f) :- uf_graph_kind(c, "Call"), uf_graph_edge(c, "func", _, f).
uf_arg(c, i, n) :- uf_graph_kind(c, "Call"), uf_graph_edge(c, "args", i, n).
uf_none(n) :- uf_graph_kind(n, "Constant"), option_item("astValueTypes", n, "NoneType").
uf_tuple_item(t, n) :- uf_graph_kind(t, "Tuple"), uf_graph_edge(t, "elts", _, n).
uf_operand(c, 0, n) :- uf_graph_kind(c, "Compare"), uf_graph_edge(c, "left", _, n).
uf_operand(c, i + 1, n) :- uf_graph_kind(c, "Compare"), uf_graph_edge(c, "comparators", i, n).
.decl uf_op_name(kind: symbol, op: symbol)
uf_op_name("Is", "is"). uf_op_name("IsNot", "is not"). uf_op_name("Eq", "=="). uf_op_name("NotEq", "!=").
uf_op(c, i, op) :- uf_graph_kind(c, "Compare"), uf_graph_edge(c, "ops", i, n), uf_graph_kind(n, kind), uf_op_name(kind, op).
uf_parent(n, p) :- uf_graph_edge(p, _, _, n).
uf_contains(p, n) :- uf_graph_ancestor(p, n).
uf_contains(n, n) :- uf_graph_kind(n, _).
uf_function_member(f, n) :- uf_graph_function(f), uf_graph_ancestor(f, n).
uf_assign_value(a, v) :- uf_graph_kind(a, "Assign"), uf_graph_edge(a, "value", _, v).
uf_assign_target(a, t) :- uf_graph_kind(a, "Assign"), uf_graph_edge(a, "targets", _, t).
uf_function_name(f, name) :- uf_graph_function(f), option_item("astNames", f, name).
uf_module(m) :- uf_graph_kind(m, "Module").
.decl uf_supplied_line(n: node, line: number)
uf_supplied_line(n, to_number(line)) :- option_item("astLines", n, line).
.decl uf_supplied_snippet(n: node, snippet: symbol)
uf_supplied_snippet(n, @replace_all(raw, trim, "")) :- uf_supplied_line(n, line), line > 0,
    option_item("sourceLines", line - 1, raw), uf_trim(trim).
uf_supplied_snippet(n, "") :- uf_supplied_line(n, line), !option_item("sourceLines", line - 1, _).
.violation Supplied(at: span, function: symbol, problem: symbol, source: symbol, ordinal: number)
    code "unresolved_forward_ref" "{function}(): {problem}"
Supplied(@place("provided-tree", line), function, problem, snippet, n) :- uf_bad(n, function, problem),
    option("entryPoint", "AnnotationUsageFinder.analyze"), option("providedSource", text), contains(".annotation", text),
    uf_supplied_line(n, line), uf_supplied_snippet(n, snippet).
'''
