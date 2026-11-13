#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate fifth-batch policies using verified pinned upstream constants."""
from __future__ import annotations
import argparse
import ast
import importlib
import json
from pathlib import Path
from collections import Counter
import re
import sys
from data_layer_batch_five import MODULES
from data_layer_probe_record import verify_sources

PIN='2b1504adc4291daaf080b2e932962b134da1a69a'
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--source',type=Path,required=True);p.add_argument('--repo',type=Path,required=True)
p.add_argument('--output',type=Path,default=Path('packs/data-layer/candidates/rules'))
args=p.parse_args();sys.path.insert(0,str(args.source.resolve()));sys.path.insert(1,str(args.source.resolve()/'scripts'))
modules={name:importlib.import_module(module) for name,module in MODULES.items()}
from scripts.lint_lib.discovery import data_layer_production_dirs
production_roots=[directory+'/**/*.py' for directory in ['api',*data_layer_production_dirs(args.source.resolve())]]
verify_sources(args.source.resolve(),args.repo,PIN);args.output.mkdir(parents=True,exist_ok=True)

def A(*rules):return {'all':list(rules)}
def O(*rules):return {'any':list(rules)}
def N(rule):return {'not':rule}
def K(kind):return {'kind':kind}
def RX(pattern):return {'regex':pattern}
def P(name):return {'pattern':'$'+name}
def H(rule,field=None,stop='neighbor'):return {'has':{**rule,'stopBy':stop,**({'field':field} if field else {})}}
def I(rule,stop='end'):return {'inside':{**rule,'stopBy':stop}}
def F(field,rule):return H(rule,field)
def M(name,**kw):return {'matches':{name:{k:'$'+v for k,v in kw.items()}}} if kw else {'matches':name}
def names(items):return A(K('identifier'),RX('^(?:'+'|'.join(re.escape(x) for x in sorted(items))+')$' if items else r'\b\B'))
def eq(left,right,text=None,op='eq'):return {'compare':{'left':left,'right':right,'op':op,**({'text':text} if text else {})}}
def fn(name=None,async_=None):return A(K('function_definition'),*([] if name is None else [F('name',name)]),*([] if async_ is None else [RX(r'^async\b' if async_ else r'^def\b')]))
def glob_regex(pattern):
 tokens=re.findall(r'\*\*/|\*\*|\*|\?|\[[^\]]+\]|.',pattern)
 return '^'+''.join('(?:.*/)?' if t=='**/' else '.*' if t=='**' else '[^/]*' if t=='*' else '[^/]' if t=='?' else t if t.startswith('[') else re.escape(t) for t in tokens)+'$'
def C(id,rule,code,each=None,anchor='witness.span',paths=None,transform=None):
 q={'language':'python','syntax':'valid','rule':rule}
 if paths:q['file']={'path':{**({'regex':'|'.join(glob_regex(p) for p in paths['include'])} if paths.get('include') else {}),
                           **({'notRegex':'|'.join(glob_regex(p) for p in paths['exclude'])} if paths.get('exclude') else {})}}
 if transform:q['transform']=transform
 return {'id':id,'forbid':{'ast':q},'violation':{'code':code,'anchor':anchor,'message':code+': '+id,**({'each':each} if each else {})}}
def W(name,clauses,utils=None,options=None,roots=None,mode='block',exclude=None):
 if name in ('async_sync_delegation','public_api_triples','internal_dead_code'):
  # Repeated rule expressions share authored utilities and native memo entries.
  frequencies=Counter()
  rule_keys={'all','any','not','has','inside','precedes','follows','matches','kind','regex','pattern','compare','count','filePath','value','arguments','layout'}
  def collect(value):
   if isinstance(value,dict):
    if set(value)<=rule_keys and set(value)&rule_keys:
     key=json.dumps(value,sort_keys=True,separators=(',',':'))
     if len(key)>=(1500 if name=='async_sync_delegation' else 300):frequencies[key]+=1
    for child in value.values():collect(child)
   elif isinstance(value,list):
    for child in value:collect(child)
  collect(clauses);collect(utils)
  shared={};definitions={}
  def factor(value,root=False):
   if isinstance(value,dict):
    key=json.dumps(value,sort_keys=True,separators=(',',':'))
    if not root and frequencies[key]>1:
     if key not in shared:
      shared[key]='b5-common-'+str(len(shared)+1)
      definitions[shared[key]]=factor(value,True)
     return M(shared[key])
    return {k:factor(v) for k,v in value.items()}
   if isinstance(value,list):return [factor(v) for v in value]
   return value
  clauses=factor(clauses)
  utils={**(factor(utils) or {}),**definitions}
 definition={'options':{'oracleAnalyzer':{'type':'boolean','default':False},'sourceGate':{'type':'boolean','default':True},**(options or {})}}
 if utils:definition.update(language='python',utils=utils)
 document={'format':'rerail.policy/v1','id':'data-layer-'+name.replace('_','-'),'revision':1,'mode':mode,
   'description':f'Candidate port of nera-data-layer {name} at {PIN}.',
   'scope':{'include':roots or ['**/*.py'],'exclude':exclude if exclude is not None else ['**/.git/**','**/.venv/**','**/tmp/**','**/__pycache__/**','**/tests/**']},
   'clauses':[{'id':'shared','define':definition},*clauses]}
 triggers=next((getattr(v,'source_triggers',()) for v in vars(modules[name]).values()
                if getattr(v,'extractor_id','').startswith(name+'.') and getattr(v,'source_triggers',())),())
 if triggers:
  gate={'$option':'sourceGate','cases':{'true':'|'.join(re.escape(x) for x in triggers),'false':r'(?s).*'}}
  for clause in clauses:
   query=clause.get('forbid',{}).get('ast') or clause.get('budget',{}).get('query',{}).get('evidence',{}).get('query',{}).get('ast')
   if query is not None:query['rule']=A(query['rule'],O(A(K('module'),RX(gate)),I(A(K('module'),RX(gate)))))
 (args.output/(document['id']+'.json')).write_text(json.dumps(document,indent=2)+'\n')

TOP=O(I(K('module'),'neighbor'),I(A(K('decorated_definition'),I(K('module'),'neighbor')),'neighbor'))
MODULE_SCOPE=N(I(O(K('function_definition'),K('class_definition'))))
CONST=O(K('integer'),K('float'),K('true'),K('false'),K('none'),K('ellipsis'),
        A(O(K('string'),K('concatenated_string')),N(RX(r'(?i)^[rub]*f')),N(H(A(K('string'),RX(r'(?i)^[rub]*f'))))))
