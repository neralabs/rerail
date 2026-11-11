"""State Tripwires helper semantics over caller-supplied AST columns."""


def program():
    return r'''
.decl sph_kind(n: number, k: symbol)
sph_kind(n,k) :- option_item("astKinds",n,k).
.decl sph_edge(p: number, field: symbol, ordinal: number, n: number)
sph_edge(to_number(p),f,to_number(i),n) :- option_item("astParents",n,p), p != "-1",
    option_item("astFields",n,f), option_item("astOrdinals",n,i).
.decl sph_ref(n: number, identity: number)
sph_ref(n,to_number(r)) :- option_item("helperAstRefs",n,r).
.decl sph_desc(p: number, n: number, depth: number)
sph_desc(n,n,0) :- sph_kind(n,_).
sph_desc(p,n,d+1) :- sph_desc(p,c,d), sph_edge(c,_,_,n), n > c.
.decl sph_function(n: number, name: symbol)
sph_function(n,s) :- sph_kind(n,k), (k="FunctionDef";k="AsyncFunctionDef"), option_item("astNames",n,s).
.decl sph_name(n: number, name: symbol)
sph_name(n,s) :- sph_kind(n,"Name"), option_item("astIds",n,s).
sph_name(n,s) :- sph_kind(n,"Attribute"), option_item("astAttrs",n,s).
.decl sph_decorator_name(n: number, name: symbol)
sph_decorator_name(n,s) :- sph_name(n,s).
sph_decorator_name(n,s) :- sph_kind(n,"Call"), sph_edge(n,"func",_,f), sph_decorator_name(f,s).
.decl sph_chain_last(n: number, name: symbol)
sph_chain_last(n,s) :- sph_name(n,s).
sph_chain_last(n,s) :- sph_kind(n,"Call"), sph_edge(n,"func",_,f), sph_chain_last(f,s).
.decl sph_chain_previous(n: number, name: symbol)
sph_chain_previous(n,s) :- sph_kind(n,"Attribute"), sph_edge(n,"value",_,v), sph_chain_last(v,s).
sph_chain_previous(n,s) :- sph_kind(n,"Call"), sph_edge(n,"func",_,f), sph_chain_previous(f,s).
.decl sph_forbidden(n: number)
sph_forbidden(n) :- sph_kind(n,"Call"), sph_edge(n,"func",_,f), sph_name(f,s), spt_native_name(s).
sph_forbidden(n) :- sph_kind(n,"Call"), sph_edge(n,"func",_,f), sph_chain_previous(f,p),
    sph_chain_last(f,s), spt_native_pair(p,s).
sph_forbidden(n) :- sph_kind(n,"Call"), sph_edge(n,"func",_,f), sph_kind(f,"Attribute"),
    option_item("astAttrs",f,s), spt_native_method(s).

// Collection follows module and class bodies only. JSON preserves each class
// stack component, including supplied names which contain periods or quotes.
.decl sph_scope(n: number, stack: symbol)
sph_scope(0,s) :- option("entryPoint","_collect_function_nodes"), option("helperClassStack",s).
sph_scope(c,cat(@sql_substr(s,1,@sql_length(s)-1),@json_str(name),"]")) :- sph_scope(p,s),
    sph_edge(p,"body",_,c), c > p, sph_kind(c,"ClassDef"), option_item("astNames",c,name), s="[]".
sph_scope(c,cat(@sql_substr(s,1,@sql_length(s)-1),",",@json_str(name),"]")) :- sph_scope(p,s),
    sph_edge(p,"body",_,c), c > p, sph_kind(c,"ClassDef"), option_item("astNames",c,name), s!="[]".
.violation StateCollectedFunction(at: span, occurrence: number, class_stack: symbol) code "SPT_HELPER_COLLECT" "function {occurrence}"
StateCollectedFunction(@place("input.py",1),n,s) :- sph_scope(p,s), sph_edge(p,"body",_,n), sph_function(n,_).

.decl sph_input(slot: number, n: number, identity: number)
sph_input(i,to_number(n),r) :- option_item("helperFunctionNodes",i,n), sph_ref(to_number(n),r).
.decl sph_repr(identity: number, n: number)
sph_repr(r,n) :- sph_input(_,_,r), n = min k : { sph_input(_,k,r) }.
.decl sph_cache_root(identity: number, root: number)
sph_cache_root(to_number(r),to_number(n)) :- option_item("helperCacheOwners",i,r), option_item("helperCacheRoots",i,n).
.decl sph_body(identity: number, n: number)
sph_body(r,n) :- sph_cache_root(r,root), sph_edge(root,"body",_,n).
sph_body(r,n) :- sph_repr(r,f), !sph_cache_root(r,_), sph_desc(f,n,_).
.decl sph_argument(f: number, arg: number, name: symbol)
sph_argument(f,a,name) :- sph_function(f,_), sph_edge(f,"args",_,args), sph_edge(args,field,_,a),
    (field="posonlyargs";field="args";field="kwonlyargs"), option_item("astArgNames",a,name).
.decl sph_argument_type(f: number, name: symbol)
sph_argument_type(f,name) :- sph_argument(f,a,_), sph_edge(a,"annotation",_,ann), sph_desc(ann,n,_), sph_name(n,name).
.decl sph_root(identity: number)
sph_root(r) :- sph_repr(r,f), sph_function(f,name), spt_planning_name(name).
sph_root(r) :- sph_repr(r,f), sph_function(f,name), suffix("_lazy",name).
sph_root(r) :- sph_repr(r,f), sph_edge(f,"decorator_list",_,d), sph_decorator_name(d,"pure_lazy").
sph_root(r) :- sph_repr(r,f), sph_function(f,"pending_pipeline_effects").
sph_root(r) :- sph_repr(r,f), sph_function(f,name), prefix("apply_",name), suffix("_effect",name).
sph_root(r) :- sph_repr(r,f), sph_function(f,name), sph_argument(f,_,"runtime_refs"), prefix("materialize_",name), suffix("builder",name).
sph_root(r) :- sph_repr(r,f), sph_argument(f,_,"runtime_refs"), sph_argument_type(f,"RuntimeBuilderSpec").
sph_root(r) :- sph_repr(r,f), sph_function(f,name), suffix("sql_builder",name), !spt_planning_name(name), sph_argument_type(f,"CompilationContext").
sph_root(r) :- sph_repr(r,f), sph_function(f,name), option("helperPath",p), spt_provider_path(p), spt_provider_token(t), contains(t,name).
sph_root(r) :- sph_repr(r,_), option("helperNativeProvided","true"), option_item("helperNativeRefs",_,raw), r=to_number(raw).
sph_root(r) :- sph_repr(r,_), option("helperNativeProvided","false"), spt_filter_path(0,p),
    prefix("impl/common/l2070_server_plan/",p), sph_body(r,n), sph_forbidden(n).
.decl sph_called(identity: number, name: symbol)
sph_called(to_number(r),s) :- option("helperCalledProvided","true"), option_item("helperCallerRefs",i,r), option_item("helperCalledNames",i,s).
sph_called(r,s) :- option("helperCalledProvided","false"), sph_body(r,c), sph_kind(c,"Call"), sph_edge(c,"func",_,f), sph_name(f,s).
.decl sph_selected(identity: number)
sph_selected(r) :- sph_root(r).
sph_selected(t) :- sph_selected(r), sph_called(r,s), sph_repr(t,f), sph_function(f,s).
.decl sph_last_slot(identity: number, slot: number)
sph_last_slot(r,i) :- sph_input(_,_,r), i = max j : { sph_input(j,_,r) }.
.violation StateSelectedFunction(at: span, ordinal: number, class_stack: symbol) code "SPT_HELPER_SELECT" "selected function {ordinal}"
StateSelectedFunction(@place("input.py",1),i,stack) :- option("entryPoint","_select_reachable_function_nodes"),
    sph_input(i,_,r), sph_selected(r), sph_last_slot(r,j), option_item("helperClassStacks",j,stack).
.decl sph_walk_requested(identity: number)
sph_walk_requested(r) :- option("entryPoint","_select_reachable_function_nodes"), sph_repr(r,_),
    option("helperNativeProvided","false"), spt_filter_path(0,p), prefix("impl/common/l2070_server_plan/",p).
sph_walk_requested(r) :- option("entryPoint","_select_reachable_function_nodes"), sph_selected(r), option("helperCalledProvided","false").
.violation StateHelperCache(at: span, owner: number, occurrence: number, depth: number) code "SPT_HELPER_CACHE" "cached AST {occurrence}"
StateHelperCache(@place("input.py",1),r,n,d) :- sph_walk_requested(r), !sph_cache_root(r,_), sph_repr(r,f), sph_desc(f,n,d).
'''
