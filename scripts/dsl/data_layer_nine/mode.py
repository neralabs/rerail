# SPDX-License-Identifier: Apache-2.0
from .common import *

def candidate(root):
 c=constants(root/'scripts/lint_connection_mode_split.py');old=literals(c['OLD_PUBLIC_NAMES']);private=literals(c['PRIVATE_CORE_NAMES'])
 allowed='n.__path IN '+literals(c['ALLOWED_PRIVATE_CORE_FILES'])+' OR '+ ' OR '.join('substr(n.__path,1,'+str(len(p))+')='+q(p) for p in sorted(c['ALLOWED_PRIVATE_CORE_ROOTS']))
 stages=[S('symbols',f"""SELECT n.*,CASE WHEN n.kind='Attribute' THEN 'attribute' ELSE 'name' END AS label,n.name AS symbol,n.name_repr AS quoted,n.id AS ordering FROM a n WHERE n.kind IN ('Name','Attribute')
 UNION ALL SELECT i.*, 'name',CASE WHEN i.kind='Import' THEN substr(s.name,length(rtrim(s.name,replace(s.name,'.','')))+1) ELSE s.name END,s.name_repr,i.id+s.position*0.001 FROM a i JOIN a s ON s.__path=i.__path AND s.parent=i.id AND s.kind='alias'
 UNION ALL SELECT i.*, 'name',s.alias,''' '||s.alias||'''',i.id+s.position*0.001+0.0001 FROM a i JOIN a s ON s.__path=i.__path AND s.parent=i.id AND s.kind='alias' WHERE s.alias!=''""",('__path','id'))]
 # Imported basenames and aliases are valid Python identifiers. Format their repr
 # after normalization, rather than reusing the whole qualified import spelling.
 stages[0]['query']=stages[0]['query'].replace("''' '||s.alias||''''","''''||s.alias||''''")
 sql=[row('connection_mode_split',"'old mixed connection class '||n.name_repr||' is forbidden'",where=f"n.kind='ClassDef' AND n.name IN {old}"),
 row('connection_mode_split',"'old mixed connection '||n.label||' '||CASE WHEN n.kind IN ('Import','ImportFrom') THEN ''''||n.symbol||'''' ELSE n.quoted END||' is forbidden'",'symbols n',f'n.symbol IN {old}',order='n.ordering'),
 row('connection_mode_split',"'private connection core '||CASE WHEN n.kind IN ('Import','ImportFrom') THEN ''''||n.symbol||'''' ELSE n.quoted END||' is not allowed here'",'symbols n',f'n.symbol IN {private} AND NOT ({allowed})',order='n.ordering')]
 for cond,msg in [
 (f"c.name IN {literals(c['PRIVATE_HANDLE_CORE_NAMES'])} AND m.name IN {literals(c['FORBIDDEN_PRIVATE_HANDLE_CORE_METHODS'])}","'private handle core '||c.name||' defines forbidden mixed entrypoint '||m.name_repr"),
 ("substr(c.name,1,4)='Sync' AND m.kind='AsyncFunctionDef' AND substr(m.name,1,1)!='_'","'sync facade '||c.name||' defines public async method '||m.name_repr"),
 (f"substr(c.name,1,5)='Async' AND m.kind='FunctionDef' AND substr(m.name,1,1)!='_' AND substr(m.name,-6)!='_async' AND m.name NOT IN {literals(c['ASYNC_METHODS_ALLOWED_ON_ASYNC_FACADES'])}","'async facade '||c.name||' defines public sync method '||m.name_repr")]:
  if 'facade' in msg:cond+=" AND (substr(c.name,-10)='Connection' OR substr(c.name,-13)='SessionHandle' OR substr(c.name,-13)='VersionHandle')"
  sql.append(row('connection_mode_split',msg,'a c JOIN methods m ON m.__path=c.__path AND m.parent=c.id',cond,node='m',order='c.id+m.position*0.001+0.0001'))
 stages.append(S('mode_configuration',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}))
 trigger_pattern='|'.join(sorted(c['OLD_PUBLIC_NAMES']|c['PRIVATE_CORE_NAMES']|c['PRIVATE_HANDLE_CORE_NAMES']|{'Async','Connection','SessionHandle','Sync','VersionHandle'}))
 query="SELECT * FROM ("+' UNION ALL '.join(sql)+") WHERE EXISTS(SELECT 1 FROM mode_configuration p WHERE json_extract(p.data,'$.probeSource') IS NULL OR rx_matches(json_extract(p.data,'$.probeSource'),"+q(trigger_pattern)+'))'
 item=document('connection_mode_split',[source_triggers(clause('mode-split',stages,query),c['OLD_PUBLIC_NAMES']|c['PRIVATE_CORE_NAMES']|c['PRIVATE_HANDLE_CORE_NAMES']|{'Async','Connection','SessionHandle','Sync','VersionHandle'})],[p+'/**/*.py' for p in c['SCAN_ROOTS']])
 item['scope']['exclude']=['**/'+part+'/**' for part in ['.git', '.venv', 'venv', '__pycache__', 'tmp', 'tests', 'test']]
 return item