utils={}
dictionary_item=O(
 A(K('pair'),F('key',M('simple-value')),F('value',M('simple-value'))),
 A(K('dictionary_splat'),H(M('simple-value'))),
)
utils['simple-value']=O(
 CONST,K('identifier'),A(K('attribute'),F('object',M('simple-value'))),
 A(K('parenthesized_expression'),H(M('simple-value'))),
 A(O(K('list'),K('tuple'),K('set')),N(H(A(P('_'),N(K('comment')),N(M('simple-value')))))),
 A(K('dictionary'),N(H(A(P('_'),N(K('comment')),N(dictionary_item))))),
)
utils['simple-assignment']=A(K('assignment'),F('left',K('identifier')),N(F('right',N(O(M('simple-value'),M('simple-assignment'))))))
assignment=O(K('assignment'),K('augmented_assignment'))
expression=A(K('expression_statement'),I(K('module'),'neighbor'))
import_module=O(A(K('import_from_statement'),F('module_name',RX(r'^impl\.'))),
 A(K('import_statement'),H(O(A(K('dotted_name'),RX(r'^impl\.'),P('IMPORT')),A(K('aliased_import'),F('name',A(RX(r'^impl\.'),P('IMPORT'))))))))
clauses=[C('definition',A(O(K('function_definition'),K('class_definition')),TOP),'FAC001'),
 C('impl-from',A(K('import_from_statement'),F('module_name',RX(r'^impl\.')),TOP),'FAC003'),
 C('impl-import',A(K('import_statement'),TOP,H(O(A(K('dotted_name'),RX(r'^impl\.'),P('IMPORT')),A(K('aliased_import'),F('name',A(RX(r'^impl\.'),P('IMPORT'))))))),'FAC003',each='IMPORT'),
 C('assignment',A(expression,H(A(assignment,N(M('simple-assignment'))))),'FAC004'),
 C('expression',A(expression,N(H(assignment)),N(H(A(O(K('string'),K('concatenated_string')),{'value':{'type':'str'}})))),'FAC002'),
 C('statement',A(TOP,N(O(K('comment'),K('expression_statement'),K('import_statement'),K('import_from_statement'),K('future_import_statement'),K('function_definition'),K('class_definition'),K('decorated_definition')))),'FAC002')]
m=modules['data_layer_facade_purity']
W('data_layer_facade_purity',clauses,utils,roots=[p.relative_to(m.REPO_ROOT).as_posix() for p in m.DEFAULT_TARGETS])

public=A(K('identifier'),N(RX('^_')))
alias=A(K('aliased_import'),F('alias',A(public,P('ALIAS'))),F('name',P('SOURCE')),eq('ALIAS','LEAF',op='ne'))
transform={'LEAF':{'replace':{'source':'$SOURCE','replace':r'^.*\.','by':''}}}
utils={'alias-value':O(A(K('identifier'),P('SOURCE')),A(K('attribute'),F('attribute',A(K('identifier'),P('SOURCE'))))),
       'final-alias':O(M('alias-value'),A(K('assignment'),F('right',M('final-alias'))))}
clauses=[C('import-alias',A(O(K('import_statement'),K('import_from_statement')),MODULE_SCOPE,H(A(alias,P('ALIAS_NODE')))),'DLA001',each='ALIAS_NODE',transform=transform),
 C('assignment-alias',A(K('assignment'),MODULE_SCOPE,F('left',A(public,P('TARGET'))),F('right',M('final-alias')),eq('TARGET','SOURCE',op='ne')),'DLA002')]
W('data_layer_compat_aliases',clauses,utils,roots=['data_layer/**/*.py'])

# Import checks intentionally inspect only the first alias in plain imports.
m=modules['backend_coupling']
first={'nthChild':{'position':1,'ofRule':N(K('comment'))}}
imported=O(A(K('import_from_statement'),F('module_name',P('MOD'))),
 A(K('import_statement'),H(A(O(A(K('dotted_name'),P('MOD')),A(K('aliased_import'),F('name',P('MOD')))),first))))
typed=N(I(A(K('if_statement'),F('condition',names(['TYPE_CHECKING'])))))
registry=[r'**/*'+p for p in sorted(m.REGISTRATION_FILES)]
backend='^(?:'+'|'.join(re.escape(v) for v in sorted(m.BACKEND_PREFIXES))+')'
sql='(?:'+'|'.join(re.escape('impl.common.'+v) for v in sorted(m.SQL_SPECIFIC_DIRS))+')'
publicpaths=['api/**/*.py','data_layer/**/*.py','_internal/**/*.py','l[0-9][0-9][0-9][0-9]_*/**/*.py','impl/common/**/*.py']
common=A(imported,typed)
def module_matches(pattern):return F('module_name',RX(pattern))
def import_test(pattern):return O(A(K('import_from_statement'),module_matches(pattern)),A(K('import_statement'),H(A(O(A(K('dotted_name'),RX(pattern)),A(K('aliased_import'),F('name',RX(pattern)))),first))))
clauses=[C('backend-import',A(common,import_test(backend)),'BC001',paths={'include':publicpaths,'exclude':registry+['_internal/dynamic_import.py','api/**/__init__.py','data_layer/**/__init__.py','_internal/**/__init__.py','l*/**/__init__.py','impl/common/**/test_*.py']}),
 C('sql-in-nonsql',A(common,import_test(sql)),'BC001',paths={'include':['impl/mongodb*/**/*.py','impl/neo4j*/**/*.py'],'exclude':registry})]
W('backend_coupling',clauses,mode='log',roots=['api/**/*.py','data_layer/**/*.py','_internal/**/*.py','impl/**/*.py','l*/**/*.py'])

def local_literal(module, function, variable):
 tree=ast.parse(Path(module.__file__).read_text())
 body=next(node for node in tree.body if isinstance(node,ast.FunctionDef) and node.name==function)
 for node in ast.walk(body):
  if isinstance(node,ast.Assign) and any(isinstance(t,ast.Name) and t.id==variable for t in node.targets):return ast.literal_eval(node.value)
 raise ValueError((function,variable))

m=modules['layer_violations']
imports=[A(K('import_from_statement'),F('module_name',P('MOD')),typed),
 A(O(A(K('dotted_name'),P('MOD')),A(K('aliased_import'),F('name',P('MOD')))),I(K('import_statement'),'neighbor'),typed)]
