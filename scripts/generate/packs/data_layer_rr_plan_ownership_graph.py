"""The supplied collector AST uses the same ownership predicates as the native tape."""


def supplied_ast():
    return r'''
.decl pso_graph_kind(n: node, kind: symbol)
pso_graph_kind(n, k) :- option("entryPoint", "collector"), option_item("astKinds", n, k).
.decl pso_graph_edge(p: node, field: symbol, n: node)
pso_graph_edge(to_number(p), field, n) :- option("entryPoint", "collector"), option_item("astParents", n, p), p != "-1", option_item("astFields", n, field).
pso_document(0, p) :- option("entryPoint", "collector"), option("relativePath", p).
pso_raw(0, text) :- option("entryPoint", "collector"), option("providedSource", text).
pso_valid(0) :- option("entryPoint", "collector").
pso_node(n, 0, to_number(line), n) :- pso_graph_kind(n, _), option_item("astLines", n, line), line != "0".
pso_function(f, name) :- pso_graph_kind(f, k), (k = "FunctionDef" ; k = "AsyncFunctionDef"), option_item("astNames", f, name).
pso_class(c, name) :- pso_graph_kind(c, "ClassDef"), option_item("astNames", c, name).
pso_name(n, name) :- pso_graph_kind(n, "Name"), option_item("astIds", n, name).
pso_attr(n, obj, name) :- pso_graph_kind(n, "Attribute"), pso_graph_edge(n, "value", obj), option_item("astAttrs", n, name).
pso_call(c, fn) :- pso_graph_kind(c, "Call"), pso_graph_edge(c, "func", fn).
pso_inside(n, p) :- pso_graph_edge(p, _, n).
pso_inside(n, a) :- pso_graph_edge(p, _, n), pso_inside(p, a).
.decl pso_graph_frame(n: node, f: node)
pso_graph_frame(n, n) :- pso_function(n, _).
pso_graph_frame(n, f) :- pso_graph_edge(p, _, n), pso_graph_frame(p, f), !pso_function(n, _).
pso_owner(n, name) :- pso_graph_edge(p, _, n), pso_graph_frame(p, f), pso_function(f, name).
pso_store(n) :- pso_graph_edge(n, "ctx", ctx), pso_graph_kind(ctx, "Store").
.decl pso_graph_print(n: node, text: symbol)
pso_graph_print(to_number(@slice(name, 1, strlen(name))), printed) :- option("entryPoint", "collector"),
 sig_param(_, _, nm, _, _, ann, _), text(nm, name), prefix("e", name), annotation_text(ann, printed).
pso_parameter(f, p, to_number(i), printed) :- pso_function(f, _), pso_graph_edge(f, "args", args), pso_graph_edge(args, "args", p),
 option_item("astOrdinals", p, i), pso_graph_edge(p, "annotation", ann), pso_graph_print(ann, printed).
pso_parameter(f, p, to_number(i), "") :- pso_function(f, _), pso_graph_edge(f, "args", args), pso_graph_edge(args, "args", p),
 option_item("astOrdinals", p, i), !pso_graph_edge(p, "annotation", _).
pso_method(c, f, name) :- pso_class(c, _), pso_graph_edge(c, "body", f), pso_function(f, name).
pso_build(c, st, printed) :- pso_class(c, _), pso_graph_edge(c, "body", st), pso_graph_kind(st, "AnnAssign"),
 pso_graph_edge(st, "target", t), pso_name(t, "build"), pso_graph_edge(st, "annotation", ann), pso_graph_print(ann, printed).
pso_call_print(c, printed) :- pso_graph_kind(c, "Call"), pso_graph_print(c, printed).
pso_callee_print(c, printed) :- pso_graph_kind(c, "Call"), pso_graph_edge(c, "func", fn), pso_graph_print(fn, printed).
'''
