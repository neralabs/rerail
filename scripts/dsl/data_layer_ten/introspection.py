# SPDX-License-Identifier: Apache-2.0
from .common import *

def data_path_predicate(c,column='path'):
    return '('+' OR '.join('substr('+column+',1,'+str(len(x))+')='+q(x) for x in c['DATA_PATH_PREFIXES'])+') AND NOT ('+' OR '.join("instr('/'||"+column+","+q(x)+')>0' for x in c['ALLOWED_PATH_MARKERS'])+')'

def candidate(root):
    c=constants(root/'scripts/lint_schema_introspection_data_paths.py')
    parameters=S('schema_approval_parameters',{'$option':'parameters','escape':'sql','template':'SELECT {} data'})
    entries=S('schema_approval_entries',"SELECT CAST(key AS INTEGER) ordinal,value entry FROM schema_approval_parameters,json_each(data,'$.schemaApprovals.entries')")
    path="rx_extract(entry,'\\A([^:]*)',1)"
    errors=S('schema_approval_errors',"SELECT ordinal*2 ordering,entry||': approved schema checks must not be internal impl callsites' message FROM schema_approval_entries WHERE substr("+path+",1,5)='impl/' UNION ALL SELECT ordinal*2+1,entry||': approved schema checks must be production user-facing callsites' FROM schema_approval_entries WHERE substr("+path+",1,8)='scripts/' OR instr("+path+",'/tests/')>0")
    state=S('schema_approval_state',"SELECT NOT EXISTS(SELECT 1 FROM schema_approval_errors) AND (NOT EXISTS(SELECT 1 FROM schema_approval_entries) OR coalesce(json_extract(data,'$.schemaApprovals.signatureVerified'),0)=1) approved FROM schema_approval_parameters")
    approval_stages=[parameters,entries,errors,state]
    authority='EXISTS(SELECT 1 FROM functions owner WHERE owner.__path=n.__path AND n.id BETWEEN owner.id AND owner.last AND ('+' OR '.join('instr(lower(owner.name),'+q(x)+')>0' for x in c['SCHEMA_AUTHORITY_NAME_MARKERS'])+'))'
    primitive="EXISTS(SELECT 1 FROM functions f WHERE f.__path=n.__path AND f.id=n.fn AND n.__path||':'||f.name||':'||n.callee IN "+literals(c['PRIMITIVE_DEFINITION_CALLS'])+')'
    rows=[row('SIDP001',"'schema plan flush call '||n.callee||' in versioned backend path; use schema operation projections instead of materializing DDL mid-bootstrap'",'calls n',f"n.callee IN {literals(c['FORBIDDEN_SCHEMA_FLUSH_CALL_NAMES'])} AND ("+' OR '.join('substr(n.__path,1,'+str(len(x))+')='+q(x) for x in c['SCHEMA_FLUSH_FORBIDDEN_PREFIXES'])+')',extra=',n.callee call_name'),
      row('SIDP001',"'schema introspection call '||n.callee||' in runtime data path'",'calls n',f"n.callee IN {literals(c['FORBIDDEN_CALL_NAMES'])} AND NOT {authority} AND NOT {primitive}",order='n.id+0.001',extra=',n.callee call_name')]
    for i,marker in enumerate(c['FORBIDDEN_SQL_TEXT']):rows.append(row('SIDP001',q('schema introspection SQL marker '+repr(marker)+' in runtime data path'),where="n.kind='Constant' AND n.value_kind='str' AND instr(lower(n.value),"+q(marker)+")>0 AND NOT "+authority+" AND NOT EXISTS(SELECT 1 FROM a expr WHERE expr.__path=n.__path AND expr.id=n.parent AND expr.kind='Expr')",order=f'n.id+{i}*0.001',extra=','+q(marker)+' call_name'))
    query=' UNION ALL '.join(rows)
    enabled=data_path_predicate(c)
    permitted="(SELECT approved FROM schema_approval_state)=1 AND NOT EXISTS(SELECT 1 FROM schema_approval_entries WHERE entry=path||':'||line||':'||call_name)"
    item=make_clause('schema-introspection',approval_stages,select_entry('SELECT * FROM ('+query+') WHERE '+enabled+' AND '+permitted,{'_collect_violations':query}))
    original=item['relational']['inputs']['gate']['evidence']['query']['ast']['rule']
    triggered(item,[*c['FORBIDDEN_CALL_NAMES'],*c['FORBIDDEN_SCHEMA_FLUSH_CALL_NAMES'],*c['FORBIDDEN_SQL_TEXT']])
    item['relational']['inputs']['gate']['evidence']['query']['ast']['rule']=select_entry(item['relational']['inputs']['gate']['evidence']['query']['ast']['rule'],{'_collect_violations':original})
    configuration_path='scripts/lint_schema_introspection_data_paths.py'
    prefix="SELECT "+q(configuration_path)+" path,0 start_byte,0 end_byte,1 line,'SIDP000' issue,"
    preflight=prefix+"message,'entry-'||ordering identity,ordering FROM schema_approval_errors UNION ALL "+prefix+"'schema introspection data-path approval signature verification failed' message,'signature' identity,0 ordering FROM schema_approval_state WHERE approved=0 AND NOT EXISTS(SELECT 1 FROM schema_approval_errors)"
    approval_clause={'id':'schema-approval-configuration','relational':{'inputs':{'files':{'evidence':{'as':'file','query':{'regex':{'source':'file.content','pattern':r'(?s)\A.*\z'}},'project':{'path':'file.path'}}}},'locations':[configuration_path],'stages':approval_stages,'query':select_entry(preflight,{'_collect_violations':'SELECT * FROM ('+preflight+') WHERE 0'}),'key':['identity'],'line':'line','code':'issue','message':'message'},'violation':{'code':'SIDP000','message':'Invalid schema approvals','anchor':'witness.span'}}
    return make_document('schema_introspection_data_paths',[item,approval_clause],[p+'**/*.py' for p in c['DATA_PATH_PREFIXES']])