layer_excludes=['**/tests/**','**/l0500_ts_models/**','**/l2070_server_plan/**','**/l3000_schema_builder/**',
 *['**/l7200_versioned_transactions/**/'+n for n in ('versioned_lazy_mixin.py','versioned_transactions.py','plan_revision_history_query.py')]]
clauses=[]
for idx,rule in enumerate(imports):
 q={'language':'python','syntax':'valid','rule':A(rule,P('EDGE')),
    'file':{'path':{'regex':r'(?:^|/)(?:api|impl)/','notRegex':'|'.join(glob_regex(x) for x in layer_excludes)}}}
 clauses.append({'id':'layer-'+str(idx),'budget':{'query':{'evidence':{'as':'edge','query':{'ast':q},
   'project':{'path':'file.path','start':'capture.EDGE.startByte',
    'from':{'source':'file.path','regex':r'(?:^|/)l([0-9]{4})_','group':1},
    'to':{'source':'capture.MOD','regex':r'\.l([0-9]{4})_','group':1},
    'grammar':{'source':'file.path','regex':r'(?:^|/)(l2100_sql_grammar)/','group':1}},
   'where':{'all':[{'lt':['edge.from','edge.to']},{'not':{'all':[{'eq':['edge.grammar','l2100_sql_grammar']},{'eq':['edge.to','2200']}]}}]}}},
   'count':{'lte':0},'group_by':['edge.path','edge.start']},
   'violation':{'code':'layer_dependency','anchor':'witness.span','message':'Import from a higher layer'}})
bridges=local_literal(m,'check_api_impl_separation','allowed_api_impl_bridges')
api_excludes=['**/__init__.py','**/l9900_ts_models/**','**/api/**/tests/**','**/l8000_admin/**/tools/**',
 '**/l2000_connection/**/connection_factory.py','**/l4200_event_pipeline/**/bridges/**','**/l3200_metadata/**',
 *['**/'+(layer+'/**/' if layer else '')+name for name,layer in sorted(bridges)]]
whitebox=local_literal(m,'check_conform_imports','whitebox_test_files')
for idx,rule in enumerate(imports):
 for code,include,exclude in [('api_impl',['**/api/**/*.py'],api_excludes),
                             ('conform_impl',['**/conform/**/*.py'],['**/conftest.py',*['**/'+n for n in sorted(whitebox)]])]:
  clauses.append(C(code+'-'+str(idx),A(rule,O(F('module_name',RX(r'^impl\.')),F('name',RX(r'^impl\.')),A(K('dotted_name'),RX(r'^impl\.')))),code,
                   paths={'include':include,'exclude':exclude}))
W('layer_violations',clauses,mode='log',roots=['api/**/*.py','impl/**/*.py','conform/**/*.py'],exclude=['**/.git/**','**/.venv/**','**/tmp/**','**/__pycache__/**'])

def positional_utils():
 normal=N(O(K('list_splat_pattern'),K('dictionary_splat_pattern'),A(K('typed_parameter'),H(O(K('list_splat_pattern'),K('dictionary_splat_pattern'))))))
 nonself=O(A(K('identifier'),N(names(['self']))),
  A(K('typed_parameter'),H(A(K('identifier'),N(names(['self']))))),
  A(O(K('default_parameter'),K('typed_default_parameter')),F('name',A(K('identifier'),N(names(['self']))))))
 return {'pos-param':A(nonself,normal,I(K('parameters'),'neighbor'),
  N({'precedes':{**K('positional_separator'),'stopBy':'end'}}),
  N({'follows':{**O(K('keyword_separator'),K('list_splat_pattern'),A(K('typed_parameter'),H(K('list_splat_pattern')))),'stopBy':'end'}}))}

def bind_param(name):
 return O(A(K('identifier'),P(name)),A(K('typed_parameter'),H(A(K('identifier'),P(name)))),
          A(O(K('default_parameter'),K('typed_default_parameter')),F('name',P(name))))

m=modules['sync_async_signatures'];utils=positional_utils()
interesting=set(m.TRANSACTION_TYPES)|set(m.CONNECTION_TYPES)
for label,values in [('interesting',interesting),('txn',m.TRANSACTION_TYPES),('conn',m.CONNECTION_TYPES)]:
 leaf=O(names(values),A(K('attribute'),F('attribute',names(values))),
        A(O(K('string'),K('concatenated_string')),{'value':{'regex':'^(?:'+'|'.join(re.escape(x) for x in sorted(values))+')$'}}))
 # Subscripts use their outer constructor. Union annotations prefer the left interesting type.
 utils['annotation-'+label]=O(leaf,
  A(K('type'),H(M('annotation-'+label))),
  A(K('generic_type'),H(A(M('annotation-'+label),{'nthChild':{'position':1,'ofRule':P('_')}}))),
  A(K('subscript'),F('value',M('annotation-'+label))),
  A(K('binary_operator'),O(F('left',M('annotation-'+label)),
      A(N(F('left',M('annotation-interesting'))),F('right',M('annotation-'+label))))))
first=A(M('pos-param'),N({'follows':{**M('pos-param'),'stopBy':'end'}}))
utils['has-param']=F('parameters',H(M('pos-param')))
utils['later-signature']={'params':['NAME','CURRENT'],'rule':I(A(K('module'),H(A(fn(),M('has-param'),F('name',P('NAME')),
 {'layout':{'before':{'to':'capture.CURRENT','breaks':{'min':0}}}}),stop='end')))}
def selected(name,node):return A(fn(),P(node),F('name',P(name)),M('has-param'),N(M('later-signature',NAME=name,CURRENT=node)))
async_sig=A(selected('ASYNC','ASYNC_NODE'),F('name',RX('_async$')),
 F('parameters',H(A(first,F('type',M('annotation-conn'))))))
sync_sig=A(selected('SYNC_NAME','SYNC_NODE'),eq('SYNC_NAME','BASE'),F('parameters',H(A(first,F('type',M('annotation-txn'))))))
W('sync_async_signatures',[C('transaction-connection',A(async_sig,I(A(K('module'),H(sync_sig,stop='end')))),
 'sync_async_signature_mismatch',transform={'BASE':{'replace':{'source':'$ASYNC','replace':'_async$','by':''}}})],utils,
 roots=production_roots,mode='log')

