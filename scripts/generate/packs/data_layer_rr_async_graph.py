"""Adapt caller-supplied AST columns to the same async rule predicates as the native tape."""


def supplied_ast():
    return r'''
ap_parent(n, p) :- ap_graph_edge(p, _, n).
.decl ap_graph_kind(n: node, kind: symbol)
ap_graph_kind(n, kind) :- option_item("astKinds", n, kind).
.decl ap_graph_edge(p: node, field: symbol, n: node)
ap_graph_edge(to_number(parent), field, n) :- option_item("astParents", n, parent), parent != "-1", option_item("astFields", n, field).
.decl ap_graph_ancestor(n: node, a: node)
ap_graph_ancestor(n, a) :- ap_graph_edge(a, _, n).
ap_graph_ancestor(n, a) :- ap_graph_edge(p, _, n), ap_graph_ancestor(p, a).
.decl ap_graph_callable(n: node)
ap_graph_callable(n) :- ap_graph_kind(n, k), (k = "FunctionDef" ; k = "AsyncFunctionDef" ; k = "Lambda").
ap_frame_node(n) :- ap_graph_callable(n).
ap_frame_node(n) :- ap_graph_kind(n, "Module").
ap_function(f, name, 1) :- ap_graph_kind(f, "AsyncFunctionDef"), option_item("astNames", f, name).
ap_function(f, name, 0) :- ap_graph_kind(f, "FunctionDef"), option_item("astNames", f, name).
ap_sync_frame(f) :- ap_graph_kind(f, "Lambda").
ap_sync_frame(f) :- ap_graph_kind(f, "FunctionDef").
ap_root_frame(f) :- ap_graph_kind(f, "Module").
ap_class(n, name) :- ap_graph_kind(n, "ClassDef"), option_item("astNames", n, name).
ap_class_base(c, b) :- ap_graph_kind(c, "ClassDef"), ap_graph_edge(c, "bases", b).
ap_decorator(f, e) :- ap_graph_callable(f), ap_graph_edge(f, "decorator_list", e).
ap_name(n, name) :- ap_graph_kind(n, "Name"), option_item("astIds", n, name).
ap_attr(n, obj, name) :- ap_graph_kind(n, "Attribute"), ap_graph_edge(n, "value", obj), option_item("astAttrs", n, name).
ap_call(n, fn) :- ap_graph_kind(n, "Call"), ap_graph_edge(n, "func", fn).
ap_call_expr(n, fn) :- ap_graph_kind(n, "Call"), ap_call(n, fn).
ap_subscript_value(n, obj) :- ap_graph_kind(n, "Subscript"), ap_graph_edge(n, "value", obj).
ap_parameter(f, name) :- ap_graph_callable(f), ap_graph_edge(f, "args", args), ap_graph_edge(args, _, arg),
    ap_graph_kind(arg, "arg"), option_item("astArgNames", arg, name).
ap_parameter_annotation(f, ann) :- ap_graph_callable(f), ap_graph_edge(f, "args", args), ap_graph_edge(args, _, arg),
    ap_graph_kind(arg, "arg"), ap_graph_edge(arg, "annotation", ann).
ap_typed_parameter(f, name, ann) :- ap_graph_callable(f), ap_graph_edge(f, "args", args), ap_graph_edge(args, _, arg),
    ap_graph_kind(arg, "arg"), option_item("astArgNames", arg, name), ap_graph_edge(arg, "annotation", ann).
ap_annotation_edge(n, e) :- ap_graph_kind(n, "Subscript"), ap_graph_edge(n, field, e), (field = "value" ; field = "slice").
ap_annotation_edge(n, e) :- ap_graph_kind(n, "Tuple"), ap_graph_edge(n, "elts", e).
ap_annotation_edge(n, e) :- ap_graph_kind(n, "BinOp"), ap_graph_edge(n, "op", op), ap_graph_kind(op, "BitOr"),
    ap_graph_edge(n, field, e), (field = "left" ; field = "right").
// JSON preserves surrogate constants losslessly. Frozen transaction type names are ASCII, whose substrings JSON leaves unchanged.
ap_annotation_string(n, value) :- ap_graph_kind(n, "Constant"), option_item("astValueTypes", n, "str"), option_item("astStringJson", n, value).
ap_assignment(a, target, value) :- ap_graph_kind(a, "Assign"), ap_graph_edge(a, "targets", t), ap_name(t, target), ap_graph_edge(a, "value", value).
ap_assignment(a, target, value) :- ap_graph_kind(a, "AnnAssign"), ap_graph_edge(a, "target", t), ap_name(t, target), ap_graph_edge(a, "value", value).
ap_order(n, n) :- ap_graph_kind(n, _).
ap_call_line(n, to_number(line)) :- ap_graph_kind(n, "Call"), option_item("astLines", n, line).
ap_receiver_print(c, printed) :- ap_supplied(), ap_graph_kind(c, "Call"),
    sig_param(_, _, nm, _, _, ann, _), text(nm, name), prefix("call", name), c = to_number(@slice(name, 4, strlen(name))), annotation_text(ann, printed).
'''
