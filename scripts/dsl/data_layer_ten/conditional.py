# SPDX-License-Identifier: Apache-2.0
"""Local conditional checks and the existing shared typed-contract producer."""
from .common import *
import copy,json
from pathlib import Path
STEP='l2070_batching/conditional_step.py'
COMPILER='impl/common/l2070_server_plan/steps/_conditional_compile.py'
PHYSICAL='impl/common/l2070_server_plan/steps/sql_conditional.py'
BACKEND='impl/common/l2070_server_plan/sql_backend_executor.py'
TEST='impl/common/l2070_server_plan/steps/tests/test_conditional_compile_dialects.py'

def candidate(root):
 c=constants(root/'scripts/lint_conditional_architecture.py');rows=[]
 inclass="EXISTS(SELECT 1 FROM classes cls WHERE cls.__path=n.__path AND cls.id=n.cls AND cls.name='ConditionalStep')"
 compiling=inclass+" AND EXISTS(SELECT 1 FROM functions fn WHERE fn.__path=n.__path AND fn.id=n.fn AND fn.name='compile')"
 def add(code,msg,table='a n',where='1',order='n.id',phase=0):rows.append(row(code,q(msg),table,where,order=order,phase=phase))
 prefixes=lambda column:'('+' OR '.join(f'substr({column},1,{len(p)})={q(p)}' for p in c['FORBIDDEN_IMPL_PREFIXES'])+')'
 for comp,code,prefix in [(True,'CONDA-001','ConditionalStep.compile()'),(False,'CONDA-002','l2070_batching.conditional_step')]:
  context='('+compiling+')' if comp else 'NOT ('+compiling+')'
  add(code,prefix+' must not import impl SQL compiler modules.',where=f"n.__path={q(STEP)} AND n.kind='ImportFrom' AND {prefixes('n.module')} AND {context}")
  add(code,prefix+' must not import SQL grammar classes.','a n JOIN a alias ON alias.__path=n.__path AND alias.parent=n.id AND alias.kind=\'alias\'',f"n.__path={q(STEP)} AND n.kind='ImportFrom' AND substr(alias.name,1,3)='Sql' AND {context}",order='n.id+0.01+alias.position*0.001')
 add('CONDA-002','l2070_batching.conditional_step must not import impl SQL compiler modules.','a n JOIN a alias ON alias.__path=n.__path AND alias.parent=n.id AND alias.kind=\'alias\'',f"n.__path={q(STEP)} AND n.kind='Import' AND {prefixes('alias.name')}",order='n.id+alias.position*0.001')
 add('CONDA-001','ConditionalStep.compile() must not import SQL grammar classes.','a n JOIN a alias ON alias.__path=n.__path AND alias.parent=n.id AND alias.kind=\'alias\'',f"n.__path={q(STEP)} AND n.kind='Import' AND substr(alias.name,1,3)='Sql' AND ({compiling})",order='n.id+0.01+alias.position*0.001')
 for condition,msg,ordinal in [("n.callee='compile'",'ConditionalStep.compile() must not compile branch children directly.',0),("substr(n.callee,1,3)='Sql'",'ConditionalStep.compile() must not assemble SQL grammar locally.',1),("n.callee='CompiledStep' AND EXISTS(SELECT 1 FROM a kw WHERE kw.__path=n.__path AND kw.parent=n.id AND kw.kind='keyword' AND kw.name='grammar')",'ConditionalStep.compile() must not return CompiledStep(grammar=...).',2)]:add('CONDA-001',msg,'calls n',f'n.__path={q(STEP)} AND ({compiling}) AND ({condition})',order=f'n.id+{ordinal}*0.001')
 add('CONDA-003','Conditional dialect tests must compile SqlConditionalStep, not a semantic conditional.','calls n',f"n.__path={q(TEST)} AND n.callee_kind='Attribute' AND n.callee='compile' AND n.receiver_kind='Name' AND n.receiver_name IN ('cs','conditional','conditional_step')")
 add('CONDA-001','ConditionalStep must define compile() as an explicit architecture boundary.','classes n',f"n.__path={q(STEP)} AND n.name='ConditionalStep' AND n.id=(SELECT max(c.id) FROM classes c WHERE c.__path=n.__path AND c.name=n.name) AND NOT EXISTS(SELECT 1 FROM functions f JOIN classes c ON c.__path=f.__path AND c.id=f.cls WHERE f.__path=n.__path AND f.name='compile' AND c.name='ConditionalStep')",phase=1)
 add('CONDA-004','ConditionalStep branch validation must retain DDL and not_where_gatable guard constants.',where=f"n.__path={q(STEP)} AND n.kind='Module' AND (NOT EXISTS(SELECT 1 FROM a v WHERE v.__path=n.__path AND v.kind='Name' AND v.name='_DDL_TAGS') OR NOT EXISTS(SELECT 1 FROM a v WHERE v.__path=n.__path AND v.kind='Name' AND v.name='_UNGATABLE_TAG'))",phase=1)
 add('CONDA-004','ConditionalStep.__init__ must validate both then and else branch contents.',where=f"n.__path={q(STEP)} AND n.kind='Module' AND (SELECT count(*) FROM calls v JOIN functions f ON f.__path=v.__path AND f.id=v.fn JOIN classes c ON c.__path=v.__path AND c.id=v.cls WHERE v.__path=n.__path AND v.callee='_validate_branch_content' AND f.name='__init__' AND c.name='ConditionalStep')<2",phase=1)
 rows[-1]=rows[-1].replace('n.line,',"coalesce((SELECT f.line FROM functions f JOIN classes c ON c.__path=f.__path AND c.id=f.cls WHERE f.__path=n.__path AND c.name='ConditionalStep' AND f.name='__init__' ORDER BY f.id DESC LIMIT 1),1) line,",1)
 clauses=[]
 for i in range(0,len(rows),4):
  item=make_clause('conditional-local-'+str(i//4),[],' UNION ALL '.join(rows[i:i+4]));item['relational']['line']='line';triggered(item,['ConditionalStep','compile_conditional_step','.compile(','not ctx.backend_type.is_sql_backend','not ctx.supports_data_modifying_ctes']);clauses.append(item)
 # Reuse the shipped shared producer, including its tested inheritance and control flow.
 shared=json.loads((Path(__file__).resolve().parents[3]/'packs/data-layer/candidates/rules/data-layer-serverplan-quality.json').read_text())
 for original in shared['clauses']:
  if not original['id'].startswith('quality-conditional'):continue
  item=copy.deepcopy(original);plan=item['relational'];query=plan['query']
  # The shipped clause already projects full diagnostic messages and source locations.
  def bind(value):
   if isinstance(value,dict) and '$option' in value:
    name=value['$option'];return bind(value.get('cases',{}).get('conditional_architecture_findings' if name=='entryPoint' else 'true',value.get('default')))
   if isinstance(value,dict):return {k:bind(v) for k,v in value.items()}
   if isinstance(value,list):return [bind(v) for v in value]
   return value
  item=bind(item);plan=item['relational']
  messages={'semantic-compile':'semantic ConditionalStep must reject direct compilation with TypeError','executor-hook':'SqlBackendExecutor must define _lower_conditional_step','semantic-contract':'semantic typed conditional contract must unconditionally reject with TypeError','physical-contract':'physical typed conditional contract must override semantic rejection without backend selection','lowering-contract':'lowering typed conditional contract must produce SqlConditionalStep from the semantic conditional','compiler-contract':'compiler typed conditional contract must validate its SqlConditionalStep before strategy selection'}
  original_query=plan['query'];message='CASE detector '+' '.join('WHEN '+q(k)+' THEN '+q(v) for k,v in messages.items())+' END'
  prefix="coalesce(source_text(path,0,start_byte),'')"
  plan['query']="SELECT path,start_byte,end_byte,1+length("+prefix+")-length(replace("+prefix+",char(10),'')) line,'CONDA-005' issue,"+message+" message,identity,detector,0 phase,0 ordering,'' symbol,'' sort_name FROM ("+original_query+") WHERE detector IN "+literals(messages)
  plan['line']='line';plan['message']='message'
  clauses.append(item)
 result=make_document('conditional_architecture',clauses,[STEP,COMPILER,PHYSICAL,BACKEND,TEST],exclude=[])
 result['clauses'][0]['define'].update(bind({k:v for k,v in shared['clauses'][0]['define'].items() if k!='options'}))
 return result
