# SPDX-License-Identifier: Apache-2.0
"""Optimizer v2 checks preserve upstream ordering and diagnostic deduplication."""
from .common import *
import ast

def candidate(root):
 source=(root/'scripts/lint_plan_optimizer_v2.py').read_text();c=constants(root/'scripts/lint_plan_optimizer_v2.py');templates={}
 for n in ast.walk(ast.parse(source)):
  if isinstance(n,ast.Call) and isinstance(n.func,ast.Name) and n.func.id=='Finding' and len(n.args)==4 and isinstance(n.args[2],ast.Constant):templates.setdefault(n.args[2].value,[]).append(n.args[3])
 def message(code,bindings=None,index=0):
  bindings=bindings or {};expr=templates[code][index]
  def render(node):
   if isinstance(node,ast.Constant):return q(node.value)
   if isinstance(node,ast.JoinedStr):return '||'.join(render(part) for part in node.values)
   if isinstance(node,ast.FormattedValue):
    name=ast.unparse(node.value)
    if name in c:return q(repr(c[name]) if node.conversion==114 else c[name])
    if node.conversion==114 and name+'.repr' in bindings:return bindings[name+'.repr']
    value=bindings[name]
    return '(char(39)||'+value+'||char(39))' if node.conversion==114 else '('+value+')'
   raise ValueError(ast.dump(node))
  return render(expr)
 stages=[S('optimizer_contract',{'$option':'entryPoint','escape':'sql','template':'SELECT {} entry'}),S('optimizer_imports',"SELECT n.*,CASE WHEN n.kind='ImportFrom' THEN n.module ELSE alias.name END imported,coalesce(alias.position,0) ordinal FROM a n LEFT JOIN a alias ON alias.__path=n.__path AND alias.parent=n.id AND alias.kind='alias' AND n.kind='Import' WHERE n.kind='Import' OR n.kind='ImportFrom' AND n.level=0",('__path','id')),
 S('optimizer_files',"SELECT n.*,rx_replace(n.__path,'^.*/','') basename,rx_matches(n.__path,'(?:^|/)l2065_plan_core/') core,1+length(text)-length(replace(text,char(10),'')) line_count FROM (SELECT a.*,CASE WHEN (SELECT entry FROM optimizer_contract)='scan_file' THEN source_text(__path) ELSE replace(replace(source_text(__path),char(13)||char(10),char(10)),char(13),char(10)) END text FROM a WHERE kind='Module') n",('__path',)),
 S('pass_classes',"SELECT n.*,(SELECT v.name FROM a stmt JOIN a target ON target.__path=stmt.__path AND target.parent=stmt.id AND target.field='target' AND target.kind='Name' AND target.name='phase' JOIN a v ON v.__path=stmt.__path AND v.parent=stmt.id AND v.field='value' AND v.kind='Attribute' WHERE stmt.__path=n.__path AND stmt.parent=n.id AND stmt.kind='AnnAssign' ORDER BY stmt.id LIMIT 1) declared_phase FROM classes n WHERE n.parent=0 AND substr(n.name,-4)='Pass' AND NOT EXISTS(SELECT 1 FROM a base WHERE base.__path=n.__path AND base.parent=n.id AND base.field='bases' AND base.kind IN ('Name','Attribute') AND base.name='Protocol')",('__path','id'),('__path','name')),
 S('pipeline_entries',"SELECT tuple.__path,tuple.parent assignment,call.position,CASE cls.declared_phase WHEN 'NORMALIZE' THEN 0 WHEN 'ANALYZE' THEN 1 WHEN 'PHYSICAL' THEN 2 END phase FROM a tuple JOIN calls call ON call.__path=tuple.__path AND call.parent=tuple.id AND call.field='elts' AND call.callee_kind='Name' JOIN pass_classes cls ON cls.__path=call.__path AND cls.name=call.callee WHERE tuple.kind='Tuple' AND tuple.field='value' AND cls.id=(SELECT max(p.id) FROM pass_classes p WHERE p.__path=cls.__path AND p.name=cls.name AND p.id<tuple.id)",('__path','assignment','position'))]
 rows=[]
 def add(code,source='a n',where='1',bindings=None,index=0,phase=0,order='n.id'):
  rows.append(row(code,message(code,bindings,index),source+' JOIN optimizer_files file ON file.__path=n.__path',where,phase=phase,order=order))
 prefixes=lambda column,values:'('+' OR '.join(f'{column}={q(p)} OR substr({column},1,{len(p)+1})={q(p+".")}' for p in values)+')'
 io=prefixes('n.imported',c['IO_MODULE_PREFIXES']);legacy=prefixes('n.imported',c['LEGACY_PREFIXES']);core=prefixes('n.imported',c['CORE_FORBIDDEN_PREFIXES'])
 add('PV2-001','optimizer_imports n',io,{'module':'n.imported'},order='n.id+n.ordinal*0.001')
 add('PV2-002','optimizer_imports n','NOT '+io+' AND file.core AND '+core,{'module':'n.imported'},order='n.id+n.ordinal*0.001')
 add('PV2-003','optimizer_imports n','NOT '+io+" AND NOT file.core AND file.basename!='adapter.py' AND "+legacy,{'module':'n.imported'},order='n.id+n.ordinal*0.001')
 add('PV2-010',where="file.basename!='scope.py' AND n.kind='Attribute' AND n.name IN "+literals(c['SCOPE_INTERNAL_ATTRS']),bindings={'node.attr':'n.name'},phase=1)
 add('PV2-011','calls n',"file.basename!='scope.py' AND n.callee_kind='Attribute' AND n.callee IN "+literals(c['DOTTED_PARSE_METHODS'])+' AND '+child('n','args',alias='arg',condition="arg.position=0 AND arg.kind='Constant' AND arg.value='.'"),phase=1)
 add('PV2-012',"a n JOIN a owner ON owner.__path=n.__path AND owner.id=n.parent", "n.kind='Name' AND n.field IN ('target','targets') AND owner.kind IN ('Assign','AnnAssign') AND substr(n.name,-9)='_by_table'",{'name':'n.name'},phase=1,order='owner.id+n.position*0.001')
 add('PV2-012','parameters n',"n.field IN ('posonlyargs','args','kwonlyargs') AND substr(n.name,-9)='_by_table'",{'name':'n.name'},phase=1,order='n.fn+n.id*0.000001')
 add('PV2-020',"a n JOIN a operand ON operand.__path=n.__path AND operand.parent=n.id AND operand.field IN ('left','comparators') AND operand.kind='Constant'", "file.basename!='adapter.py' AND n.kind='Compare' AND operand.value IN "+literals(c['SHAPE_LITERALS']),{'operand.value':'operand.value','operand.value.repr':'operand.value_repr'},phase=2,order='n.id+operand.id*0.000001')
 add('PV2-021',where="n.kind='Constant' AND n.value_kind='str' AND substr(n.value,-6)='_count'",bindings={'node.value':'n.value','node.value.repr':'n.value_repr'},phase=2)
 add('PV2-021',where="n.kind='JoinedStr' AND "+child('n','values',alias='part',condition="part.kind='Constant' AND part.value_kind='str' AND substr(part.value,-6)='_count'"),index=1,phase=2)
 add('PV2-022',where="n.kind='BinOp' AND n.op='BitAnd' AND EXISTS(SELECT 1 FROM calls call WHERE call.__path=n.__path AND call.parent=n.id AND call.field IN ('left','right') AND call.callee_kind='Name' AND call.callee IN ('set','frozenset'))",index=1,phase=2)
 add('PV2-022','calls n',"n.callee_kind='Attribute' AND n.callee='intersection'",phase=2)
 add('PV2-023',"calls n JOIN a arg ON arg.__path=n.__path AND arg.parent=n.id AND arg.field='args' AND arg.position=0 AND arg.kind='Constant' AND arg.value_kind='str'", "n.callee_kind='Name' AND n.callee='ColumnSchema'",{'node.args[0].value':'arg.value','node.args[0].value.repr':'arg.value_repr'},phase=2)
 add('PV2-030',where="n.kind IN ('Name','Attribute') AND n.name IN "+literals(c['MODEL_INTROSPECTION_NAMES']),bindings={'name':'n.name'},phase=3)
 add('PV2-031',"functions f JOIN a n ON n.__path=f.__path AND n.id BETWEEN f.id AND f.last JOIN a exception ON exception.__path=n.__path AND exception.parent=n.id AND exception.field='exc' AND exception.kind!='Name'", "n.kind='Raise' AND NOT "+desc('f',alias='text',condition="text.kind='Constant' AND text.value_kind='str' AND rx_matches(text.value,'\\b(Fix|Use|Add|Publish|Call|Bind|Project|Split|Map|Cast|Rename|Remove|Inspect|Wrap|Keep)\\b')"),{'func.name':'f.name'},phase=3,order='f.id*1000000+n.id')
 legal="file.basename NOT IN ('effects.py','adapter.py')"
 add('PV2-060',where=legal+" AND n.kind='FunctionDef' AND n.name IN "+literals(c['LEGACY_ORACLE_METHOD_NAMES']),bindings={'node.name':'n.name'},phase=4)
 add('PV2-060',where=legal+" AND n.kind='Attribute' AND n.name IN "+literals(c['VISIBILITY_ATTR_NAMES']),bindings={'node.attr':'n.name'},index=1,phase=4)
 add('PV2-060','calls n',legal+" AND n.callee_kind='Attribute' AND n.callee='can_share_statement_after'",index=2,phase=4)
 count="file.line_count"
 add('PV2-044',where="n.kind='Module' AND ("+count+')>'+str(c['MAX_V2_FILE_LINES']),bindings={'line_count':count},phase=5,order='-1')
 add('PV2-042',where="n.kind='Assign' AND "+child('n','targets',alias='t',condition="t.position=0 AND t.kind='Attribute'")+' AND NOT EXISTS(SELECT 1 FROM functions f WHERE f.__path=n.__path AND n.id BETWEEN f.id AND f.last AND f.name IN (\'__init__\',\'__post_init__\'))',phase=5)
 add('PV2-040','pass_classes n',"file.basename='passes.py' AND (n.declared_phase IS NULL OR NOT EXISTS(SELECT 1 FROM a stmt JOIN a target ON target.__path=stmt.__path AND target.parent=stmt.id AND target.field='target' AND target.kind='Name' AND target.name='name' WHERE stmt.__path=n.__path AND stmt.parent=n.id AND stmt.kind='AnnAssign') OR NOT EXISTS(SELECT 1 FROM functions f WHERE f.__path=n.__path AND f.parent=n.id AND f.kind='FunctionDef' AND f.name='run'))",{'node.name':'n.name'},phase=5)
 add('PV2-041',where="file.basename='passes.py' AND n.parent=0 AND n.kind IN ('Assign','AnnAssign') AND "+child('n','value',alias='v',condition="v.kind='Tuple'")+" AND EXISTS(SELECT 1 FROM a t WHERE t.__path=n.__path AND t.parent=n.id AND t.field IN ('target','targets') AND t.position=0 AND t.kind='Name' AND t.name='DEFAULT_V2_PASSES') AND EXISTS(SELECT 1 FROM pipeline_entries first JOIN pipeline_entries later ON later.__path=first.__path AND later.assignment=first.assignment AND later.position>first.position AND later.phase<first.phase WHERE first.__path=n.__path AND first.assignment=n.id)",phase=5)
 add('PV2-043','functions n',"file.basename='passes.py' AND n.parent=0 AND n.kind='FunctionDef' AND n.name='run_verified_passes' AND NOT "+desc('n',alias='v',condition="v.kind='Name' AND v.name='PlanVerifier'"),phase=5)
 for index in range(0,len(rows),4):stages.append(S('optimizer_findings_'+str(index//4),' UNION ALL '.join(rows[index:index+4])))
 merged=' UNION ALL '.join('SELECT * FROM optimizer_findings_'+str(i//4) for i in range(0,len(rows),4))
 query="SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY path,issue,line ORDER BY phase,ordering) chosen FROM ("+merged+')) WHERE chosen=1'
 item=make_clause('optimizer-file-checks',stages,query);item['relational']['line']='line'
 verifier=c['VERIFIER_RELPATH'];doc=c['INVARIANT_DOC_RELPATH']
 contract_stages=[S('registered_invariants',"SELECT n.*,v.value invariant,v.value_repr invariant_repr FROM calls n JOIN a kw ON kw.__path=n.__path AND kw.parent=n.id AND kw.kind='keyword' AND kw.name='code' JOIN a v ON v.__path=kw.__path AND v.parent=kw.id AND v.field='value' AND v.kind='Constant' AND v.value_kind='str' WHERE n.__path="+q(verifier)+" AND n.callee_kind='Name' AND n.callee='OptimizerInvariant'",('__path','id'))]
 registered="SELECT * FROM registered_invariants"
 doc_present='source_text('+q(doc)+') IS NOT NULL'
 contract_rows=[row('PV2-050',message('PV2-050',{'code':'n.invariant','code.repr':'n.invariant_repr'},index=1),'registered_invariants n',doc_present+" AND NOT EXISTS(SELECT 1 FROM doc_codes d WHERE d.__path="+q(doc)+" AND d.code=n.invariant)"),row('PV2-052',message('PV2-052',{'code':'n.invariant','code.repr':'n.invariant_repr'}),'registered_invariants n',doc_present+" AND instr(source_text(n.__path),'('||n.invariant||')')=0")]
 clause_=make_clause('optimizer-invariant-registry',contract_stages,' UNION ALL '.join(contract_rows),partition=False,locations=[verifier,doc]);clause_['relational'].pop('requireNonemptyInput',None)
 clause_['relational']['inputs']['doc_codes']={'evidence':{'as':'d','query':{'regex':{'source':'file.content','pattern':r'`(?P<CODE>(?:scope|schema|effects|cardinality|rewrite)\.[a-z_0-9]+)`'}},'project':{'code':'capture.CODE'}}}
 # Document rows keep their own text witness and share the registry producer.
 stale=message('PV2-051',{'code':'d.code'})
 registry_query=clause_['relational']['query']
 line="1 line"
 missing="SELECT "+q(doc)+" path,0 start_byte,0 end_byte,1 line,'PV2-050' issue,"+message('PV2-050')+" message,'missing-document' identity,0 phase,0 ordering,'' symbol,'' sort_name WHERE NOT ("+doc_present+")"
 stale_query="SELECT d.__path path,d.__start start_byte,d.__end end_byte,1 line,'PV2-051' issue,"+stale+" message,d.code identity,0 phase,0 ordering,'' symbol,'' sort_name FROM doc_codes d WHERE d.__path="+q(doc)+" AND NOT EXISTS(SELECT 1 FROM registered_invariants r WHERE r.invariant=d.code) GROUP BY d.__path,d.code"
 # A document diagnostic reports line one. Its full-file input proves that anchor.
 stale_query=stale_query.replace('d.__start start_byte,d.__end end_byte','0 start_byte,length(CAST(source_text(d.__path) AS BLOB)) end_byte')
 all_query=registry_query+' UNION ALL '+missing+' UNION ALL '+stale_query
 clause_['relational']['query']=select_entry('SELECT * FROM ('+all_query+') WHERE EXISTS(SELECT 1 FROM a WHERE __path='+q(verifier)+' AND kind='+q('Module')+')',{'scan_file':'SELECT * FROM ('+registry_query+') WHERE 0','find_violations':all_query})
 clause_['relational']['line']='line'
 # Use a full document witness as well as each code capture.
 clause_['relational']['inputs']['documents']={'evidence':{'as':'doc','query':{'regex':{'source':'file.content','pattern':r'(?s)\A(?P<TEXT>.*)\z'}},'project':{'path':'file.path'}}}
 return make_document('plan_optimizer_v2',[item,clause_],[*[p+'/**/*.py' for p in c['V2_PACKAGE_DIRS']],doc],exclude=['**/'+part+'/**' for part in ['.git','.venv','venv','__pycache__','tmp','tests','test','.pytest_cache']])
