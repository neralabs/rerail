# SPDX-License-Identifier: Apache-2.0
"""Authoring primitives shared by the sixth-batch candidates."""
from __future__ import annotations
import json,re

def A(*items):return {'all':list(items)}
def O(*items):return {'any':list(items)}
def N(item):return {'not':item}
def K(kind):return {'kind':kind}
def P(name):return {'pattern':'$'+name}
def RX(pattern):return {'regex':pattern}
def H(rule,field=None,stop='neighbor'):return {'has':{**rule,'stopBy':stop,**({'field':field} if field else {})}}
def I(rule,stop='end',field=None):return {'inside':{**rule,'stopBy':stop,**({'field':field} if field else {})}}
def F(field,rule):return H(rule,field)
def optional(rule):return O(rule,P('_'))
def names(values):return A(K('identifier'),RX('^(?:'+'|'.join(re.escape(x) for x in sorted(values))+')$' if values else r'\b\B'))
def term(values):return O(names(values),A(K('attribute'),F('attribute',names(values))))
def call(values):return A(K('call'),F('function',term(values)))
def fn(name=None):return A(K('function_definition'),F('name',names([name]))) if name else K('function_definition')
def eq(left,right):return {'compare':{'left':left,'right':right,'op':'eq','text':'node'}}
TRUE={'filePath':{'regex':r'(?s).*'}}
FALSE={'filePath':{'regex':r'\b\B'}}

def gate(entries,enabled=TRUE,disabled=FALSE):
 return {'$option':'entryPoint','cases':{entry:enabled for entry in ['all',*entries]},'default':disabled}
def analyzer_code(code,production):return {'$option':'oracleAnalyzer','cases':{'true':code,'false':production}}
def C(id,rule,code,entries=(),anchor='witness.span',each=None):
 return {'id':id,'forbid':{'ast':{'language':'python','syntax':'valid','rule':A(gate(entries),rule)}},
  'violation':{'code':code,'message':id,'anchor':anchor,**({'each':each} if each else {})}}
def B(id,rule,code,entries=(),project=None):
 return {'id':id,'budget':{'query':{'evidence':{'as':'row','query':{'ast':{'language':'python','syntax':'valid','rule':A(gate(entries),rule)}},
   'project':project or {'path':'file.path','line':'witness.startLine'}}},'group_by':['row.path','row.line'],'count':{'lte':0}},
  'violation':{'code':code,'message':id,'anchor':'witness.span'}}
def E(rule,project,each=None,anchor=None,transform=None):
 ast={'language':'python','syntax':'valid','rule':rule}
 if transform:ast['transform']=transform
 return {'evidence':{'as':'row','query':{'ast':ast},'project':project,**({'each':each} if each else {}),**({'anchor':anchor} if anchor else {})}}
def S(name,query,*indexes):return {'name':name,'query':query,**({'indexes':[list(v) for v in indexes]} if indexes else {})}
def q(value):return "'"+str(value).replace("'","''")+"'"
def literals(values):return '('+','.join(q(v) for v in sorted(values))+')'
def choice(option,yes,no):return {'$option':option,'cases':{'true':yes,'false':no}}

def write(out,name,clauses,pin,roots,exclude,utils=None,options=None,mode='block'):
 from .ast_relations import trim_ast_projection,typed_input_partitions,batched_reachability,defer_node_text,shared_syntax_relations,whole_file_syntax_relations,narrow_ast_ordinals
 from .sql_columns import narrow_columns
 quality=name=='serverplan_quality'
 utils=dict(utils or {})
 for clause in clauses:
  if 'relational' in clause:
   if quality:defer_node_text(clause)
   trim_ast_projection(clause)
   plan=clause['relational']
   if quality:
    for stage in plan.get('stages',[]):
     if stage['name']=='returns':stage.setdefault('indexes',[]).append(['__path','__start','__end'])
   if not plan.get('locations') and not clause['id'].startswith('quality-conditional') and clause['id']!='quality-project-taxonomy':
    plan.setdefault('partitionBy','file')
   if name=='serverplan_state_tripwires' and list(plan['inputs'])==['events']:
    plan['requireNonemptyInput']='events'
   if name=='serverplan_state_tripwires' and clause['id'] in {'state-signatures-1','state-flow-1','state-projection-recovery-2','state-flow-project-1'}:
    # These generators select every CST node in each admitted file, including the
    # closed repository domain. Keep each detector's existing source/option gate.
    batched_reachability(clause,forest=True)
    whole_file_syntax_relations(clause)
    plan['requireNonemptyInput']='syntax_gate'
    continue
   if quality and shared_syntax_relations(clause):
    narrow_columns(plan)
    continue
   if quality and clause['id'].startswith('quality-conditional'):narrow_ast_ordinals(clause)
   if not clause['id'].startswith('step-'):typed_input_partitions(clause)
   if quality and clause['id'].startswith('quality-conditional'):
    # The typed columns share one selective AST query, including its captures.
    asts=[r['evidence']['query']['ast'] for r in plan['inputs'].values()]
    selection=asts[0]['rule']['all'][-1]
    if not all(ast['rule']['all'][-1]==selection for ast in asts):
     raise ValueError('conditional input partitions must retain one shared selection')
    utility=clause['id']+'-selection'
    utils[utility]=selection
    for ast in asts:ast['rule']['all'][-1]={'matches':utility}
   if quality or name=='serverplan_state_tripwires':batched_reachability(clause,forest=True)
 definition={'options':{'oracleAnalyzer':{'type':'boolean','default':False},'entryPoint':{'type':'string','default':'all'},**(options or {})}}
 if utils:definition.update(language='python',utils=utils)
 doc={'format':'rerail.policy/v1','id':'data-layer-'+name.replace('_','-'),'revision':1,'mode':mode,
  'description':f'Candidate port of nera-data-layer {name} at {pin}.',
  'scope':{'include':roots,'exclude':exclude},'clauses':[{'id':'shared','define':definition},*clauses]}
 (out/(doc['id']+'.json')).write_text(json.dumps(doc,indent=2)+'\n')
