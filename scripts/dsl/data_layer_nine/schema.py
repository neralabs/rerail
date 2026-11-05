# SPDX-License-Identifier: Apache-2.0
from .common import *

def candidate(root):
 c=constants(root/'scripts/lint_schema_architecture.py')
 queries=[row('forbid-schema-planning-callback-field',q('Represent schema ordering with artifact requirements, not executable callback fields.'),where="n.kind='AnnAssign' AND "+child('n','target',condition="v.kind IN ('Name','Attribute') AND v.name='planning_callback'"),symbol=q('planning_callback')),
 row('forbid-schema-callback-queues',q('Declare schema artifacts and dependencies before finalization instead of queueing callbacks.'),'calls n',"n.callee IN ('enqueue_post_ddl','enqueue_post_schema_ddl_flush','enqueue_post_commit_schema_exit','planning_callback')",symbol='n.callee'),
 row('forbid-materialization-plan-mutation',q('Schema materialization may consume a frozen plan only; it cannot mutate or recursively flush it.'),'calls n JOIN functions f ON f.__path=n.__path AND f.id=n.fn',"instr(f.name,'materialize_schema_operation_records')>0 AND n.callee IN ('add_operation','_materialize_schema_operation_records','_materialize_schema_operation_records_async','flush_schema_plan','flush_schema_plan_async')",symbol='n.callee')]
 standalone=source_triggers(clause('schema-planning',[],' UNION ALL '.join(queries)),['planning_callback','materialize_schema_operation_records','enqueue_post_'])
 standalone['relational']['query']={'$option':'entryPoint','cases':{'lint_file':standalone['relational']['query'],'standalone':standalone['relational']['query']},'default':'SELECT * FROM ('+' UNION ALL '.join(queries)+') WHERE 0'}
 from .schema_lifecycle import clauses
 item=document('schema_architecture',[standalone,*clauses(root)],['l2000_connection/**/*.py','l3200_metadata/**/*.py','impl/common/l7245_versioned_backends/**/*.py',*list(c['TARGETS'])])
 from .parser_errors import clause_
 item['clauses'].append(clause_('SAP000','cannot inspect schema lifecycle ownership: SyntaxError: ',A(K('module'),RX('SchemaTransaction|Callable|_schema_operation_plan'),{'filePath':{'regex':r'^(?:l2000_connection|l3200_metadata|impl/common/l7245_versioned_backends)/'}},{'$option':'entryPoint','cases':{'all':TRUE},'default':FALSE})))
 item['scope']['exclude']=['**/'+part+'/**' for part in ['.git','.venv','venv','__pycache__','tmp','tests','test']]
 return item