m=modules['missing_async_pairs'];utils=positional_utils()
owner=I(A(K('class_definition'),F('name',P('GROUP'))),K('class_definition'))
recordable=A(fn(),N(I(fn())),owner,N(F('name',names(m.EXEMPT_METHODS))),N(F('name',RX('_lazy$'))),
 N(F('name',RX('^(?:'+'|'.join(re.escape(x) for x in sorted(m.EXEMPT_METHODS))+')_async$'))),
 O(N(F('name',RX('^_'))),F('name',RX('_async$')),F('name',RX('|'.join(re.escape(x) for x in sorted(m.REQUIRED_PAIR_METHODS))))))
for label,is_async in [('sync',False),('async',True)]:
 utils['later-'+label]={'params':['GROUP','NAME','CURRENT'],'rule':I(A(K('module'),H(A(recordable,fn(async_=is_async),F('name',P('OTHER')),
  eq('OTHER','NAME'),{'layout':{'before':{'to':'capture.CURRENT','breaks':{'min':0}}}}),stop='end')))}
 def selected_pair(label,is_async):
  return A(recordable,fn(async_=is_async),P('METHOD'),F('name',P('NAME')),
   N(M('later-'+label,GROUP='GROUP',NAME='NAME',CURRENT='METHOD')))
 utils['selected-'+label]=selected_pair(label,is_async)

eligible=O(N(RX('^_')),names(m.REQUIRED_PAIR_METHODS))
owner_allowed=I(A(K('class_definition'),N(F('name',names(m.EXEMPT_CLASSES)))),K('class_definition'))
hasio=O(F('parameters',H(A(M('pos-param'),bind_param('IO_PARAM'),O(names(m._IO_PARAM_NAMES),H(names(m._IO_PARAM_NAMES)))))),
 H(A(K('call'),F('function',A(K('attribute'),F('attribute',names(set(m._IO_METHOD_NAMES)|{'execute','execute_async'}))))),stop='end'))
transform={'BASE':{'replace':{'source':'$NAME','replace':'_async$','by':''}},
 'ASYNC_TARGET':{'replace':{'source':'$NAME','replace':'$','by':'_async'}}}
def counterpart(is_async):
 return A(recordable,fn(async_=is_async),
  F('name',P('COUNTERPART_NAME')),
  O(eq('COUNTERPART_NAME','ASYNC_TARGET'),A(eq('COUNTERPART_NAME','NAME'),N(F('name',RX('_async$'))))) if is_async else eq('COUNTERPART_NAME','BASE'))
def anywhere(rule):return I(A(K('module'),H(rule,stop='end')))
clauses=[C('missing-async',A(M('selected-sync'),owner_allowed,F('name',eligible),hasio,N(anywhere(counterpart(True)))),
 'missing_pair',transform=transform),
 C('missing-sync',A(M('selected-async'),owner_allowed,O(N(F('name',RX(r'^_'))),F('name',names({x+'_async' for x in m.REQUIRED_PAIR_METHODS}))),hasio,N(anywhere(counterpart(False)))),
 'missing_pair',transform=transform)]
count=lambda rel,rule,binding:{'count':{rel:{**rule,'stopBy':'end'},'max':1000000,'as':binding}}
otherparam=A(M('pos-param'),bind_param('OTHER_PARAM'),count('follows',M('pos-param'),'OTHER_INDEX'),eq('OTHER_INDEX','INDEX'),eq('OTHER_PARAM','PARAM'))
unmatched=A(M('pos-param'),bind_param('PARAM'),count('follows',M('pos-param'),'INDEX'),
 N(anywhere(A(counterpart(True),F('parameters',H(otherparam))))))
other=A(counterpart(True),F('parameters',count('has',M('pos-param'),'ASYNC_COUNT')),
 O(eq('SYNC_COUNT','ASYNC_COUNT',op='ne'),anywhere(A(P('METHOD'),F('parameters',H(unmatched))))))
clauses.append(C('argument-mismatch',A(M('selected-sync'),owner_allowed,F('name',eligible),
 F('parameters',count('has',M('pos-param'),'SYNC_COUNT')),anywhere(other)),'signature_mismatch',transform=transform))
W('missing_async_pairs',clauses,utils,roots=[m._resolve_target_path(args.source.resolve(),p).relative_to(args.source.resolve()).as_posix()+'/**/*.py' for p in m.TARGET_PATHS],mode='log')

def term(values):return O(names(values),A(K('attribute'),F('attribute',names(values))))
def decorator(rule):return I(A(K('decorated_definition'),H(A(K('decorator'),H(O(rule,A(K('call'),F('function',rule))))))),'neighbor')
def decorator_suffix(pattern):return decorator(O(A(K('identifier'),RX(pattern)),A(K('attribute'),F('attribute',RX(pattern)))))
def body_has(rule):return F('body',H(rule,stop='end'))

m=modules['imperative_private_functions'];sg=importlib.import_module('scripts.lint_lib.symbol_graph')
execution=O(term(sg.EXECUTION_NAMES),A(K('identifier'),RX(r'^(?:execute_grammar|grammar_fetch)')),
 A(K('attribute'),F('attribute',RX(r'^(?:execute_grammar|grammar_fetch)'))))
direct=body_has(A(K('call'),F('function',execution)))
annotated_parameter=A(O(K('typed_parameter'),K('typed_default_parameter')),
 N(H(O(K('list_splat_pattern'),K('dictionary_splat_pattern')))),N({'precedes':{**K('positional_separator'),'stopBy':'end'}}),
 N(A(I(K('class_definition')),O(A(K('typed_parameter'),H(names(['self','cls']))),A(K('typed_default_parameter'),F('name',names(['self','cls'])))))))
annotation=O(F('return_type',RX('|'.join(re.escape(v) for v in sorted(sg.LAZY_MARKERS)))),
 F('parameters',H(A(annotated_parameter,F('type',RX('|'.join(re.escape(v) for v in sorted(sg.LAZY_MARKERS))))))))
lazy=O(F('name',RX('_lazy$')),decorator_suffix('lazy$'),annotation)
private=A(N(F('name',RX(r'^__.*__$'))),O(F('name',RX('^_')),I(A(K('class_definition'),F('name',RX('^_'))),K('class_definition'))))
arch_names='(?i)(?:'+'|'.join(re.escape(v) for v in m.ARCHITECTURAL_NAME_HINTS)+')'
arch=O(F('name',RX(arch_names)),I(A(K('class_definition'),F('name',RX(arch_names))),K('class_definition')),
 {'filePath':{'regex':'|'.join(re.escape(v) for v in m.ARCHITECTURAL_PATH_HINTS)}})
