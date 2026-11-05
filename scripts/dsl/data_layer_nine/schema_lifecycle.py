# SPDX-License-Identifier: Apache-2.0
"""The configured schema-architecture plugin, distinct from its standalone CLI."""
from .common import *

def clauses(root):
 c=constants(root/'scripts/lint_schema_architecture_plugin.py')
 stages=[S('expression_roots',"""WITH RECURSIVE roots(__path,id,root,name) AS (
 SELECT __path,id,id,name FROM a WHERE kind='Name'
 UNION ALL SELECT parent.__path,parent.id,roots.root,roots.name FROM roots JOIN a parent ON parent.__path=roots.__path JOIN a child ON child.__path=roots.__path AND child.id=roots.id WHERE parent.id=child.parent AND parent.kind='Attribute' AND child.field='value') SELECT * FROM roots""",('__path','id')),
 S('schema_parameters',"""SELECT DISTINCT p.__path,p.fn,p.name FROM parameters p JOIN a annotation ON annotation.__path=p.__path AND annotation.parent=p.id AND annotation.field='annotation' JOIN a term ON term.__path=annotation.__path AND term.id BETWEEN annotation.id AND annotation.last WHERE p.kind='arg' AND p.field IN ('posonlyargs','args','kwonlyargs') AND term.kind IN ('Name','Attribute') AND substr(term.name,-17)='SchemaTransaction'""",('__path','fn','name')),
 S('plan_aliases',"""SELECT DISTINCT f.__path,f.id fn,target.name FROM functions f JOIN a assignment ON assignment.__path=f.__path AND assignment.id BETWEEN f.id AND f.last AND assignment.kind IN ('Assign','AnnAssign') JOIN a value ON value.__path=assignment.__path AND value.parent=assignment.id AND value.field='value' AND value.kind='Attribute' AND value.name='_schema_operation_plan' JOIN expression_roots root ON root.__path=value.__path AND root.id=value.id JOIN schema_parameters param ON param.__path=f.__path AND param.fn=f.id AND param.name=root.name JOIN a target ON target.__path=assignment.__path AND target.parent=assignment.id AND target.field IN ('target','targets') AND target.kind='Name'""",('__path','fn','name')),
 S('executing_functions',f"SELECT DISTINCT f.__path,f.id FROM functions f JOIN calls call ON call.__path=f.__path AND call.id BETWEEN f.id AND f.last WHERE call.callee IN {literals(c['EXECUTION_TERMINALS'])}",('__path','id')),
 S('lifecycle_calls',"SELECT f.__path,f.id fn,f.bfs function_order,f.name function_name,call.*,root.name root_name FROM functions f JOIN calls call ON call.__path=f.__path AND call.id BETWEEN f.id AND f.last LEFT JOIN expression_roots root ON root.__path=call.__path AND root.id=call.receiver",('__path','fn','id'))]
 # Project call identity rather than inheriting duplicate function columns.
 stages[-1]['query']="SELECT call.*,f.id AS function_id,f.bfs function_order,f.name function_name,root.name root_name FROM functions f JOIN calls call ON call.__path=f.__path AND call.id BETWEEN f.id AND f.last LEFT JOIN expression_roots root ON root.__path=call.__path AND root.id=call.receiver"
 rows=[]
 callable_annotation=child('n','annotation',alias='annotation',condition=desc('annotation',alias='term',condition="term.kind IN ('Name','Attribute') AND term.name='Callable'"))
 owns="EXISTS(SELECT 1 FROM a field JOIN a annotation ON annotation.__path=field.__path AND annotation.parent=field.id AND annotation.field='annotation' JOIN a term ON term.__path=annotation.__path AND term.id BETWEEN annotation.id AND annotation.last WHERE field.__path=cls.__path AND field.parent=cls.id AND field.kind='AnnAssign' AND field.field='body' AND term.kind IN ('Name','Attribute') AND term.name='SchemaOperationKind')"
 rows.append(row('SAP001',"'schema operation '||cls.name||' stores callable field '||CASE WHEN target.kind='Name' THEN target.name ELSE '<field>' END",'a cls JOIN a n ON n.__path=cls.__path AND n.parent=cls.id AND n.field=\'body\' LEFT JOIN a target ON target.__path=n.__path AND target.parent=n.id AND target.field=\'target\'',"cls.kind='ClassDef' AND "+owns+" AND n.kind='AnnAssign' AND "+callable_annotation,order="printf('%020d/%03d/%020d',cls.bfs,0,n.position)"))
 hasexec="EXISTS(SELECT 1 FROM executing_functions f WHERE f.__path=n.__path AND f.id=n.function_id)"
 rootparam="EXISTS(SELECT 1 FROM schema_parameters p WHERE p.__path=n.__path AND p.fn=n.function_id AND p.name=n.root_name)"
 mutates="EXISTS(SELECT 1 FROM plan_aliases p WHERE p.__path=n.__path AND p.fn=n.function_id AND p.name=n.root_name) OR "+rootparam+" AND EXISTS(SELECT 1 FROM a receiver JOIN a attr ON attr.__path=receiver.__path AND attr.id BETWEEN receiver.id AND receiver.last WHERE receiver.__path=n.__path AND receiver.id=n.receiver AND attr.kind='Attribute' AND attr.name='_schema_operation_plan')"
 rows.append(row('SAP002',"n.function_name||' mutates schema planning state while executing/materializing it'",'lifecycle_calls n',f"{hasexec} AND n.callee_kind='Attribute' AND n.callee IN {literals(c['MUTATION_METHODS'])} AND ({mutates})",order="printf('%020d/%03d/%020d',n.function_order,0,n.bfs)"))
 rows.append(row('SAP005',"n.function_name||' adds or queries lazy schema work while executing/materializing it'",'lifecycle_calls n',hasexec+" AND n.callee_kind='Attribute' AND substr(n.callee,-5)='_lazy' AND "+rootparam,order="printf('%020d/%03d/%020d',n.function_order,1,n.bfs)"))
 rows.append(row('SAP004',"n.name||' recursively materializes schema work during execution'",'functions n',"EXISTS(SELECT 1 FROM executing_functions f WHERE f.__path=n.__path AND f.id=n.id) AND "+desc('n',table='calls',alias='call',condition="call.callee_kind='Name' AND call.callee=n.name"),order="printf('%020d/%03d/%020d',n.bfs,2,0)"))
 rows.append(row('SAP003',"n.function_name||' stores executable callback behavior on a schema transaction'",'lifecycle_calls n',"n.callee_kind='Attribute' AND "+rootparam+' AND '+child('n','args',condition="v.kind='Lambda'"),order="printf('%020d/%03d/%020d',n.function_order,3,n.bfs)"))
 query=' UNION ALL '.join(rows)
 clause_=clause('schema-lifecycle',stages,{'$option':'entryPoint','cases':{e:query for e in ['all','scan_source','_extract_facts']},'default':'SELECT * FROM ('+query+') WHERE 0'})
 # Fact collectors apply raw triggers; the public scan_source API does not.
 gate=clause_['relational']['inputs']['gate']['evidence']['query']['ast']
 gate['rule']=A(K('module'),{'$option':'entryPoint','cases':{'all':A(RX('SchemaTransaction|Callable|_schema_operation_plan'),{'filePath':{'regex':r'^(?:l2000_connection|l3200_metadata|impl/common/l7245_versioned_backends)/'}})},'default':TRUE})
 return [clause_]
