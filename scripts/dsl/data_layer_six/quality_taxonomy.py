# SPDX-License-Identifier: Apache-2.0
"""Project taxonomy evidence collected as bounded function predicates."""
from itertools import combinations
from .common import *

def taxonomy_clauses(m):
 same=O(A(K('identifier'),P('CALLEE')),A(K('attribute'),F('attribute',P('CALLEE'))))
 recursive=A(K('call'),F('function',same),{'compare':{'left':'FN_NAME','right':'CALLEE','op':'eq','text':'source'}})
 dispatch=O(call(['isinstance','is_dataclass','get_origin','id','type']),A(K('attribute'),F('attribute',names(['__name__','__module__']))))
 branch=A(O(K('if_statement'),K('elif_clause')),F('condition',O(dispatch,H(dispatch,stop='end'))))
 count=lambda rule,minimum:{'count':{'has':{**rule,'stopBy':'end'},'min':minimum}}
 literal=A(O(K('string'),K('concatenated_string')),O({'value':{'regex':'(?is)unsupported.*shape|shape.*unsupported'}},{'parse':{'grammar':'pythonFString','has':{'kind':'literal','regex':'(?is)unsupported.*shape|shape.*unsupported'}}}))
 colors=[H(call(['id']),stop='end'),H(A(K('call'),F('function',A(K('attribute'),F('attribute',names(['__getattribute__','__setattr__','__new__']))))),stop='end'),H(A(K('for_statement'),N(RX('^async')),H(recursive,stop='end')),stop='end'),
  O(H(call(['fields']),stop='end'),H(A(K('call'),F('function',A(K('attribute'),F('attribute',names(['internal_plan_ref_state','internal_plan_payload_state','items','values','model_fields']))))),stop='end'),H(A(K('for_statement'),N(RX('^async')),O(H(call(['getattr']),stop='end'),F('right',A(K('attribute'),F('attribute',names(['model_fields'])))))),stop='end'))]
 candidate=A(K('function_definition'),P('FN'),F('name',P('FN_NAME')),count(recursive,2),count(branch,3),H(literal,stop='end'),H(A(K('raise_statement'),H(A(K('call'),F('function',names(['TypeError']))))),stop='end'),O(*(A(a,b) for a,b in combinations(colors,2))),N({'filePath':{'regex':'(?:^|/)tests(?:/|$)'}}))
 query="SELECT __path AS path,__start AS start_byte,__end AS end_byte,'SPQ-049' AS issue,__start AS identity FROM candidates"
 local=query+" WHERE (SELECT count(*) FROM candidates other WHERE other.__path=candidates.__path)>=2"
 across=query+' WHERE (SELECT count(*) FROM candidates)>=2'
 import copy
 exact=[p.relative_to(m.ROOT).as_posix() for p in (*m.CORE_PLAN_FILES,*m.BATCHING_SUPPORT_FILES,*m.LIFECYCLE_FILES,m.OPTIMIZER_ANALYSIS_FILE,m.CONDITIONAL_DIALECT_TEST_FILE)]
 directories=[p.relative_to(m.ROOT).as_posix() for p in (*m.STEP_DIRS,m.STRATEGY_DIR)]
 primary=O({'filePath':{'regex':'^(?:'+'|'.join(re.escape(p) for p in sorted(set(exact)))+')$'}},A(N({'filePath':{'regex':'(?:^|/)__init__\\.py$'}}),{'filePath':{'regex':'^(?:'+'|'.join(re.escape(p)+'/' for p in directories)+r'|impl/common/l2070_server_plan/[^/]+$)'}}))
 clauses=[]
 for name,entries,sql,partition in [('local',['_check_recursive_payload_taxonomy_duplication','_check_serverplan_quality_source'],local,True),('project',['_check_recursive_payload_taxonomy_duplication_across_surfaces'],across,False)]:
  enabled=choice('oracleAnalyzer',TRUE if partition else FALSE,primary if partition else TRUE)
  gate_= {'$option':'entryPoint','cases':{'all':enabled,**{e:TRUE for e in entries}},'default':FALSE}
  evidence=E(A(copy.deepcopy(candidate),gate_),{'name':'capture.FN_NAME'})
  # Entry points and the production domain also select files before any node is matched.
  every={'path':{'regex':r'(?s).*'}};none={'path':{'regex':r'\b\B'}}
  files={'path':{'regex':'^(?:'+'|'.join(re.escape(p) for p in sorted(set(exact)))+')$|^(?:'+'|'.join(re.escape(p)+'/' for p in directories)+r'|impl/common/l2070_server_plan/[^/]+$)','notRegex':r'(?:^|/)__init__\.py$'}}
  evidence['evidence']['query']['ast']['file']={'$option':'entryPoint','cases':{'all':choice('oracleAnalyzer',every if partition else none,files if partition else every),**{e:every for e in entries}},'default':none}
  clauses.append({'id':'quality-'+name+'-taxonomy','relational':{'inputs':{'candidates':evidence},'query':sql,'key':['issue','identity'],**({'partitionBy':'file'} if partition else {})},'violation':{'code':'SPQ-049','message':'Recursive payload taxonomy duplication','anchor':'witness.span'}})
 return clauses