annotation_names=['databaseconnection','syncdatabaseconnection','lazyresult','lazyseq','readtransaction','serverplan','syncversioneddatabaseconnection','versioneddatabaseconnection','versionedreadtransaction','versionedwritetransaction','writetransaction']
annot_scope=O(F('return_type',H(A(K('identifier'),RX('(?i)^(?:'+'|'.join(annotation_names)+')$')),stop='end')),
 F('parameters',H(A(annotated_parameter,F('type',H(A(K('identifier'),RX('(?i)^(?:'+'|'.join(annotation_names)+')$')),stop='end'))),stop='end')))
signature=A(private,N(lazy),N(O(decorator_suffix(r'(?:abstractmethod|property|legacy_sync|legacy_module_sync|direct_legacy_module_sync|legacy_async|legacy_module_async|direct_legacy_module_async)$'),
 decorator(A(K('attribute'),F('attribute',names(['setter','deleter'])))))),
 O(arch,annot_scope))
later={'layout':{'before':{'to':'capture.CURRENT','breaks':{'min':0}}}}
graph_utils={
 'later-module-symbol':{'params':['NAME','CURRENT'],'rule':I(A(K('module'),H(A(fn(),F('name',P('NAME')),N(I(O(fn(),K('class_definition')))),later),stop='end')))},
 'later-method-symbol':{'params':['NAME','CURRENT','GROUP'],'rule':I(A(K('module'),H(A(fn(),F('name',P('NAME')),N(I(fn())),
   I(A(K('class_definition'),F('name',P('GROUP'))),K('class_definition')),later),stop='end')))},
}
one_binding=F('name',{'binds':{'form':['definition'],'bindings':{'max':1}}})
graph_function=A(fn(),N(I(fn())),P('SYMBOL_NODE'),F('name',P('SYMBOL_NAME')),O(
 A(N(I(K('class_definition'))),O(one_binding,N(M('later-module-symbol',NAME='SYMBOL_NAME',CURRENT='SYMBOL_NODE')))),
 A(I(A(K('class_definition'),F('name',P('SYMBOL_CLASS'))),K('class_definition')),
   O(A(one_binding,I(A(K('class_definition'),one_binding),K('class_definition'))),
     N(M('later-method-symbol',NAME='SYMBOL_NAME',CURRENT='SYMBOL_NODE',GROUP='SYMBOL_CLASS'))))))
seed=A(graph_function,O(direct,signature))
legacy_links=[
 {'decorator':r'(?:legacy_sync|legacy_module_sync|direct_legacy_module_sync)$','name':{'replace':'$','by':'_lazy'}},
 {'decorator':r'(?:legacy_async|legacy_module_async|direct_legacy_module_async)$','name':{'replace':'(?:_async)?$','by':'_lazy'}},
]
clauses=[C('direct-execution',A(graph_function,direct),'direct_execution'),
 C('eager-signature',A(graph_function,N(direct),signature),'eager_signature'),
 C('caller',A(graph_function,N(direct),N(signature),{'calls':{'graph':'references','at':'definition','through':'any','siblingLinks':legacy_links,
  'constructorFields':{'fieldToParameter':{'replace':'^_+','by':''}},**seed}}),'calls_imperative')]
W('imperative_private_functions',clauses,graph_utils,mode='log',roots=production_roots)

from data_layer_predicate_compile import compile_allowlist
m=modules['async_sync_delegation'];utils={}
doc=A(K('expression_statement'),H(A(O(K('string'),K('concatenated_string')),{'value':{'type':'str'}})),
 {'nthChild':{'position':1,'ofRule':N(K('comment'))}})
statement=A(P('_'),N(K('comment')),N(doc))
def only_body(rule):return F('body',A({'count':{'has':{**statement,'stopBy':'neighbor'},'min':1,'max':1}},H(A(statement,rule))))
abstract=O(decorator(term(['abstractmethod'])),only_body(O(K('pass_statement'),A(K('expression_statement'),H(K('ellipsis'))),
 A(K('raise_statement'),H(A(K('call'),F('function',names(['NotImplementedError']))))))))
utils['stub-setup']=O(K('pass_statement'),K('delete_statement'),A(K('expression_statement'),H(O(K('assignment'),K('ellipsis')))),
 A(K('if_statement'),F('consequence',H(statement)),N(H(A(K('block'),H(A(statement,N(O(M('stub-setup'),M('unsupported-raise'))))))))))
utils['unsupported-raise']=A(K('raise_statement'),H(O(names(['NotImplementedError','RuntimeError']),A(K('call'),F('function',names(['NotImplementedError','RuntimeError']))))))
unsupported=F('body',O(N(H(statement)),A(H(A(M('unsupported-raise'),N({'precedes':{**statement,'stopBy':'end'}}))),
 N(H(A(statement,N(O(M('stub-setup'),M('unsupported-raise')))))))))
utils['local-target']=O(K('identifier'),A(K('attribute'),F('object',names(['self']))),
 A(O(K('pattern_list'),K('tuple_pattern'),K('list_pattern')),N(H(A(P('_'),N(M('local-target')))))))
utils['static-return']=O(CONST,K('identifier'),A(K('attribute'),F('object',names(['self']))),
 A(O(K('tuple'),K('list'),K('set')),N(H(A(P('_'),N(M('static-return')))))),
 A(K('dictionary'),N(H(A(P('_'),N(O(A(K('pair'),F('key',M('static-return')),F('value',M('static-return'))),A(K('dictionary_splat'),H(M('static-return'))))))))))
local_stmt=O(K('pass_statement'),A(K('expression_statement'),H(A(O(K('assignment'),K('augmented_assignment')),F('left',M('local-target'))))),
 A(K('return_statement'),N(H(A(P('_'),N(M('static-return')))))))
lifecycle=A(F('name',names(['start','stop','_on_last_unsubscribe'])),F('body',N(H(A(statement,N(local_stmt))))))
lazy_call=A(K('call'),F('function',O(A(K('identifier'),RX('_lazy$')),A(K('attribute'),F('attribute',RX('_lazy$'))))))
safe_call=A(K('call'),F('function',names(['_attach_auto_api_hook_registry','cast','JSONResponse'])))
lazy_builder=A(H(lazy_call,stop='end'),N(H(A(K('call'),N(O(lazy_call,safe_call))),stop='end')))
await_node=O(K('await'),A(O(K('for_statement'),K('with_statement')),RX('^async\\b')))
noawait=N(H(await_node,stop=fn()))
accepted=compile_allowlist(m,'AsyncDelegationCollector','_is_acceptable_pattern')
async_common=A(fn(async_=True),P('ASYNC_FN'),F('name',P('ASYNC_NAME')),noawait,N(abstract),N(unsupported),N(lifecycle),N(lazy_builder),
 N(decorator(term(['legacy_async','legacy_module_async']))),N(accepted))
