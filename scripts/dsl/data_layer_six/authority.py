# SPDX-License-Identifier: Apache-2.0
"""Execution authority checks over source scopes and authored callsite inventories."""
from .common import *
import json,re

def authority_clause(m):
 declaration=lambda kind:A(K(kind),P('DECL'),F('name',P('NAME')),optional(I(A(K('decorated_definition'),P('WRAPPER')),'neighbor')))
 # ast.Name includes reads and writes, but not declaration/import/keyword labels.
 identifier=A(K('identifier'),names(m.PRIVATE_AUTH_SYMBOLS),
  N(I(O(K('import_statement'),K('import_from_statement')))),
  *[N(I(K(kind),'neighbor',field=field)) for kind,field in [('attribute','attribute'),('keyword_argument','name'),('function_definition','name'),('class_definition','name'),('default_parameter','name'),('typed_default_parameter','name')]],
  N(I(O(K('parameters'),K('list_splat_pattern'),K('dictionary_splat_pattern')),'neighbor')),
  N(A(I(K('typed_parameter'),'neighbor'),{'nthChild':{'position':1,'ofRule':N(K('comment'))}})))
 private=O(identifier,A(K('attribute'),F('attribute',names(m.PRIVATE_AUTH_SYMBOLS))))
 string=lambda pattern:A(O(K('string'),K('concatenated_string')),RX(r'''(?i)^[rub]*["']'''),{'value':{'regex':pattern}})
 arg=lambda n,rule:F('arguments',H(A(rule,{'nthChild':{'position':n,'ofRule':N(K('comment'))}})))
 marker='(?:'+'|'.join(re.escape(x) for x in m.AUTH_MONKEYPATCH_MARKERS)+')'
 monkey=A(K('call'),F('function',A(K('attribute'),F('attribute',names(['setattr'])))),O(arg(1,string(marker)),
  A(arg(2,string('^(?:'+'|'.join(re.escape(x) for x in sorted(m.PRIVATE_AUTH_SYMBOLS|{'_caller_token','verify_callsite_authorization'}))+')$')),
    arg(1,O(names(['execution_boundary']),A(K('attribute'),F('attribute',names(['execution_boundary']))))))))
 # Frames matter only in files holding a candidate site; every site's text names one of these
 # literals, so the frame inputs carry them as a file-level text requirement.
 site_text=I(A(K('module'),RX('_legacy_boundary_var|_explicit_authority_var|explicit_execution_authority|execution_boundary')))
 declaration_in=lambda kind:A(declaration(kind),site_text)
 inputs={
  'raw_functions':E(declaration_in('function_definition'),{'name':'capture.NAME','wrapper':'capture.WRAPPER.startByte'}),
  'raw_classes':E(declaration_in('class_definition'),{'name':'capture.NAME','wrapper':'capture.WRAPPER.startByte'}),
  'private_symbols':E(private,{'path':'file.path'}),
  'explicit_calls':E(call(['explicit_execution_authority']),{'path':'file.path'}),
  'monkeypatches':E(monkey,{'path':'file.path'}),
 }
 def inventory(name,pairs,fields):
  return S(name,'SELECT '+','.join(f"json_extract(value,'$[{i}]') AS {field}" for i,field in enumerate(fields))+f' FROM json_each({q(json.dumps(sorted(pairs),separators=(",",":")))})',tuple(fields))
 stages=[
  S('frames',"SELECT __path,__start,__end,coalesce(wrapper,__start) AS lexical_start,name,1 AS is_function FROM raw_functions UNION ALL SELECT __path,__start,__end,coalesce(wrapper,__start),name,0 FROM raw_classes",('__path','__start'),('__path','lexical_start')),
  S('parents',"""SELECT f.*,(SELECT p.__start FROM frames p WHERE p.__path=f.__path AND p.__start<f.__start AND p.__end>=f.__end ORDER BY p.__start DESC LIMIT 1) AS parent FROM frames f""",('__path','parent'),('__path','__start')),
  S('contexts',"""WITH RECURSIVE scopes(path,start,qualname,depth) AS (
 SELECT __path,__start,name,is_function FROM parents WHERE parent IS NULL UNION ALL
 SELECT c.__path,c.__start,p.qualname || CASE WHEN c.is_function AND p.depth>0 THEN '.<locals>.' ELSE '.' END || c.name,p.depth+c.is_function
 FROM scopes p CROSS JOIN parents c ON c.__path=p.path AND c.parent=p.start)
 SELECT * FROM scopes""",('path','start')),
  S('sites',"""SELECT r.*,coalesce((SELECT c.qualname FROM frames f JOIN contexts c ON c.path=f.__path AND c.start=f.__start
 WHERE f.__path=r.__path AND f.lexical_start<=r.__start AND f.__end>=r.__end ORDER BY f.__start DESC LIMIT 1),'<module>') AS qualname,
 replace(rx_replace(r.__path,'\\.py$',''),'/','.') AS module FROM (
 SELECT *, 'AUTH001' AS issue FROM private_symbols UNION ALL SELECT *,'AUTH002' FROM explicit_calls UNION ALL SELECT *,'AUTH003' FROM monkeypatches) r"""),
  inventory('private_exceptions',m.PRIVATE_SYMBOL_TEST_EXCEPTIONS,['path','qualname']),
  inventory('explicit_exceptions',m.EXPLICIT_AUTHORITY_TEST_EXCEPTIONS,['path','qualname']),
  inventory('signed_callsites',m._signed_authority_callsites(),['module','qualname']),
 ]
 query="""SELECT __path AS path,__start AS start_byte,__end AS end_byte,issue FROM sites s WHERE
 (issue='AUTH001' AND __path<>'l2050_transaction/execution_boundary.py' AND NOT EXISTS(SELECT 1 FROM private_exceptions e WHERE e.path=s.__path AND e.qualname=s.qualname))
 OR (issue='AUTH002' AND NOT EXISTS(SELECT 1 FROM signed_callsites e WHERE e.module=s.module AND e.qualname=s.qualname) AND NOT EXISTS(SELECT 1 FROM explicit_exceptions e WHERE e.path=s.__path AND e.qualname=s.qualname))
 OR (issue='AUTH003' AND NOT EXISTS(SELECT 1 FROM private_exceptions e WHERE e.path=s.__path AND e.qualname=s.qualname))"""
 return {'id':'authority-sites','relational':{'inputs':inputs,'stages':stages,'query':query,'key':['issue'],'code':'issue','partitionBy':'file'},
  'violation':{'code':'AUTH001','message':'Execution authority boundary violation','anchor':'witness.span'}}
