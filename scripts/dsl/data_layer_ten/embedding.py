# SPDX-License-Identifier: Apache-2.0
from .common import *


def candidate(root):
    c=constants(root/'scripts/lint_embedding_vector_orchestration.py');worker=q(c['EMBEDDING_WORKER_REL_PATH'])
    stages=[S('conn_receivers',"WITH RECURSIVE r(__path,id) AS (SELECT __path,id FROM a WHERE kind='Name' AND name IN ('conn','connection') OR kind='Attribute' AND name IN ('conn','connection','_conn') UNION ALL SELECT c.__path,c.id FROM r JOIN calls c ON c.__path=r.__path AND c.receiver=r.id AND c.callee_kind='Attribute' AND c.callee='session') SELECT * FROM r",('__path','id')),
      S('txn_receivers',"SELECT __path,id FROM a WHERE kind IN ('Name','Attribute') AND (substr(name,-3)='txn' OR kind='Attribute' AND name='_txn')",('__path','id')),
      S('query_chains',"WITH RECURSIVE r(__path,origin,id) AS (SELECT __path,id,receiver FROM calls WHERE callee_kind='Attribute' UNION ALL SELECT r.__path,r.origin,c.receiver FROM r JOIN calls c ON c.__path=r.__path AND c.id=r.id AND c.callee_kind='Attribute') SELECT r.__path,r.origin,c.id,c.receiver FROM r JOIN calls c ON c.__path=r.__path AND c.id=r.id WHERE c.callee='query' AND c.callee_kind='Attribute'",('__path','origin','id')),
      S('embedding_calls',"SELECT n.*,coalesce(f.qualified,f.name,'<module>') qualname FROM calls n LEFT JOIN qual_functions f ON f.__path=n.__path AND f.id=n.fn",('__path','id'))]
    conn="EXISTS(SELECT 1 FROM conn_receivers receiver WHERE receiver.__path=n.__path AND receiver.id=n.receiver)"
    txn="EXISTS(SELECT 1 FROM txn_receivers receiver WHERE receiver.__path=n.__path AND receiver.id=n.receiver)"
    messages=[('EVO001','new connection-owned orchestration in embedding/vector scope is forbidden',f"n.callee_kind='Attribute' AND n.callee IN {literals(c['CONNECTION_ENTRYPOINTS'])} AND {conn} AND n.__path||'::'||n.qualname NOT IN {literals(c['GRANDFATHERED_ORCHESTRATION_CALLSITES'])}"),
      ('EVO106','connection transaction entrypoints are forbidden in embedding worker modules; use _dispatch_maybe_versioned_* / planned lazy bridges',f"n.__path IN {literals(c['WORKER_CONN_TRANSACTION_FORBIDDEN_PATHS'])} AND n.callee_kind='Attribute' AND n.callee IN {literals(c['CONNECTION_TRANSACTION_ENTRYPOINTS'])} AND {conn}"),
      ('EVO101','worker txn.query(...).terminal() is suspect; prefer lazy/serverplan composition',f"n.__path={worker} AND n.callee_kind='Attribute' AND n.callee IN {literals(c['QUERY_TERMINALS'])} AND EXISTS(SELECT 1 FROM query_chains chain JOIN txn_receivers receiver ON receiver.__path=chain.__path AND receiver.id=chain.receiver WHERE chain.__path=n.__path AND chain.origin=n.id AND chain.id=(SELECT min(first.id) FROM query_chains first WHERE first.__path=n.__path AND first.origin=n.id))"),
      ('EVO105','embedding_worker must use txn.query_lazy(...), not txn.query(...)',f"n.__path={worker} AND n.callee_kind='Attribute' AND n.callee='query' AND {txn}"),
      ('EVO107','embedding_worker must use next_lazy_plan_ref(...) instead of direct plan._next_ref()',f"n.__path={worker} AND n.callee_kind='Attribute' AND n.callee='_next_ref'"),
      ('EVO108','embedding_worker may publish server steps only inside dedicated _publish_* helper functions',f"n.__path={worker} AND n.callee_kind='Attribute' AND n.callee='_add_server_step' AND NOT EXISTS(SELECT 1 FROM functions f WHERE f.__path=n.__path AND f.id=n.fn AND f.name IN {literals(c['WORKER_SERVER_STEP_HELPER_NAMES'])})")]
    rows=[row(code,q(message),'embedding_calls n',condition,order=f'n.id+{i}*0.001',extra=',n.qualname') for i,(code,message,condition) in enumerate(messages)]
    rows.append(row('EVO104',q('forbidden hybrid lazy/sync worker function name (see embedding_worker_lazy_intents.md)'),'functions n',f"n.__path={worker} AND ("+' OR '.join('substr(n.name,-'+str(len(x))+')='+q(x) for x in c['FORBIDDEN_WORKER_FUNCTION_NAME_SUFFIXES'])+')',extra=',n.name qualname'))
    rows.append(row('EVO102',q('*_lazy worker body must not call .resolve() / .resolve_async(); use sync/async sibling'),'embedding_calls n JOIN functions lazy ON lazy.__path=n.__path AND n.id BETWEEN lazy.id AND lazy.last',f"n.__path={worker} AND n.callee_kind='Attribute' AND n.callee IN ('resolve','resolve_async') AND substr(lazy.name,-5)='_lazy' AND lazy.id=(SELECT max(f.id) FROM functions f WHERE f.__path=n.__path AND n.id BETWEEN f.id AND f.last AND substr(f.name,-5)='_lazy')",order='n.id+0.006',extra=',lazy.name qualname'))
    query=' UNION ALL '.join(rows)
    default='SELECT path,start_byte,end_byte,line,issue,qualname||\': \'||message message,identity,phase,ordering,symbol,sort_name,qualname FROM ('+query+') WHERE issue IN '+literals(c['FAILING_CODES'])
    item=make_clause('embedding-orchestration',stages,select_entry(default,{entry:query for entry in ['scan_source','scan_file','_violations_for_files']}))
    triggered(item,c['EMBEDDING_VECTOR_ORCHESTRATION_EXTRACTOR'].source_triggers if hasattr(c.get('EMBEDDING_VECTOR_ORCHESTRATION_EXTRACTOR'),'source_triggers') else ['.read','.write','.query','.resolve','_lazy','query_lazy','versioned_','_add_server_step','_next_ref'])
    return make_document('embedding_vector_orchestration',[item],[*(p+'/**/*.py' for p in c['SCOPED_DIRECTORIES']),*c['SCOPED_FILES']])