sync_call=H(A(K('call'),F('function',O(A(K('identifier'),P('SYNC_CALL')),A(K('attribute'),F('attribute',P('SYNC_CALL'))))),
 eq('SYNC_CALL','SYNC_BASE')),stop=fn())
transform={'SYNC_BASE':{'replace':{'source':'$ASYNC_NAME','replace':'_async$','by':''}}}
clauses=[C('sync-delegation',A(async_common,F('name',RX('_async$')),sync_call),'A',transform=transform),
 C('no-await',A(async_common,N(A(F('name',RX('_async$')),sync_call))),'B',transform=transform),
 C('mixed-name',A(fn(),F('name',A(RX('(?i)lazy'),RX('(?i)(?:sync|async)')))),
 'C',paths={'exclude':['**/tests/**','**/conform/**']})]
def direct_method(rule):return F('body',H(O(rule,A(K('decorated_definition'),F('definition',rule)))))
def method_slot(node,rule):return I(A(K('class_definition'),P('OWNER'),direct_method(A(fn(),P(node),rule))),K('class_definition'))
def callable_capture(name):return A(O(K('identifier'),K('attribute')),P(name))
def thin(name):
 rawcall=A(K('call'),F('function',callable_capture(name)))
 boundary=A(K('call'),F('function',names(['run_in_legacy_execution_boundary','run_in_legacy_execution_boundary_async'])),
  F('arguments',A({'count':{'has':{**A(P('_'),N(K('comment'))),'stopBy':'neighbor'},'min':1,'max':1}},
   H(A(K('lambda'),F('body',rawcall))))))
 call=A(rawcall,N(boundary))
 statement_of=lambda value:O(A(K('return_statement'),H(value)),A(K('expression_statement'),H(value)))
 return O(A(fn(async_=False),only_body(statement_of(O(call,boundary)))),
          A(fn(async_=True),only_body(statement_of(A(K('await'),H(O(call,boundary)))))))

callback=F('parameters',H(O(names(['fn','callback']),A(O(K('typed_parameter'),K('typed_default_parameter')),
 O(H(names(['fn','callback'])),F('type',RX('Callable')))))))
pair_name=A(F('name',RX('_async$')),N(F('name',RX('^_'))))
sync_member=A(fn(),P('SYNC_FN'),F('name',P('SYNC_NAME')),eq('SYNC_NAME','PAIR_BASE'))
pair=A(fn(),P('ASYNC_FN'),F('name',P('PAIR_NAME')),pair_name,
 I(A(K('class_definition'),P('OWNER'),direct_method(sync_member)),K('class_definition')),
 {'filePath':{'notRegex':r'(?:^|/)(?:tests|conform)/|(?:^|/)impl/browser/'}})
pair_transform={'PAIR_BASE':{'replace':{'source':'$PAIR_NAME','replace':'_async$','by':''}}}
dunder=A(fn(),P('ASYNC_FN'),F('name',A(P('PAIR_NAME'),names(['__aenter__','__aexit__']))),
 I(A(K('class_definition'),P('OWNER'),direct_method(sync_member)),K('class_definition')),
 {'filePath':{'notRegex':r'(?:^|/)(?:tests|conform)/|(?:^|/)impl/browser/'}})
dunder_transform={'PAIR_BASE':{'replace':{'source':'$PAIR_NAME','replace':'^__a','by':'__'}}}
callback_pair=A(pair,N(abstract),method_slot('SYNC_FN',N(abstract)),
 O(callback,method_slot('SYNC_FN',callback)),N(decorator(term(['legacy_async']))),method_slot('SYNC_FN',N(decorator(term(['legacy_sync'])))),
 N(H(A(K('call'),F('function',A(K('attribute'),F('attribute',names(['_raise_async_not_supported']))))),stop='end')),
 method_slot('SYNC_FN',N(H(A(K('call'),F('function',A(K('attribute'),F('attribute',names(['_raise_async_not_supported']))))),stop='end'))))
def normalized_helpers(base):
 return {**base,'SYNC_COMPACT':{'replace':{'source':'$SYNC_HELPER','replace':r'\s','by':''}},
  'ASYNC_COMPACT':{'replace':{'source':'$ASYNC_HELPER','replace':r'\s','by':''}},
  'SYNC_FAMILY':{'replace':{'source':'$SYNC_COMPACT','replace':r'_(?:async|sync)$','by':''}},
  'ASYNC_FAMILY':{'replace':{'source':'$ASYNC_COMPACT','replace':r'_(?:async|sync)$','by':''}}}
for label,common,transforms in [('context',dunder,dunder_transform),('callback',callback_pair,pair_transform)]:
 clauses += [C(label+'-sync-not-thin',A(common,method_slot('SYNC_FN',N(thin('SYNC_HELPER')))),'C',transform=transforms),
  C(label+'-async-not-thin',A(common,method_slot('SYNC_FN',thin('SYNC_HELPER')),N(thin('ASYNC_HELPER'))),'C',transform=transforms),
  C(label+'-different-family',A(common,method_slot('SYNC_FN',thin('SYNC_HELPER')),thin('ASYNC_HELPER'),eq('SYNC_FAMILY','ASYNC_FAMILY',op='ne')),
   'C',transform=normalized_helpers(transforms))]

txn=F('parameters',H(O(names(['txn','tx','vtxn','read_txn','write_txn','base_txn']),
 A(O(K('typed_parameter'),K('typed_default_parameter')),F('type',RX('Transaction'))))))
lazy_sibling=direct_method(A(fn(),F('name',A(P('SIBLING_NAME'),eq('SIBLING_NAME','PAIR_LAZY')))))
execution_lazy=O(A(decorator(term(['lazy','pure_lazy'])),O(N(F('return_type',P('_'))),F('return_type',RX('LazyResult|LazySeq|ServerPlan')))),
 A(F('name',RX('_lazy$')),F('return_type',RX('LazyResult')),H(A(K('call'),F('function',names(['grammar_effect_lazy','grammar_row_lazy','grammar_rows_lazy','grammar_value_lazy']))),stop='end')))
