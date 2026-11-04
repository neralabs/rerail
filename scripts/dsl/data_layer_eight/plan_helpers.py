# SPDX-License-Identifier: Apache-2.0
"""Resolve versioned helper aliases and exact lexical callsites in batched SQL."""
from data_layer_six.common import S,q,literals,choice
from data_layer_generate_batch_seven import constant
from data_layer_eight.relations import clause
from data_layer_eight.plan_names import module_sql,path_scope


def candidate(source,policy):
    module=source/'scripts/lint_plan_helpers_allowlist.py'
    value=lambda name:constant(module,name)
    public=set(value('PUBLIC_TARGET_HELPERS'));private=set(value('PRIVATE_TERMINAL_HELPERS'));lazy=set(value('ALLOWED_LAZY_TERMINALS'));legacy=set(value('LEGACY_PRIVATE_TERMINAL_HELPERS'))
    tracked=public|private|lazy|legacy
    p=policy('plan_helpers_allowlist',['**/*.py'])
    p['clauses'].append({'id':'entry','define':{'options':{'oracleAnalyzer':{'type':'boolean','default':False},'entryPoint':{'type':'string','default':'scan_repo'}}}})
    stages=[
      S('helper_names',' UNION ALL '.join('SELECT '+q(n)+' name,'+str(int(n in public))+' public,'+str(int(n in lazy))+' lazy,'+str(int(n in legacy))+' legacy' for n in sorted(tracked)),('name',)),
      S('imports',"""SELECT n.__path,n.id,n.__start,n.__end,n.module_name_source module,x.id alias_id,coalesce(x.name_source,x.text) name,coalesce(x.alias_source,x.name_source,x.text) local FROM nodes n JOIN nodes x ON x.__path=n.__path AND x.parent=n.id WHERE n.kind IN ('import_statement','import_from_statement') AND x.kind IN ('aliased_import','dotted_name') AND x.id IS NOT n.module_name_kind||':'||n.module_name_start||':'||n.module_name_end""",('__path','local')),
      S('module_aliases',"SELECT __path,__start,CASE WHEN module IS NULL AND name='impl.common.l2070_server_plan.plan_helpers' AND local=name THEN 'plan_helpers' ELSE local END name FROM imports WHERE module='impl.common.l2070_server_plan' AND name='plan_helpers' OR module IS NULL AND name='impl.common.l2070_server_plan.plan_helpers'",('__path','name')),
      S('target_names',"""WITH RECURSIVE targets(__path,assignment,node) AS (
         SELECT __path,id,target FROM assignments UNION ALL SELECT t.__path,t.assignment,n.id FROM targets t JOIN nodes p ON p.__path=t.__path AND p.id=t.node JOIN nodes n ON n.__path=p.__path AND n.parent=p.id WHERE p.kind IN ('pattern_list','tuple_pattern','list_pattern','tuple','list'))
         SELECT t.__path,t.assignment,n.id,n.__start,n.__end,n.text name FROM targets t JOIN nodes n ON n.__path=t.__path AND n.id=t.node WHERE n.kind='identifier'""",('__path','assignment')),
      S('bindings',"SELECT i.__path,i.alias_id id,i.local name,i.name helper,NULL expr,i.__start at FROM imports i WHERE i.module='impl.common.l2070_server_plan.plan_helpers' AND (i.name IN (SELECT name FROM helper_names) OR i.name GLOB '_*via_plan*') UNION ALL SELECT t.__path,t.id,t.name,NULL,a.value,a.__start FROM target_names t JOIN assignments a ON a.__path=t.__path AND a.id=t.assignment WHERE a.value IS NOT NULL",('__path','name','at')),
      S('requests',"SELECT __path,id,callee_id expr,__start at FROM calls UNION ALL SELECT __path,id,value,__start FROM assignments WHERE value IS NOT NULL",('__path','id')),
      S('resolved',"""WITH RECURSIVE lookup(__path,request,node,at,helper,priority) AS (
        SELECT __path,id,expr,at,NULL,'' FROM requests
        UNION ALL
        SELECT w.__path,w.request,b.expr,b.at,b.helper,w.priority||printf('%020d/',b.at+1) FROM lookup w JOIN nodes n ON n.__path=w.__path AND n.id=w.node JOIN bindings b ON b.__path=n.__path AND b.name=n.text AND b.at<w.at WHERE w.helper IS NULL AND n.kind='identifier'
        UNION ALL
        SELECT w.__path,w.request,NULL,w.at,n.text,w.priority||'00000000000000000000/' FROM lookup w JOIN nodes n ON n.__path=w.__path AND n.id=w.node WHERE w.helper IS NULL AND n.kind='identifier' AND n.text IN (SELECT name FROM helper_names)
        UNION ALL
        SELECT w.__path,w.request,NULL,w.at,a.attr,w.priority||'00000000000000000000/' FROM lookup w JOIN attributes a ON a.__path=w.__path AND a.id=w.node JOIN nodes o ON o.__path=a.__path AND o.id=a.object WHERE w.helper IS NULL AND (a.attr IN (SELECT name FROM helper_names) OR a.attr GLOB '_*via_plan*' AND o.kind='identifier' AND EXISTS(SELECT 1 FROM module_aliases m WHERE m.__path=o.__path AND m.name=o.text AND m.__start<w.at)))
        SELECT __path,request,helper FROM (SELECT __path,request,helper,row_number() OVER(PARTITION BY __path,request ORDER BY priority DESC) chosen FROM lookup WHERE helper IS NOT NULL) WHERE chosen=1""",('__path','request')),
      S('modules',"""SELECT __path,replace(substr(trim(replace(__path,char(92),'/'),'/'),1,length(trim(replace(__path,char(92),'/'),'/'))-3),'/','.') raw FROM nodes WHERE kind='module'""",('__path',)),
      S('module_names','SELECT __path,'+module_sql('__path')+' name FROM modules',('__path',)),
      S('regions',"SELECT __path,id,name,__start,__end,'class' kind FROM classes UNION ALL SELECT f.__path,f.id,f.name,coalesce(d.__start,f.__start),f.__end,'function' FROM functions f LEFT JOIN nodes d ON d.__path=f.__path AND d.id=f.parent AND d.kind='decorated_definition'",('__path',)),
      S('raw_issues',"""SELECT i.__path,i.__start,i.__end,i.alias_id identity,'import' kind,i.name helper,i.local FROM imports i WHERE i.module='impl.common.l2070_server_plan.plan_helpers' AND (i.name IN (SELECT name FROM helper_names WHERE public OR lazy) OR i.name GLOB '_*via_plan*' OR i.name IN (SELECT name FROM helper_names WHERE legacy))
       UNION ALL SELECT t.__path,t.__start,t.__end,t.id,'alias',r.helper,t.name FROM target_names t JOIN resolved r ON r.__path=t.__path AND r.request=t.assignment
       UNION ALL SELECT c.__path,c.__start,c.__end,c.id,'call',r.helper,NULL FROM calls c JOIN resolved r ON r.__path=c.__path AND r.request=c.id"""),
      S('issues',"""SELECT i.*,m.name module,
       m.name||'::'||coalesce((SELECT group_concat(name,'.') FROM (SELECT name FROM regions r WHERE r.__path=i.__path AND r.__start<=i.__start AND r.__end>=i.__end ORDER BY kind,__start)),'<module>') callsite,
       CASE WHEN i.kind='call' THEN 'old or private '||helper||'() call is forbidden'
        WHEN helper IN (SELECT name FROM helper_names WHERE public) OR helper GLOB '_*via_plan*' OR i.kind='import' AND helper IN (SELECT name FROM helper_names WHERE legacy) THEN 'old or private '||helper||' '||i.kind||' is forbidden'
        WHEN helper IN (SELECT name FROM helper_names WHERE lazy) AND rx_matches(lower(local),'execute|fetch|scalar|ddl') THEN 'lazy terminal '||helper||CASE WHEN kind='import' THEN ' imported under eager alias' ELSE ' assigned to eager alias' END END reason
       FROM raw_issues i JOIN module_names m ON m.__path=i.__path WHERE NOT (i.kind='alias' AND helper IN (SELECT name FROM helper_names WHERE legacy)) AND NOT (i.kind='call' AND m.name='impl.common.l2070_server_plan.plan_helpers' AND (helper GLOB '_*via_plan*' OR helper IN (SELECT name FROM helper_names WHERE legacy)))"""),
    ]
    # Diagnostic identity is the original reason for direct scanner observations.
    base="SELECT __path path,__start start_byte,__end end_byte,{code} issue,identity,callsite FROM issues WHERE reason IS NOT NULL AND callsite NOT IN "+literals(value('ALLOWLISTED_CALLSITES'))
    discovery=' AND '+path_scope('__path',value('SKIP_PARTS'))
    direct=base.format(code='reason')
    query={'$option':'oracleAnalyzer','cases':{'true':{'$option':'entryPoint','cases':{'scan_repo':direct+discovery},'default':direct},'false':base.format(code="'PHA001'")+discovery}}
    # The generic builder needs string SQL for dependency and column analysis.
    item=clause('plan-helper-aliases',stages,direct)
    item['relational']['query']=query;p['clauses'].append(item)
    return p
