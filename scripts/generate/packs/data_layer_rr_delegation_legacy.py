"""Legacy decorator triples, including the source's final-name rebinding fold."""


def legacy_relations():
    return r'''
.decl asdl_class_candidate(c: node, module: node, name: symbol, i: number)
asdl_class_candidate(c, module, @nfkc(name), i) :- kind(module, "module"), body_stmt(module, stmt, i), defstmt(c, stmt), kind(c, "class_definition"), def_name(c, name).
.decl asdl_scope(c: node, owner: symbol, scope_order: number)
asdl_scope(m, "<module>", 0) :- kind(m, "module").
asdl_scope(c, owner, first+1) :- asdl_class_candidate(c, m, owner, i), i = max j : { asdl_class_candidate(_, m, owner, j) }, first = min j : { asdl_class_candidate(_, m, owner, j) }.
.decl asdl_function_candidate(c: node, f: node, name: symbol, i: number)
asdl_function_candidate(c, f, name, i) :- asdl_scope(c, _, _), body_of(c, b), body_stmt(b, stmt, i), defstmt(f, stmt), asdp_function(f, _, name).
.decl asdl_function(c: node, f: node, name: symbol, first: number)
asdl_function(c, f, name, first) :- asdl_function_candidate(c, f, name, i), i = max j : { asdl_function_candidate(c, _, name, j) }, first = min j : { asdl_function_candidate(c, _, name, j) }.
.decl asdl_kind(f: node, decorator: symbol, priority: number)
asdl_kind(f, "legacy_sync", 0) :- asdp_decorator(f, d, "legacy_sync"), !kind(d, "call").
asdl_kind(f, "legacy_async", 1) :- asdp_decorator(f, d, "legacy_async"), !kind(d, "call").
asdl_kind(f, "legacy_module_sync", 2) :- asdp_decorator(f, d, "legacy_module_sync"), !kind(d, "call").
asdl_kind(f, "legacy_module_async", 3) :- asdp_decorator(f, d, "legacy_module_async"), !kind(d, "call").
.decl asdl_selected(f: node, decorator: symbol)
asdl_selected(f, decorator) :- asdl_kind(f, decorator, p), p = min q : { asdl_kind(f, _, q) }.
.decl asdl_base(f: node, decorator: symbol, base: symbol)
asdl_base(f, "legacy_sync", @replace_all(name, "_sync\\z", "")) :- asdl_selected(f, "legacy_sync"), asdp_function(f, _, name).
asdl_base(f, "legacy_async", @replace_all(name, "_async\\z", "")) :- asdl_selected(f, "legacy_async"), asdp_function(f, _, name).
asdl_base(f, "legacy_module_sync", name) :- asdl_selected(f, "legacy_module_sync"), asdp_function(f, _, name).
asdl_base(f, "legacy_module_async", @replace_all(name, "_async\\z", "")) :- asdl_selected(f, "legacy_module_async"), asdp_function(f, _, name).
.decl asdl_keyword(d: node, value: symbol, i: number)
asdl_keyword(d, value, i) :- kind(d, "call"), field(d, "arguments", args), child(args, kw, i), kind(kw, "keyword_argument"), field(kw, "name", nm), text(nm, raw), @nfkc(raw) = "lazy_name", field(kw, "value", rawvalue), unparen(rawvalue, v), str_const(v, value).
.decl asdl_override(f: node, decorator: symbol, value: symbol)
asdl_override(f, decorator, value) :- asdl_selected(f, decorator), asdp_decorator(f, d, decorator), visit_index(d, i), i = min j : { asdp_decorator(f, other, decorator), visit_index(other, j) }, asdl_keyword(d, value, k), k = min j : { asdl_keyword(d, _, j) }, value != "".
.decl asdl_lazy_name(f: node, name: symbol)
asdl_lazy_name(f, value) :- asdl_override(f, _, value).
asdl_lazy_name(f, cat(base, "_lazy")) :- asdl_base(f, decorator, base), !asdl_override(f, decorator, _).
.decl asdl_requirement(f: node, base: symbol, lazy: symbol, counterpart: symbol, decorator: symbol)
asdl_requirement(f, base, lazy, cat(base, "_async"), "legacy_async") :- asdl_base(f, "legacy_sync", base), asdl_lazy_name(f, lazy).
asdl_requirement(f, base, lazy, base, "legacy_sync") :- asdl_base(f, "legacy_async", base), asdl_lazy_name(f, lazy).
asdl_requirement(f, base, lazy, cat(base, "_async"), "legacy_module_async") :- asdl_base(f, "legacy_module_sync", base), asdl_lazy_name(f, lazy).
asdl_requirement(f, base, lazy, base, "legacy_module_sync") :- asdl_base(f, "legacy_module_async", base), asdl_lazy_name(f, lazy).
// Only ast.Assign with exactly one ast.Name target participates. Other
// assignments do not clear an earlier simple-name binding in the source.
.decl asdl_assignment(c: node, name: symbol, value: node, i: number)
asdl_assignment(c, @nfkc(name), value, i) :- asdl_scope(c, _, _), body_of(c, b), body_stmt(b, stmt, i), asgstmt(a, stmt), !field(a, "type", _), count : { assign_target(a, _) } = 1, assign_target(a, raw), unparen(raw, target), kind(target, "identifier"), text(target, name), assign_value(a, rawvalue), unparen(rawvalue, value).
.decl asdl_rebinding_candidate(c: node, name: symbol, target: symbol, i: number)
asdl_rebinding_candidate(c, name, @nfkc(target), i) :- asdl_assignment(c, name, value, i), kind(value, "identifier"), text(value, target).
.decl asdl_rebinding(c: node, name: symbol, target: symbol)
asdl_rebinding(c, name, target) :- asdl_rebinding_candidate(c, name, target, i), i = max j : { asdl_rebinding_candidate(c, name, _, j) }.
.decl asdl_reach(c: node, start: symbol, end: symbol)
asdl_reach(c, a, b) :- asdl_rebinding(c, a, b).
asdl_reach(c, a, z) :- asdl_reach(c, a, b), asdl_rebinding(c, b, z).
.decl asdl_cycle(c: node, name: symbol)
asdl_cycle(c, name) :- asdl_reach(c, name, name).
.decl asdl_walk(c: node, root: symbol, name: symbol)
asdl_walk(c, root, root) :- asdl_function(c, f, _, _), asdl_requirement(f, _, root, _, _).
asdl_walk(c, root, next) :- asdl_walk(c, root, name), asdl_rebinding(c, name, next).
.decl asdl_final(c: node, root: symbol, final: symbol)
asdl_final(c, root, final) :- asdl_walk(c, root, final), !asdl_rebinding(c, final, _).
asdl_final(c, root, root) :- asdl_walk(c, root, root), asdl_cycle(c, root).
asdl_final(c, root, next) :- asdl_walk(c, root, name), !asdl_cycle(c, name), asdl_rebinding(c, name, next), asdl_cycle(c, next).
.decl asdl_final_lazy(c: node, root: symbol, f: node)
asdl_final_lazy(c, root, f) :- asdl_final(c, root, name), asdl_function(c, f, name, _).
.decl asdl_non_execution_lazy(c: node, root: symbol)
asdl_non_execution_lazy(c, root) :- asdl_final_lazy(c, root, f), asds_lazy_decorated(f), !asds_execution_lazy(f).
.decl asdl_builder(f: node, base: symbol)
asdl_builder(f, base) :- asdl_requirement(f, base, _, _, _), (base = "query" ; base = "query_unfiltered"), asds_return(f, ann), contains("Query", ann).
.decl asdl_target(c: node, f: node, owner: symbol, target: symbol, base: symbol, lazy: symbol, counterpart: symbol, decorator: symbol)
asdl_target(c, f, owner, name, base, lazy, counterpart, decorator) :- asdl_scope(c, owner, _), owner = "<module>", asdl_function(c, f, name, _), asdl_requirement(f, base, lazy, counterpart, decorator), asdp_function(f, path, _), !asdp_test_path(path).
asdl_target(c, f, owner, cat(owner, ".", name), base, lazy, counterpart, decorator) :- asdl_scope(c, owner, _), owner != "<module>", asdl_function(c, f, name, _), asdl_requirement(f, base, lazy, counterpart, decorator), !asdl_builder(f, base), !asdl_non_execution_lazy(c, lazy), asdp_function(f, path, _), !asdp_test_path(path).
.decl asdl_local_required(c: node)
asdl_local_required(c) :- asdl_scope(c, owner, _), (owner = "<module>" ; suffix("Connection", owner) ; suffix("SessionHandle", owner) ; suffix("VersionHandle", owner)).
.decl asdl_local_lazy(c: node, name: symbol)
asdl_local_lazy(c, name) :- asdl_final_lazy(c, name, f), asds_lazy_decorated(f).
.decl asdl_final_detail(c: node, name: symbol, text: symbol)
asdl_final_detail(c, name, "") :- asdl_final(c, name, name).
asdl_final_detail(c, name, cat(" final_binding=", @python_repr(final), ".")) :- asdl_final(c, name, final), name != final.
.decl asdl_counter_scope(c: node, other: node)
asdl_counter_scope(c, other) :- asdl_scope(c, owner, _), prefix("Sync", owner), counterpart = cat("Async", @slice(owner, 4, strlen(owner))), asdl_scope(other, counterpart, _), docof(c, d), docof(other, d).
asdl_counter_scope(c, other) :- asdl_scope(c, owner, _), prefix("Async", owner), counterpart = cat("Sync", @slice(owner, 5, strlen(owner))), asdl_scope(other, counterpart, _), docof(c, d), docof(other, d).
.decl asdl_counterpart(c: node, name: symbol, scope: node, f: node)
asdl_counterpart(c, name, c, f) :- asdl_function(c, f, name, _).
asdl_counterpart(c, name, other, f) :- asdl_counter_scope(c, other), asdl_function(other, f, name, _), !asdl_function(c, _, name, _).
.decl asdl_assignment_decorator(c: node, name: symbol, decorator: symbol)
asdl_assignment_decorator(c, name, decorator) :- asdl_assignment(c, name, value, _), kind(value, "call"), field(value, "function", rawfn), unparen(rawfn, fn), asdp_callable(fn, decorator), call_arg(value, 0, "", rawarg), unparen(rawarg, arg), kind(arg, "identifier"), text(arg, argument), @nfkc(argument) = name.
.decl asdl_paired(c: node, name: symbol, decorator: symbol)
asdl_paired(c, name, decorator) :- asdl_counterpart(c, name, _, f), asdp_decorator(f, _, decorator).
asdl_paired(c, name, decorator) :- asdl_counterpart(c, name, scope, _), asdl_assignment_decorator(scope, name, decorator).
.decl asdl_scope_max(d: number, size: number)
asdl_scope_max(d, size) :- asdl_scope(c, _, _), docof(c, d), size = max n : { asdl_scope(s, _, _), docof(s, d), body_of(s, b), body_len(b, n) }.
.decl asdl_ordinal(c: node, f: node, rank: number)
asdl_ordinal(c, f, (scope_order*(size+1)+first)*2) :- asdl_scope(c, _, scope_order), asdl_function(c, f, _, first), docof(c, d), asdl_scope_max(d, size).
ASDAnalysis(@place(path, line), target, "C", cat("Legacy wrapper ", target, " does not resolve through a local ", lazy, "() lazy surface"), "", cat("legacy_base=", @python_repr(base), "; expected_lazy=", @python_repr(lazy), ".", detail, " Every @legacy_* wrapper must resolve through a sibling *_lazy() function decorated with @lazy or @pure_lazy, and the final binding of that lazy symbol must stay lazy."), 0, 1, 4, rank) :- asdl_target(c, f, _, target, base, lazy, _, _), asdl_local_required(c), !asdl_local_lazy(c, lazy), asdl_final_detail(c, lazy, detail), asdp_function(f, path, _), line(f, line), asdl_ordinal(c, f, rank).
ASDAnalysis(@place(path, line), target, "C", cat("Legacy wrapper ", target, " is missing the required ", counterpart, "() decorated pair"), "", cat("legacy_base=", @python_repr(base), "; expected_counterpart=", @python_repr(counterpart), "; expected_decorator=", decorator, ". Legacy wrappers must always ship as sync/async pairs over one shared lazy family."), 0, 1, 4, rank+1) :- asdl_target(c, f, _, target, base, _, counterpart, decorator), !asdl_paired(c, counterpart, decorator), asdp_function(f, path, _), line(f, line), asdl_ordinal(c, f, rank).
'''