lazy_triple=A(decorator(term(['legacy_async'])),method_slot('SYNC_FN',decorator(term(['legacy_sync']))),
 I(A(K('class_definition'),P('OWNER'),direct_method(A(fn(),F('name',A(P('SIBLING_NAME'),eq('SIBLING_NAME','PAIR_LAZY'))),execution_lazy))),K('class_definition')))
txn_pair=A(pair,N(callback),method_slot('SYNC_FN',N(callback)),N(abstract),method_slot('SYNC_FN',N(abstract)),
 O(txn,method_slot('SYNC_FN',txn)),I(A(K('class_definition'),P('OWNER'),lazy_sibling),K('class_definition')),N(lazy_triple))
txn_transform={**pair_transform,'PAIR_LAZY':{'replace':{'source':'$PAIR_BASE','replace':'$','by':'_lazy'}}}
# A resolving call carries the lazy call's callable, rather than its eager resolve method.
def resolved_call(name, async_):
 terminal='resolve_async' if async_ else 'resolve'
 inner=A(K('call'),F('function',A(callable_capture(name),RX('_lazy$'))))
 direct=A(K('call'),F('function',A(K('attribute'),F('attribute',names([terminal])),F('object',inner))))
 boundary=A(K('call'),F('function',names(['run_in_legacy_execution_boundary_async' if async_ else 'run_in_legacy_execution_boundary'])),
  F('arguments',H(A(K('attribute'),F('attribute',names([terminal])),F('object',inner)))))
 resolver=A(K('call'),F('function',names(['_resolve_legacy_result_async' if async_ else '_resolve_legacy_result_sync'])),
  F('arguments',H(A(K('lambda'),F('body',inner)),stop='end')))
 return A(K('await'),H(O(direct,boundary,resolver))) if async_ else O(direct,boundary,resolver)
def eager_signature(name,post):
 result=[]
 for async_ in (False,True):
  value=resolved_call(name,async_)
  resolve=O(A(K('return_statement'),H(value)),A(K('expression_statement'),H(value)))
  single=only_body(resolve)
  exception=only_body(A(K('try_statement'),N(H(O(K('else_clause'),K('finally_clause')))),
   F('body',A({'count':{'has':{**statement,'stopBy':'neighbor'},'min':1,'max':1}},H(resolve))),
   N(H(A(K('except_clause'),F('body',N(H(A(K('raise_statement'),N({'precedes':{**statement,'stopBy':'end'}}))))))))))
  withpost=F('body',A({'count':{'has':{**statement,'stopBy':'neighbor'},'min':2,'max':2}},
   H(A(resolve,{'precedes':{**A(K('expression_statement'),H(A(K('call'),F('function',callable_capture(post))))),'stopBy':'end'}}))))
  result.append(A(fn(async_=async_),O(single,exception,withpost)))
 return O(*result)
# Non-resolving wrappers cannot satisfy a lazy/eager triple.
for label,rootrule in [('sync',method_slot('SYNC_FN',N(eager_signature('SYNC_LAZY','SYNC_POST')))),
                       ('async',A(method_slot('SYNC_FN',eager_signature('SYNC_LAZY','SYNC_POST')),N(eager_signature('ASYNC_LAZY','ASYNC_POST'))))]:
 clauses.append(C('txn-'+label+'-not-resolving',A(txn_pair,rootrule),'C',transform=txn_transform))
lazy_transforms={**txn_transform,'SYNC_LAZY_NAME':{'replace':{'source':'$SYNC_LAZY','replace':r'\s','by':''}},
 'ASYNC_LAZY_NAME':{'replace':{'source':'$ASYNC_LAZY','replace':r'\s','by':''}}}
clauses.append(C('txn-different-lazy',A(txn_pair,method_slot('SYNC_FN',eager_signature('SYNC_LAZY','SYNC_POST')),
 eager_signature('ASYNC_LAZY','ASYNC_POST'),eq('SYNC_LAZY_NAME','ASYNC_LAZY_NAME',op='ne')),'C',transform=lazy_transforms))

def direct_scope(rule):return O(A(K('module'),H(rule)),A(K('class_definition'),F('body',H(rule))))
def scope_function(rule):return direct_scope(O(rule,A(K('decorated_definition'),F('definition',rule))))
def simple_rebind(left,right):return A(K('expression_statement'),H(A(K('assignment'),F('left',P(left)),F('right',A(K('identifier'),P(right))))))
utils['final-rebinding']={'params':['NAME','NEXT'],'rule':direct_scope(A(simple_rebind('NAME','NEXT'),P('ASSIGNMENT'),
 N({'precedes':{**simple_rebind('NAME','LATER'),'stopBy':'end'}})))}
utils['valid-lazy-binding']={'params':['NAME'],'rule':A(P('REBIND_SCOPE'),O(
 A(N(M('final-rebinding',NAME='NAME',NEXT='NEXT')),
  scope_function(A(fn(),F('name',P('NAME')),decorator(term(['lazy','pure_lazy']))))),
 A(M('final-rebinding',NAME='NAME',NEXT='NEXT'),eq('NAME','NEXT'),scope_function(A(fn(),F('name',P('NAME')),decorator(term(['lazy','pure_lazy']))))),
 A(M('final-rebinding',NAME='NAME',NEXT='NEXT'),eq('NAME','NEXT',op='ne'),H(A(K('identifier'),P('NEXT'),
  I(A(P('REBIND_SCOPE'),M('valid-lazy-binding',NAME='NEXT')))),stop='end'))))}
