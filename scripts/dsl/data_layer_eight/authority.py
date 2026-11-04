# SPDX-License-Identifier: Apache-2.0
"""Read-through authority ownership and closed-plan ordering."""
import ast,re
from data_layer_six.common import A,O,N,K,F,H,I,names,call,RX

ANY=RX('(?s).*')
def contains(rule):return O(rule,H(rule,stop='end'))
def symbol(values):
    return A(names(values),N(I(O(K('import_statement'),K('import_from_statement')))),
        *[N(I(K(kind),field=field,stop='neighbor')) for kind,field in [('function_definition','name'),('class_definition','name'),('keyword_argument','name')]],
        N(I(K('parameters'),stop='neighbor')))
def function(name,rule=ANY):return A(K('function_definition'),F('name',names([name])),rule)
def method_statement(name,rule=ANY):
    fn=function(name,rule)
    return O(fn,A(K('decorated_definition'),H(fn)))
def first_method(name,rule=ANY):
    return F('body',H(A(method_statement(name,rule),N({'follows':{**method_statement(name),'stopBy':'end'}}))))
def source_paths(source,name):
    for node in ast.parse(source.read_text()).body:
        targets=node.targets if isinstance(node,ast.Assign) else [node.target] if isinstance(node,ast.AnnAssign) else []
        if any(isinstance(t,ast.Name) and t.id==name for t in targets):
            return [item.right.value for item in node.value.elts]
    raise ValueError('Missing source paths '+name)

def candidate(source,policy):
    module=source/'scripts/lint_readthrough_authority_architecture.py'
    p=policy('readthrough_authority_architecture',source_paths(module,'DEFAULT_SOURCES'))
    p['clauses'].append({'id':'entry','define':{'options':{'oracleAnalyzer':{'type':'boolean','default':False},'entryPoint':{'type':'string','default':'scan_sources'}}}})
    trigger=O(A(K('module'),RX('ReadThrough|readthrough|overlay|authority')),I(A(K('module'),RX('ReadThrough|readthrough|overlay|authority'))))
    gate={'$option':'oracleAnalyzer','cases':{'true':ANY,'false':trigger}}
    def add(id,rule,code):
        p['clauses'].append({'id':id,'forbid':{'ast':{'language':'python','syntax':'valid','rule':A(gate,rule)}},
                             'violation':{'code':code,'message':code,'anchor':'witness.span'}})
    add('restore',call(['_restore_persisted_local_overlays','_restore_persisted_local_overlays_async']),'RTA001')
    add('runtime-seed',A(O(K('string'),K('concatenated_string')),{'value':{'eq':'__readthrough_local_overlays'}}),'RTA002')
    add('bind-load',A(K('function_definition'),F('name',names(['_bind_cache_to_current_authority','_bind_cache_to_current_authority_native'])),
                     H(call(['_load_local_row_overlays','_load_local_row_overlays_async']),stop='end')),'RTA001')
    add('cache-heuristics',function('_authority_transition_models',H(symbol(['_cached_tables','_tables_with_local_writes']),stop='end')),'RTA003')
    cls=lambda name:A(K('class_definition'),F('name',names([name])))
    annotated=A(K('expression_statement'),H(A(K('assignment'),F('left',names(['principal_user_id'])),F('type',ANY))))
    add('principal-field',A(cls('ReadThroughLocalOverlayRecord'),N(F('body',H(annotated)))),'RTA004')
    dependency=A(K('keyword_argument'),F('name',names(['after_refs'])),F('value',contains(symbol(['proposed_lease_ref']))))
    add('closed-dependency',function('_local_overlay_record_steps',N(H(dependency,stop='end'))),'RTA005')
    add('durable-ownership',A(cls('ReadThroughDeleteChangedPrincipalRowsStep'),N(first_method('compile',H(symbol(['ReadThroughAuthorityOwnedTableRecord']),stop='end')))),'RTA006')
    identity=['upstream_namespace','authority_namespace','authorization_epoch','catalog_fingerprint','admitted_session_id','principal_user_id']
    literals=[H(A(O(K('string'),K('concatenated_string')),{'value':{'eq':name}}),stop='end') for name in identity]
    add('complete-identity',A(cls('ReadThroughDeleteChangedPrincipalRowsStep'),N(first_method('compile',A(*literals)))),'RTA008')
    assignment=A(K('assignment'),F('left',names(['lease_conflict_update'])),N(F('type',ANY)),F('right',A(K('dictionary'),
        H(symbol(['last_authenticated']),stop='end'),N(H(symbol(['established_at']),stop='end')))))
    add('preserve-establishment',function('_remote_authority_lease_steps',N(H(assignment,stop='end'))),'RTA009')
    add('publication-precondition',A(K('function_definition'),F('name',names(['_lower_generic_cache_first_object','_lower_generic_rowset_merge'])),
                                  N(H(call(['require_publication_precondition']),stop='end'))),'RTA007')
    return p
