# SPDX-License-Identifier: Apache-2.0
from .common import *

def candidate(root):
 c=constants(root/'scripts/lint_schema_bootstrap_contracts.py')
 stages=[S('params',"""SELECT p.*,v.id AS annotation_id,f.name AS function_name,f.name_repr AS function_repr,
 CASE p.field WHEN 'posonlyargs' THEN 0 WHEN 'args' THEN 1 WHEN 'kwonlyargs' THEN 2 WHEN 'vararg' THEN 3 ELSE 4 END*10000+p.position AS param_order
 FROM parameters p JOIN functions f ON f.__path=p.__path AND f.id=p.fn
 LEFT JOIN a v ON v.__path=p.__path AND v.parent=p.id AND v.field='annotation' WHERE p.kind='arg'""",('__path','fn'),('__path','id')),
 S('param_types',"""SELECT p.__path,p.id,n.name FROM params p JOIN annotation_names n ON n.__path=p.__path AND n.annotation=p.annotation_id""",('__path','id','name')),
 S('function_annotations',"""SELECT p.__path,p.fn,v.id AS annotation,0 AS category,p.param_order AS ordinal FROM params p JOIN a v ON v.__path=p.__path AND v.id=p.annotation_id
 UNION ALL SELECT v.__path,v.parent,v.id,1,0 FROM a v WHERE v.field='returns'
 UNION ALL SELECT f.__path,f.id,v.id,2,n.bfs FROM functions f JOIN a n ON n.__path=f.__path AND n.id BETWEEN f.id AND f.last AND n.kind='AnnAssign' JOIN a v ON v.__path=n.__path AND v.parent=n.id AND v.field='annotation'""",('__path','fn','annotation')),
 S('forbidden_annotations',f"""SELECT x.__path,x.fn,x.annotation,x.category,x.ordinal,group_concat(n.name,', ') AS names FROM function_annotations x JOIN annotation_names n ON n.__path=x.__path AND n.annotation=x.annotation WHERE n.name IN {literals(c['LAZY_FORBIDDEN_SYNC_ASYNC_TYPE_NAMES'])} GROUP BY x.__path,x.fn,x.annotation,x.category,x.ordinal""",('__path','fn')),
 S('broad_params',f"SELECT p.__path,p.id,group_concat(t.name,', ') AS names FROM params p JOIN param_types t ON t.__path=p.__path AND t.id=p.id WHERE t.name IN {literals(c['BROAD_ANNOTATION_NAMES'])} GROUP BY p.__path,p.id",('__path','id')),
 S('schema_params',f"SELECT p.__path,p.id,group_concat(t.name,', ') AS names FROM params p JOIN param_types t ON t.__path=p.__path AND t.id=p.id WHERE t.name IN {literals(c['SCHEMA_TRANSACTION_FORBIDDEN_SYNC_ASYNC_TYPE_NAMES'])} GROUP BY p.__path,p.id",('__path','id')),
 S('readonly_params',"""SELECT p.__path,p.id FROM params p JOIN a ann ON ann.__path=p.__path AND ann.id=p.annotation_id AND ann.kind='Subscript'
 JOIN a root ON root.__path=ann.__path AND root.parent=ann.id AND root.field='value'
 JOIN a element ON element.__path=ann.__path AND element.parent=ann.id AND element.field='slice'
 WHERE substr(p.function_name,1,1)!='_' AND EXISTS(SELECT 1 FROM a n WHERE n.__path=root.__path AND n.id BETWEEN root.id AND root.last AND n.kind IN ('Name','Attribute') AND n.name='Sequence')
 AND NOT EXISTS(SELECT 1 FROM a n WHERE n.__path=root.__path AND n.id BETWEEN root.id AND root.last AND n.kind IN ('Name','Attribute') AND n.name!='Sequence')
 AND EXISTS(SELECT 1 FROM annotation_names n WHERE n.__path=element.__path AND n.annotation=element.id)
 AND NOT EXISTS(SELECT 1 FROM annotation_names n WHERE n.__path=element.__path AND n.annotation=element.id AND n.name IN ('Any','bytes','bytearray','float','int','object','str'))""",('__path','id'))]
 def flag(names,types,alias='p'):
  return f"{alias}.annotation_id IS NOT NULL AND {alias}.name NOT IN ('self','cls') AND ({alias}.name IN {literals(names)} OR EXISTS(SELECT 1 FROM param_types t WHERE t.__path={alias}.__path AND t.id={alias}.id AND t.name IN {literals(types)}))"
 sql=[row('SBC001',"n.name||'() ends with _lazy but is not decorated with @pure_lazy'",'functions n',"substr(n.name,-5)='_lazy' AND NOT "+pure('n')),
 row('SBC006',"f.name||'() uses sync/async type annotation '||x.names||' inside a lazy function; use LazySchemaTransaction or another lazy-universe contract'",'functions f JOIN forbidden_annotations x ON x.__path=f.__path AND x.fn=f.id JOIN a n ON n.__path=x.__path AND n.id=x.annotation',"substr(f.name,-5)='_lazy'",order='f.id+0.01+x.category*0.01+x.ordinal*0.00000001'),
 row('SBC002',"'parameter '||n.function_name||'('||n.name||') is missing an explicit type annotation'",'params n',"n.name NOT IN ('self','cls') AND n.annotation_id IS NULL",order='n.fn+0.1+n.param_order*0.000001'),
 row('SBC002',"'parameter '||n.function_name||'('||n.name||') uses broad annotation '||b.names||'; use a concrete contract type'",'params n JOIN broad_params b ON b.__path=n.__path AND b.id=n.id',"n.name NOT IN ('self','cls') AND NOT (b.names='Sequence' AND EXISTS(SELECT 1 FROM readonly_params r WHERE r.__path=n.__path AND r.id=n.id))",order='n.fn+0.1+n.param_order*0.000001'),
 row('SBC006',"'parameter '||n.function_name||'('||n.name||') uses sync/async schema transaction type '||b.names||'; use LazySchemaTransaction'",'params n JOIN schema_params b ON b.__path=n.__path AND b.id=n.id',f"n.name IN {literals(c['SCHEMA_TRANSACTION_PARAM_NAMES'])} AND substr(n.function_name,-5)!='_lazy'",order='n.fn+0.1000001+n.param_order*0.000001'),
 row('SBC004',"n.name||'() accepts both connection-like and transaction-like parameters; remove the connection and make the transaction authoritative'",'functions n',"EXISTS(SELECT 1 FROM params p WHERE p.__path=n.__path AND p.fn=n.id AND ("+flag(c['CONNECTION_PARAM_NAMES'],c['CONNECTION_TYPE_NAMES'])+")) AND EXISTS(SELECT 1 FROM params p WHERE p.__path=n.__path AND p.fn=n.id AND ("+flag(c['TRANSACTION_PARAM_NAMES'],c['TRANSACTION_TYPE_NAMES'])+"))",order='n.id+0.9')]
 for code,key,tail in [('SBC003','RUNTIME_ATTRIBUTE_PROBES','use explicit typed attributes or protocols'),('SBC005','RUNTIME_TYPE_PROBES','use concrete typed collaborators instead')]:
  sql.append(row(code,"n.callee||'() is forbidden in schema/bootstrap contract files (current function: '||coalesce(f.name,'<module>')||'); "+tail+"'",'calls n LEFT JOIN functions f ON f.__path=n.__path AND f.id=n.fn',f'n.callee IN {literals(c[key])}'))
 return document('schema_bootstrap_contracts',[clause('bootstrap-contracts',stages,' UNION ALL '.join(sql))],['l2000_connection/schema_transaction.py','l2000_connection/connection_caches.py','l3200_metadata/bootstrap_state.py','impl/common/l2200_sql_dialect/sql_connection.py','impl/common/l2200_sql_dialect/sql_schema_services.py','impl/common/l2200_sql_dialect/sql_versioning_services.py','impl/common/l7245_versioned_backends/generic_sparse.py','l7000_versioned_schema/physical_keys.py'])
