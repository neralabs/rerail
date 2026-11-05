# SPDX-License-Identifier: Apache-2.0
from .common import *

def candidate(root,alias=False):
 c=constants(root/'scripts/lint_bootstrap_lazy_sql.py')
 stages=[S('iterations',"""WITH RECURSIVE it(__path,root,id) AS (
 SELECT n.__path,n.id,v.id FROM a n JOIN a v ON v.__path=n.__path AND v.parent=n.id AND v.field='iter' WHERE n.kind IN ('For','AsyncFor')
 UNION ALL SELECT it.__path,it.root,v.id FROM it JOIN calls c ON c.__path=it.__path AND c.id=it.id
 JOIN a v ON v.__path=c.__path AND v.parent=c.id AND v.field='args' AND v.position=0 WHERE c.callee_kind='Name' AND c.callee IN ('list','tuple','set','sorted'))
 SELECT n.*,v.name AS iter_name,v.name_repr AS iter_repr FROM it JOIN a v ON v.__path=it.__path AND v.id=it.id JOIN a n ON n.__path=it.__path AND n.id=it.root WHERE v.kind='Name'""",('__path','id'))]
 qexec="n.callee_kind='Attribute' AND n.callee='execute_lazy' AND EXISTS(SELECT 1 FROM calls r WHERE r.__path=n.__path AND r.id=n.receiver AND r.callee_kind='Attribute' AND r.callee='query_lazy')"
 queries=[row('BLB-001',q('bootstrap full query_lazy(...).execute_lazy() must move to proof/keyed SQL before hard-fail'),'calls n',qexec,order='n.bfs'),
 row('BLB-004',"'bootstrap eager schema probe '||n.callee||' must route through BootstrapScopeProof providers'",'calls n',f'n.callee IN {literals(c["EAGER_SCHEMA_PROBE_NAMES"])}',order='n.bfs'),
 row('BLB-003',"'bootstrap/server-plan Python model materializer '||n.callee||' must stay out of lowering paths'",'calls n',"n.callee_kind='Attribute' AND n.callee IN ('model_dump','model_copy')",order='n.bfs'),
 row('BLB-002',"'bootstrap Python row iteration over '||n.iter_repr||' must move to SQL/proof-owned matching'",'iterations n',f'n.iter_name IN {literals(c["ROW_COLLECTION_NAMES"])}',order='n.bfs')]
 query='SELECT * FROM ('+' UNION ALL '.join(queries)+") WHERE instr(path,'bootstrap')>0"
 item=document('bootstrap_eager_resolve' if alias else 'bootstrap_lazy_sql',[clause('bootstrap-sql',stages,query)],list(c['DEFAULT_BOOTSTRAP_PATHS']))

 item['mode']='log'
 return item
