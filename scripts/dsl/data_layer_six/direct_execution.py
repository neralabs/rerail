# SPDX-License-Identifier: Apache-2.0
"""Direct execution line deduplication with ordinary literal span masking."""
from .common import *
import re

def direct_execution_clause(m):
 methods=sorted(m._DIRECT_EXECUTION_ATTR_PATTERNS)
 pattern=r'(?P<TOKEN>\.(?P<METHOD>'+ '|'.join(re.escape(v) for v in methods)+r')\()'
 inputs={
  'raw_tokens':E(A(K('module'),RX(pattern)),{'line':'capture.TOKEN.startLine','method':'capture.METHOD'},each='TOKEN',anchor='capture.TOKEN'),
  'raw_calls':E(A(K('call'),P('CALL'),F('function',A(K('attribute'),F('attribute',A(names(methods),P('METHOD')))))),{'line':'capture.CALL.startLine','method':'capture.METHOD'}),
  'raw_strings':E(A(O(A(K('string'),RX(r'''(?i)^[rub]*["']''')),K('string_content')),RX(pattern)),{'path':'file.path'}),
  'full_comments':E(A(K('comment'),RX(pattern)),{'path':'file.path'}),
 }
 allowed='(?:'+'|'.join(['(?:^|/)'+re.escape(p)+'$' for p in sorted(m.ALLOWED_FILES)]+['(?:^|/)'+p for p in m.ALLOWED_DIR_PATTERNS])+')'
 fts=r'(?:^|/)impl/[^/]+/fts/.*\.py$'
 stages=[S('strings','SELECT * FROM raw_strings',('__path','__start')),S('comments','SELECT * FROM full_comments',('__path','__start')),
  S('matches',f"""SELECT *,rx_matches(__path,{q(fts)}) AS fts,rx_matches(__path,{q(allowed)}) AS allowed FROM (
 SELECT * FROM raw_calls UNION ALL SELECT * FROM raw_tokens t
 WHERE NOT EXISTS(SELECT 1 FROM strings s WHERE s.__path=t.__path AND s.__start<=t.__start AND s.__end>t.__start)
 AND NOT EXISTS(SELECT 1 FROM comments s WHERE s.__path=t.__path AND s.__start<=t.__start AND s.__end>t.__start))""")]
 qfull="""SELECT __path AS path,__start AS start_byte,__end AS end_byte,line FROM (
 SELECT *,row_number() OVER(PARTITION BY __path,line ORDER BY __start,__end) AS selected FROM matches
 WHERE NOT allowed OR (fts AND method IN ('_execute_direct_internal','_execute_direct_internal_async'))) WHERE selected=1"""
 qfts="""SELECT __path AS path,__start AS start_byte,__end AS end_byte,line FROM (
 SELECT *,row_number() OVER(PARTITION BY __path,line ORDER BY __start,__end) AS selected FROM matches
 WHERE fts AND method IN ('_execute_direct_internal','_execute_direct_internal_async')) WHERE selected=1"""
 query={'$option':'entryPoint','cases':{e:qfts for e in ['scan_fts_manager_source','scan_fts_manager_direct_execution']},'default':qfull}
 return {'id':'direct-execution-lines','relational':{'inputs':inputs,'stages':stages,'query':query,'key':['line']},
  'violation':{'code':analyzer_code('IDE001','direct_execution'),'message':'Direct database execution outside ServerPlan','anchor':'witness.span'}}