for mode in ['legacy_sync','legacy_async','legacy_module_sync','legacy_module_async']:
 bare=I(A(K('decorated_definition'),H(A(K('decorator'),H(term([mode]))))),'neighbor')
 strip='_async$' if mode.endswith('async') else '_sync$' if mode=='legacy_sync' else r'\b\B'
 transforms={'LEGACY_BASE':{'replace':{'source':'$LEGACY_NAME','replace':strip,'by':''}},
  'LEGACY_LAZY':{'replace':{'source':'$LEGACY_BASE','replace':'$','by':'_lazy'}},
  'LEGACY_COUNTERPART':{'replace':{'source':'$LEGACY_BASE','replace':'$','by':'' if mode.endswith('async') else '_async'}}}
 othermode=mode.removesuffix('async')+'sync' if mode.endswith('async') else mode.removesuffix('sync')+'async'
 for owner_kind in ['module','class']:
  owner_transforms=dict(transforms)
  if owner_kind=='module':
   owner=A(K('module'),P('LEGACY_OWNER'))
   context=TOP
   required={'regex':r'(?s).*'}
  else:
   owner=A(K('class_definition'),P('LEGACY_OWNER'),F('name',P('LEGACY_CLASS')),TOP)
   owner_transforms.update({'CLASS_TEMP':{'replace':{'source':'$LEGACY_CLASS','replace':'^Sync','by':'@@'}},
    'CLASS_SYNC':{'replace':{'source':'$CLASS_TEMP','replace':'^Async','by':'Sync'}},
    'CLASS_OTHER':{'replace':{'source':'$CLASS_SYNC','replace':'^@@','by':'Async'}}})
   context=I(owner,K('class_definition'))
   required=I(A(K('class_definition'),F('name',RX('(?:Connection|SessionHandle|VersionHandle)$'))),K('class_definition'))
  common=A(fn(),P('LEGACY_FN'),F('name',P('LEGACY_NAME')),bare,context)
  if owner_kind=='class':
   nonexecution_lazy=I(A(owner,scope_function(A(fn(),F('name',A(P('BUILDER_LAZY'),eq('BUILDER_LAZY','LEGACY_LAZY'))),
    decorator(term(['lazy','pure_lazy'])),F('return_type',N(RX('LazyResult|LazySeq|ServerPlan')))))),K('class_definition'))
   common=A(common,N(A(F('name',names(['query','query_sync','query_unfiltered','query_unfiltered_sync'])),F('return_type',RX('Query')))),N(nonexecution_lazy))
  valid_lazy=I(A(owner,H(A(K('identifier'),P('LAZY_LOOKUP'),eq('LAZY_LOOKUP','LEGACY_LAZY')),stop='end'),M('valid-lazy-binding',NAME='LAZY_LOOKUP')),
               K(owner_kind) if owner_kind=='module' else K('class_definition'))
  clauses.append(C(mode+'-'+owner_kind+'-missing-lazy',A(common,required,N(valid_lazy)),'C',transform=owner_transforms))
  other=A(fn(),F('name',A(P('COUNTERPART'),eq('COUNTERPART','LEGACY_COUNTERPART'))),decorator(term([othermode])))
  counterpart=I(A(owner,scope_function(other)),K(owner_kind) if owner_kind=='module' else K('class_definition'))
  if owner_kind=='class':counterpart=O(counterpart,I(A(K('module'),H(A(K('class_definition'),TOP,
   F('name',A(P('COUNTERPART_CLASS'),eq('COUNTERPART_CLASS','CLASS_OTHER'))),scope_function(other)),stop='end'))))
  clauses.append(C(mode+'-'+owner_kind+'-missing-pair',A(common,N(counterpart)),'C',transform=owner_transforms))

# Escape-hatch reports collapse all calls on one physical source line.
escape_name=O(names(m.ESCAPE_HATCH_PATTERNS),A(K('attribute'),F('attribute',names([x for x in m.ESCAPE_HATCH_PATTERNS if '.' not in x]))))
escape_allowed=O(*[A({'filePath':{'regex':re.escape(path)+'$'}},I(A(fn(),F('name',names([name]))),fn())) for path,name in sorted(m.ESCAPE_HATCH_FUNCTION_ALLOWLIST)])
escape=A(K('call'),P('ESCAPE'),F('function',escape_name),N(escape_allowed),
 {'filePath':{'notRegex':r'(?:^|/)(?:tests|conform)/|(?:^|/)impl/browser/'}})
clauses.append({'id':'escape-hatch','budget':{'query':{'evidence':{'as':'row','query':{'ast':{'language':'python','syntax':'valid','rule':escape}},
 'project':{'path':'file.path','line':'capture.ESCAPE.startLine'}}},'count':{'lte':0},'group_by':['row.path','row.line']},
 'violation':{'code':'C','message':'Thread-pool execution escape hatch','anchor':'witness.span'}})
# Each of these alternatives reports at the same function, and the alternatives
# within a family are mutually exclusive. One traversal can evaluate the family.
def merge_alternatives(ids, name, transform_overrides=None):
 selected=[c for c in clauses if c['id'] in ids]
 if not selected:return
 transforms={k:v for c in selected for k,v in c['forbid']['ast'].get('transform',{}).items()}
 transforms.update(transform_overrides or {})
 merged=C(name,O(*(c['forbid']['ast']['rule'] for c in selected)),'C',transform=transforms)
 clauses[:]=[c for c in clauses if c['id'] not in ids]+[merged]
merge_alternatives({c['id'] for c in clauses if c['id'].startswith(('context-','callback-','txn-'))},'wrapper-shape',
 {'PAIR_UNDUNDER':{'replace':{'source':'$PAIR_NAME','replace':'^__a','by':'__'}},
  'PAIR_BASE':{'replace':{'source':'$PAIR_UNDUNDER','replace':'_async$','by':''}}})
for mode in ['legacy_sync','legacy_async','legacy_module_sync','legacy_module_async']:
 for requirement in ['missing-lazy','missing-pair']:
  merge_alternatives({mode+'-'+owner+'-'+requirement for owner in ['module','class']},mode+'-'+requirement)
W('async_sync_delegation',clauses,utils,roots=[p for p in production_roots if p!='api/**/*.py'])

from data_layer_relational_five import public_api_clause
from scripts.lint_lib import symbol_graph as source_symbol_graph
W('public_api_triples',[public_api_clause(modules['public_api_triples'],source_symbol_graph)],options={'executionOnly':{'type':'boolean','default':True}},roots=production_roots,mode='log')

from data_layer_relational_five import dead_code_clause
def reference_contract(value):
 if isinstance(value,dict):
  if value.get('calls',{}).get('graph')=='references':return {key:value['calls'][key] for key in ('siblingLinks','constructorFields') if key in value['calls']}
  for child in value.values():
   found=reference_contract(child)
   if found is not None:return found
 elif isinstance(value,list):
  for child in value:
   found=reference_contract(child)
   if found is not None:return found
 return None
contract=reference_contract(json.loads((args.output/'data-layer-imperative-private-functions.json').read_text()))
W('internal_dead_code',[dead_code_clause(modules['internal_dead_code'],source_symbol_graph,contract)],roots=production_roots,mode='log')
