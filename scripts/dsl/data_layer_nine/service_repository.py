# SPDX-License-Identifier: Apache-2.0
from .common import *

BROAD="substr(__path,1,17)='l2000_connection/' OR substr(__path,1,30)='impl/common/l2200_sql_dialect/' OR __path IN ('impl/sqlite/connection.py','impl/postgres/connection.py','impl/mariadb/connection.py','impl/common/l5150_fts_polyfill/manager.py')"
SQLFILE='impl/common/l2200_sql_dialect/sql_connection.py'
BACKENDS={'impl/sqlite/connection.py':'_SqliteConnectionCore','impl/postgres/connection.py':'_PostgresConnectionCore','impl/mariadb/connection.py':'_MariaDBConnectionCore'}

def stages(c):
 required=[]
 for owner_rank,(owner,names) in enumerate(c['REQUIRED_SQL_SERVICE_METHODS'].items()):
  required.extend('('+','.join([str(owner_rank),q(owner),q(name)])+')' for name in sorted(names))
 return [S('required_services','SELECT column1 ordinal,column2 owner,column3 method FROM (VALUES '+','.join(required)+')'),
 S('expected_backends','SELECT column1 ordinal,column2 path,column3 owner FROM (VALUES '+','.join(f'({i},{q(p)},{q(owner)})' for i,(p,owner) in enumerate(BACKENDS.items()))+')'),
 S('service_configuration',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}),
 S('file_order',"SELECT value path,CAST(key AS INTEGER) ordinal FROM service_configuration,json_each(data,'$.fileOrder')",('path',)),
 S('broad_owners',{'$option':'entryPoint','cases':{'all':'SELECT * FROM owners WHERE '+BROAD},'default':'SELECT * FROM owners'}),
 S('cross_owner_groups',"""SELECT name,count(DISTINCT __path||':'||class_name) distinct_owners,group_concat(display_path||':'||class_name,', ') names FROM (
 SELECT o.*,coalesce(json_extract(p.data,'$.displayPaths.'||json_quote(o.__path)),o.__path) display_path FROM broad_owners o CROSS JOIN service_configuration p
 LEFT JOIN file_order ord ON ord.path=o.__path ORDER BY coalesce(ord.ordinal,2147483647),o.__path,o.id) GROUP BY name""",('name',))]

def sql_services(c):
 where=f"n.kind='Module' AND n.__path={q(SQLFILE)}"
 result=[row('CONN000',q('missing _SqlConnectionCore'),'a n',where+" AND NOT EXISTS(SELECT 1 FROM first_classes c WHERE c.__path=n.__path AND c.name='_SqlConnectionCore')",order='0')]
 forbidden="(SELECT group_concat(name,', ') FROM (SELECT DISTINCT m.name FROM first_classes c JOIN methods m ON m.__path=c.__path AND m.parent=c.id WHERE c.__path=n.__path AND c.name='_SqlConnectionCore' AND m.name IN "+literals(c['FORBIDDEN_SQL_CORE_SCHEMA_METHODS'])+' ORDER BY m.name))'
 result.append(row('CONN000',"'_SqlConnectionCore defines forbidden schema wrappers: '||"+forbidden,'a n',where+' AND '+forbidden+' IS NOT NULL',order='0'))
 source="a n JOIN (SELECT DISTINCT ordinal,owner FROM required_services) r"
 exists="EXISTS(SELECT 1 FROM first_classes c WHERE c.__path=n.__path AND c.name=r.owner)"
 result.append(row('CONN000',"'missing '||r.owner",source,where+' AND NOT '+exists,order='r.ordinal+1'))
 missing="(SELECT group_concat(method,', ') FROM required_services required WHERE required.owner=r.owner AND NOT EXISTS(SELECT 1 FROM first_classes c JOIN methods m ON m.__path=c.__path AND m.parent=c.id WHERE c.__path=n.__path AND c.name=r.owner AND m.name=required.method))"
 result.append(row('CONN000',"r.owner||' missing service-owned methods: '||"+missing,source,where+' AND '+exists+' AND '+missing+' IS NOT NULL',order='r.ordinal+1'))
 return ' UNION ALL '.join(result)

def backend_cores(c):
 source='a n JOIN expected_backends expected ON expected.path=n.__path'
 where="n.kind='Module'"
 exists="EXISTS(SELECT 1 FROM first_classes cls WHERE cls.__path=n.__path AND cls.name=expected.owner)"
 rows=[row('CONN000',"'missing '||expected.owner",source,where+' AND NOT '+exists,order='expected.ordinal*3')]
 direct="(SELECT group_concat(name,', ') FROM (SELECT DISTINCT m.name FROM first_classes cls JOIN methods m ON m.__path=cls.__path AND m.parent=cls.id WHERE cls.__path=n.__path AND cls.name=expected.owner AND m.name!='__init__' ORDER BY m.name))"
 rows.append(row('CONN000',"expected.owner||' regrew direct method bucket: '||"+direct,source,where+' AND '+exists+' AND '+direct+' IS NOT NULL',order='expected.ordinal*3+1'))
 fields=' AND '.join(desc('init',condition="n.kind='Attribute' AND n.name="+q(field)) for field in sorted(c['BACKEND_SERVICE_FIELDS']))
 rows.append(row('CONN000',q('missing backend service assembly in __init__'),source,where+' AND '+exists+" AND NOT EXISTS(SELECT 1 FROM functions init WHERE init.__path=n.__path AND init.name='__init__' AND "+fields+')',order='expected.ordinal*3+2'))
 return ' UNION ALL '.join(rows)

def cross_owners():
 return row('CONN301',"n.name_repr||' has multiple non-delegate owners across connection surface: '||g.names",'broad_owners n JOIN cross_owner_groups g ON g.name=n.name',"g.distinct_owners>1 AND substr(n.class_name,-7)!='Service'",sort_name='n.name')
