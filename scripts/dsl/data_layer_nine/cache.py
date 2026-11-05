# SPDX-License-Identifier: Apache-2.0
from .common import *

def candidate(root):
 c=constants(root/'scripts/lint_bootstrap_cache_health.py');helpers=literals(c['BOOTSTRAP_HELPERS'])
 # CST calls give the original source-line indices used by the upstream collector.
 stages=[{'name':'call_lines','pythonAst':{'syntax':'cst','roots':"SELECT __path,id FROM cst WHERE kind='call'",'fields':['source_line','previous_line']}},
 S('cache_calls',f"""SELECT c.*,CASE WHEN substr(f.name,1,5)='test_' OR EXISTS(SELECT 1 FROM a d JOIN qualified q ON q.__path=d.__path AND q.id=d.id WHERE d.__path=f.__path AND d.parent=f.id AND d.field='decorator_list' AND (q.name='fixture' OR substr(q.name,-14)='pytest.fixture')) THEN 1 ELSE 0 END AS inside_test
 FROM calls c LEFT JOIN functions f ON f.__path=c.__path AND f.id=c.fn
 JOIN cst s ON s.__path=c.__path AND s.kind='call' AND s.__start=c.__start AND s.__end=c.__end
 JOIN call_lines l ON l.__path=s.__path AND l.id=s.id
 WHERE instr(l.source_line,{q(c['ALLOW_COMMENT'])})=0 AND instr(l.previous_line,{q(c['ALLOW_COMMENT'])})=0""",('__path','id'))]
 queries=[row('BCH001',"n.callee||'() inside a test/fixture can create repeated cold bootstraps; prefer module-level TEST_BOOTSTRAP_TABLES or TEST_BOOTSTRAP_VERSIONING_DOMAINS'",'cache_calls n',f'n.inside_test AND n.callee IN {helpers}'),
 row('BCH002',"n.callee||'() uses an explicit manifest_key; per-test manifest keys commonly cause BOOT-WARM-CACHE-MISS'",'cache_calls n',f'n.callee IN {helpers} AND '+child('n','keywords',condition="v.name='manifest_key'"),order='n.id+0.0001'),
 row('BCH003',"n.callee||'() inside a conform test usually causes a warm schema mutation; prefer module-level TEST_BOOTSTRAP_TABLES'",'cache_calls n',f"n.inside_test AND n.callee_kind='Name' AND n.callee IN {literals(c['CREATE_TABLES_HELPERS'])}")]
 item=document('bootstrap_cache_health',[source_triggers(clause('bootstrap-cache',stages,' UNION ALL '.join(queries)),c['BOOTSTRAP_HELPERS']|c['CREATE_TABLES_HELPERS'])],['conform/**/*.py'])
 from .parser_errors import clause_
 item['clauses'].append(clause_('BCH000','SyntaxError: ',A(item['clauses'][1]['relational']['inputs']['gate']['evidence']['query']['ast']['rule'],{'$option':'entryPoint','cases':{'all':FALSE},'default':TRUE})))
 item['scope']['exclude']=['**/'+part+'/**' for part in ['.git', '.venv', 'venv', '__pycache__', 'tmp']]
 return item
