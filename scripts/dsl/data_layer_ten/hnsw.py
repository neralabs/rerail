# SPDX-License-Identifier: Apache-2.0
from .common import *


def candidate(root):
    c=constants(root/'scripts/lint_hnsw_architecture.py')
    stages=[S('imports',"SELECT n.*,CASE WHEN n.kind='ImportFrom' THEN n.module ELSE (SELECT group_concat(name,',') FROM (SELECT name FROM a alias WHERE alias.__path=n.__path AND alias.parent=n.id AND alias.kind='alias' ORDER BY position)) END imported,(SELECT group_concat(name,' ') FROM (SELECT name FROM a alias WHERE alias.__path=n.__path AND alias.parent=n.id AND alias.kind='alias' ORDER BY position)) imported_names FROM a n WHERE n.kind IN ('Import','ImportFrom')",('__path','id')),
      S('hnsw_symbols',"SELECT n.*,CASE WHEN n.kind IN ('Name','Attribute') THEN n.name WHEN n.kind='Constant' AND n.value_kind='str' THEN n.value WHEN n.kind IN ('Import','ImportFrom') THEN i.imported||' '||i.imported_names ELSE '' END token FROM a n LEFT JOIN imports i ON i.__path=n.__path AND i.id=n.id",('__path','id'))]
    docstring="EXISTS(SELECT 1 FROM a expr JOIN a owner ON owner.__path=expr.__path AND owner.id=expr.parent WHERE expr.__path=n.__path AND expr.id=n.parent AND expr.kind='Expr' AND expr.field='body' AND expr.position=0 AND owner.kind IN ('Module','ClassDef','FunctionDef','AsyncFunctionDef'))"
    # The sqljson arm accepts an empty suffix, so its Boolean check needs no suffix scan.
    # Scope case folding to literals; it must not change CPython's word character set.
    json_token='(?:(?i:sqljson)|'+python_word('(?i:jsonb?)(?:_'+PY_WORD+'+)?')+')'
    stages.append(S('python_json_pattern','SELECT '+q(json_token)+' pattern'))
    queries=[row('HNSW007',q('HNSW must use indexed SQL rows, not JSON operations'),'hnsw_symbols n',"(instr(lower(rx_replace(n.__path,'^.*/','')),'hnsw')>0 OR instr('/'||n.__path||'/','/sql_table_hnsw/')>0) AND NOT (n.kind='Constant' AND "+docstring+") AND rx_matches(n.token,(SELECT pattern FROM python_json_pattern))"),
      row('HNSW001',"'forbidden pseudo-HNSW symbol: '||n.name",where=f"n.kind IN ('Name','Attribute') AND n.name IN {literals(c['_FORBIDDEN_NAMES'])}"),
      row('HNSW001',"'forbidden pseudo-HNSW definition: '||n.name",where=f"n.kind IN ('ClassDef','FunctionDef','AsyncFunctionDef') AND n.name IN {literals(c['_FORBIDDEN_NAMES'])}"),
      row('HNSW003',"'algorithm imports backend implementation: '||n.imported",'imports n',"rx_replace(n.__path,'^.*/','')='algorithm.py' AND ("+' OR '.join('instr(n.imported,'+q(x)+')>0' for x in c['_FORBIDDEN_ALGORITHM_IMPORT_PARTS'])+')'),
      row('HNSW004',q('algorithm defines a custom render method'),where="rx_replace(n.__path,'^.*/','')='algorithm.py' AND n.kind='FunctionDef' AND n.name='render'"),
      row('HNSW005',q('algorithm constructs SQL fragments instead of query-builder nodes'),'calls n',"rx_replace(n.__path,'^.*/','')='algorithm.py' AND n.callee_kind='Attribute' AND n.receiver_kind='Name' AND n.receiver_name='SqlFragment'")]
    for i,marker in enumerate(c['_FORBIDDEN_TEXT']):queries.append(row('HNSW002',q('forbidden pseudo-HNSW state: '+marker),where="n.kind='Constant' AND n.value_kind='str' AND instr(n.value,"+q(marker)+')>0',order=f'n.id+{i+1}*0.001'))
    parameters=S('hnsw_parameters',{'$option':'parameters','escape':'sql','template':'SELECT {} data'})
    stages.insert(0,parameters)
    clause_=make_clause('hnsw-architecture',stages,'SELECT * FROM ('+' UNION ALL '.join(queries)+") WHERE NOT (instr('/'||path||'/','/rust_bridge/')>0 AND substr(rx_replace(path,'^.*/',''),1,10)='generated_')")
    gate=clause_['relational']['inputs']['gate']['evidence']['query']['ast']
    original=gate['rule'];triggered(clause_,['hnsw','Hnsw','HNSW','json','Json','JSON']);gate['rule']=select_entry(gate['rule'],{'find_hnsw_architecture_findings':original})
    forbidden=['impl/common/l2100_sql_grammar/domain/sql_table_hnsw/'+name for name in ['frontier.py','normalized_vectors.py','recovery_schema.py','search_program.py','sqlite_construction.py','sqlite_frontier.py']]
    # The standalone API checks file existence before parsing; the configured plugin does not.
    inventory="hnsw_result.path IN "+literals(forbidden)+" AND NOT EXISTS(SELECT 1 FROM hnsw_parameters,json_each(data,'$.cwdExternalPaths') WHERE value=hnsw_result.path)"
    # Transport path context lost when absolute host names become snapshot
    # paths. SQL still decides membership in the forbidden-file inventory.
    clause_['relational']['stages'].insert(0,parameters)
    original=clause_['relational']['query'];clause_['relational']['query']=select_entry(original,{'find_hnsw_architecture_findings':'SELECT * FROM ('+original+') hnsw_result WHERE NOT ('+inventory+')'})
    files={'evidence':{'as':'file','query':{'regex':{'source':'file.content','pattern':r'(?s)\A(?P<TEXT>.*)\z'}},'project':{'path':'file.path'}}}
    query="SELECT * FROM (SELECT __path path,0 start_byte,0 end_byte,1 line,'HNSW006' issue,'forbidden HNSW module exists' message,__path identity,0 phase,0 ordering,'' symbol,'' sort_name FROM files) hnsw_result WHERE "+inventory
    inventory_clause={'id':'hnsw-forbidden-files','relational':{'inputs':{'files':files},'stages':[parameters],'query':select_entry('SELECT * FROM ('+query+') WHERE 0',{'find_hnsw_architecture_findings':query}),'key':['identity'],'code':'issue','message':'message'},'violation':{'code':'HNSW006','message':'Forbidden HNSW module','anchor':'witness.span'}}
    return make_document('hnsw_architecture',[clause_,inventory_clause],['impl/common/**/*.py','l5200_vector_index/**/*.py','vector_types.py'])
