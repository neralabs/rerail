#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate native API contracts from the production rule's shared SQL."""
import argparse
import copy
import json
from pathlib import Path
import sys
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from data_layer_generate_batch_seven import constant
from data_layer_six.common import S,q,literals
from data_layer_eight.plan_names import module_sql,path_scope,recognized
from data_layer_eight.relations import clause


def source_contract(source, name, projection):
    module=__import__('data_layer_eight.'+name,fromlist=['candidate'])
    captured={};original=module.clause
    def capture(identifier,stages,query,inputs=None):
        captured[identifier]=(copy.deepcopy(stages),copy.deepcopy(inputs))
        return original(identifier,stages,query,inputs)
    from data_layer_generate_batch_seven import policy
    with patch.object(module,'clause',capture):module.candidate(source,policy)
    stages,inputs=next(iter(captured.values()))
    # A fact batch is anchored to its actual source document. Each exported
    # fact carries its own native line in the payload, including non-node maps.
    query="SELECT result.path,document.__start start_byte,document.__end end_byte,result.identity,result.relation,result.payload FROM ("+projection+") result JOIN nodes document ON document.__path=result.path AND document.kind='module'"
    item=clause('native-facts',stages,query,inputs)
    item['relational'].pop('partitionBy',None)
    item['relational'].pop('code',None)
    item['relational']['key']=['identity']
    return {'format':'rerail.policy/v1','id':'native-'+name+'-facts','revision':1,'mode':'log','clauses':[item]}


def lazy_facts(source):
    query="""SELECT f.__path path,f.__start start_byte,f.__end end_byte,'function:'||f.id identity,'function' relation,
      json_object('order',coalesce((SELECT group_concat(piece,'') FROM (SELECT printf('%020d/1/',d.region_start) piece FROM declarations d WHERE d.__path=f.__path AND d.kind='class_definition' AND d.region_start<f.region_start AND d.__end>=f.__end ORDER BY d.region_start)),'')||printf('%020d',f.region_start),'id',f.id,'symbol',f.symbol,'qualname',f.qualname,'name',f.name,'line',f.line+1,
        'scope',coalesce(cls.qualname,''),'is_async',rx_matches(f.text,'^async\\b'),
        'is_lazy',f.pure OR f.name GLOB '*_lazy','has_pure_lazy_decorator',f.pure,
        'has_legacy_boundary_decorator',f.legacy,'is_public',f.name NOT GLOB '_*','is_tombstone',f.tombstone,'is_execution_context_method',f.execution_context) payload
      FROM function_flags f LEFT JOIN declaration_contexts cls ON cls.__path=f.__path AND cls.id=f.lexical_parent
      UNION ALL SELECT f.__path,f.__start,f.__end,'field:'||f.id,'function',
      json_object('order',coalesce((SELECT group_concat(piece,'') FROM (SELECT printf('%020d/',d.region_start)||CASE WHEN d.qualname=f.scope THEN '0/' ELSE '1/' END piece FROM declaration_contexts d WHERE d.__path=f.__path AND d.kind='class_definition' AND d.region_start<f.__start AND d.__end>=f.__end ORDER BY d.region_start)),'')||printf('%020d',f.__start),'id',f.id,'symbol',replace(substr(f.__path,1,length(f.__path)-3),'/','.')||'.'||f.qualname,'qualname',f.qualname,'name',f.name,'line',f.line+1,'scope',f.scope,'is_async',0,'is_lazy',1,'has_pure_lazy_decorator',1,'has_legacy_boundary_decorator',0,'is_public',0,'is_tombstone',0,'is_execution_context_method',0) FROM side_queue_fields f
      UNION ALL SELECT f.__path,f.__start,f.__end,'field-event:'||f.id,'event',json_object('order',printf('%020d',f.__start),'fn',f.id,'category','bootstrap','detail','forbidden side-queue declaration '||f.name,'line',f.line+1) FROM side_queue_fields f
      UNION ALL SELECT e.__path,e.__start,e.__end,'event:'||row_number() OVER(ORDER BY e.__path,e.__start,e.__end DESC,e.category),
      'event',json_object('order',coalesce((SELECT o.visit_key FROM owned_nodes o WHERE o.__path=e.__path AND o.fn=e.fn AND o.__start=e.__start AND o.__end=e.__end LIMIT 1),printf('%020d/%020d',e.__start,2147483647-e.__end)),'fn',e.fn,'category',e.category,'detail',e.detail,'event_kind',e.event_kind,'uses_conn',e.uses_conn,
        'line',(SELECT min(n.line)+1 FROM nodes n WHERE n.__path=e.__path AND n.__start=e.__start)) FROM function_events e
      UNION ALL SELECT t.__path,t.__start,t.__start,'call:'||t.id,'callee',json_object('order',t.visit_key,'fn',t.fn,'target',t.target,
        'line',(SELECT min(n.line)+1 FROM nodes n WHERE n.__path=t.__path AND n.__start=t.__start)) FROM call_targets t WHERE target IS NOT NULL
      UNION ALL SELECT t.__path,t.__start,t.__end,'delegate:'||row_number() OVER(ORDER BY t.__path,t.__start,t.keyword_start),'delegate',json_object('order',printf('%020d',t.keyword_start),'fn',t.fn,'target',t.target,'line',(SELECT min(n.line)+1 FROM nodes n WHERE n.__path=t.__path AND n.__start=t.__start)) FROM delegate_targets t WHERE target IS NOT NULL
      UNION ALL SELECT i.__path,i.__start,i.__end,'import:'||row_number() OVER(ORDER BY i.__path,i.__start,i.local),'import',json_object('kind',i.kind,'module',i.module,'name',i.name,'local',i.local)
      FROM imports i JOIN nodes n ON n.__path=i.__path AND n.id=i.id JOIN nodes p ON p.__path=n.__path AND p.id=n.parent AND p.kind='module'
      UNION ALL SELECT a.__path,a.__start,a.__end,'binding:'||a.id,'binding',json_object('name',a.target_text,'source',a.expr_text)
      FROM assignments a JOIN nodes n ON n.__path=a.__path AND n.id=a.parent JOIN nodes p ON p.__path=n.__path AND p.id=n.parent AND p.kind='module' WHERE a.target_kind='identifier' AND a.type_start IS NULL
      UNION ALL SELECT a.__path,a.__start,a.__start,'alias:'||a.cls||':'||a.name||':'||a.__start,'class_alias',json_object('class',a.qualname,'name',a.name,'target',a.target) FROM class_method_aliases a"""
    return source_contract(source,'lazy',query)


def metadata(stages, query):
    return {'format':'rerail.policy/v1','id':'native-helper-contract','revision':1,'mode':'log','clauses':[
      {'id':'parameters','define':{'options':{'parameters':{'type':'string','default':'{}'}}}},
      {'id':'contract','relational':{
        'inputs':{'empty':{'evidence':{'as':'file','query':{'ast':{'language':'python','rule':{'kind':'module'}}},'project':{'path':'file.path'}}}},
        'stages':[S('parameters',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}),*stages],
        'locations':['metadata'],'query':"SELECT 'metadata' path,0 start_byte,0 end_byte,'result' identity,"+query,'key':['identity']},
       'violation':{'code':'ROW','message':'Native helper result','anchor':'witness.span'}}]}


def plan_metadata(source):
    module=source/'scripts/lint_plan_helpers_allowlist.py';value=lambda key:constant(module,key)
    tracked=set().union(*(value(name) for name in ('PUBLIC_TARGET_HELPERS','PRIVATE_TERMINAL_HELPERS','ALLOWED_LAZY_TERMINALS','LEGACY_PRIVATE_TERMINAL_HELPERS')))
    stages=[S('helper_names',' UNION ALL '.join('SELECT '+q(name)+' name' for name in sorted(tracked))),
      S('request',"SELECT json_extract(data,'$.path') path,json_extract(data,'$.name') name,json_extract(data,'$.kind') kind,json_extract(data,'$.receiver') receiver,data FROM parameters"),
      S('names','SELECT r.*,'+module_sql('r.path')+" module,coalesce((SELECT group_concat(value,'.') FROM (SELECT value FROM json_each(r.data,'$.classes') UNION ALL SELECT value FROM json_each(r.data,'$.functions'))),'<module>') context FROM request r"),
      S('values_',"SELECT n.*,module||'::'||context callsite,CASE WHEN kind='identifier' THEN coalesce((SELECT value FROM json_each(n.data,'$.imports') WHERE key=n.name AND "+recognized('value','1')+"),CASE WHEN name IN (SELECT name FROM helper_names) THEN name END) WHEN kind='attribute' AND "+recognized('name',"EXISTS(SELECT 1 FROM json_each(n.data,'$.module_aliases') WHERE value=n.receiver)")+" THEN name END helper FROM names n")]
    return metadata(stages,'path input_path,module,context,callsite,helper,('+path_scope('path',value('SKIP_PARTS'))+') should_scan,callsite IN '+literals(value('ALLOWLISTED_CALLSITES'))+' allowlisted FROM values_')


def memoized_facts(source):
    from data_layer_eight.cached import memoized
    query="""SELECT n.__path path,n.__start start_byte,n.__end end_byte,f.function identity,
      json_object('function',f.function,'path',f.__path,'line',n.line+1,'max_entries',b.max_entries,
        'body_hash',f.body_sha256,'signature_hash',f.signature_sha256,
        'errors',(SELECT json_group_array(error) FROM (SELECT error FROM memoized_errors e WHERE e.__path=f.__path AND e.fn=f.id ORDER BY error))) payload
      FROM actual_functions f JOIN nodes n ON n.__path=f.__path AND n.id=f.id JOIN memoized_bounds b ON b.__path=f.__path AND b.id=f.id
      UNION ALL SELECT n.__path,n.__start,n.__end,a.function,
      json_object('function',a.function,'path',a.__path,'line',n.line+1,'max_entries',NULL,'body_hash',h.dump_sha256,'signature_hash',h.dump_sha256,'errors',json_array(a.error))
      FROM memoized_aliases a JOIN nodes n ON n.__path=a.__path AND n.id=a.id JOIN alias_fingerprints h ON h.__path=a.__path AND h.id=a.id"""
    item=memoized(source,query);item['relational'].pop('code',None);item['relational']['key']=['identity']
    return {'format':'rerail.policy/v1','id':'native-memoized-discovery','revision':1,'mode':'log','clauses':[item]}


def runtime_facts(source):
    from data_layer_eight.cached import runtime_contract
    item=runtime_contract(source)
    item['relational']['query']="SELECT 'l2055_lazy_result/cached_pure_lazy_policy.py' path,0 start_byte,0 end_byte,'runtime' identity,actual hash,(SELECT json_group_array(name) FROM (SELECT name FROM protected_names EXCEPT SELECT name FROM runtime_nodes ORDER BY name)) missing FROM runtime_hash"
    item['relational'].pop('message',None)
    item['relational'].pop('code',None);item['relational']['key']=['identity']
    return {'format':'rerail.policy/v1','id':'native-runtime-fingerprint','revision':1,'mode':'log','clauses':[item]}


def adapter_facts(source):
    query="""SELECT f.__path path,f.__start start_byte,f.__end end_byte,f.id identity,'function' relation,
      json_object('name',f.name,'class_name',f.class_name,'line',f.line+1,'is_async',rx_matches(f.text,'^async\\b'),
        'decorators',(SELECT json_group_array(name) FROM (SELECT name FROM decorators d WHERE d.__path=f.__path AND d.fn=f.id ORDER BY d.__start)),
        'legacy_target',CASE WHEN f.legacy_decorator IS NOT NULL THEN f.target END,'adapter_stub',s.adapter,'abstract_stub',s.abstract,
        'transaction_parameters',(SELECT json_group_array(name) FROM (SELECT name FROM transaction_parameters p WHERE p.__path=f.__path AND p.fn=f.id ORDER BY name)),
        'transaction_calls',(SELECT json_group_array(callee) FROM (SELECT c.callee FROM owned_calls c JOIN nodes receiver ON receiver.__path=c.__path AND receiver.id=c.receiver AND receiver.kind='identifier' JOIN transaction_parameters p ON p.__path=c.__path AND p.fn=c.fn AND p.name=receiver.text WHERE c.__path=f.__path AND c.fn=f.id ORDER BY c.__start)),
        'called_attributes',(SELECT json_group_array(callee) FROM (SELECT callee FROM owned_calls c WHERE c.__path=f.__path AND c.fn=f.id ORDER BY c.__start)),
        'has_await',EXISTS(SELECT 1 FROM nodes n WHERE n.__path=f.__path AND n.kind='await' AND n.__start>=f.region_start AND n.__end<=f.__end)) payload
      FROM function_targets f JOIN stubs s ON s.__path=f.__path AND s.id=f.id"""
    return source_contract(source,'adapters',query)


def eager_facts(source):
    return source_contract(source,'eager',"SELECT f.__path path,f.__start start_byte,f.__end end_byte,f.id identity,'wrapper' relation,json_object('name',f.name,'line',f.line+1,'family',w.family) payload FROM function_info f LEFT JOIN canonical_wrappers w ON w.__path=f.__path AND w.id=f.id")


def eager_metadata(source):
    import re
    from data_layer_eight.contracts import constants
    from data_layer_eight.eager_contracts import ignored_sql
    values=constants(source/'scripts/lint_eager_resolve.py')
    vals=lambda *names:literals(set().union(*(values[name] for name in names)))
    prefixes=lambda name:q('^(?:'+'|'.join(map(re.escape,sorted(values[name])))+')')
    public=constant(source/'l2070_batching/allowlist_signing.py','PUBLIC_KEY_HEX')
    name=values['EXTERNAL_IO_BOUNDARY_ITEM_LINTER_NAME']
    stages=[S('request',"SELECT json_extract(data,'$.path') path,json_extract(data,'$.pattern') pattern,json_extract(data,'$.symbol') symbol,json_extract(data,'$.text') text,data FROM parameters"),
      S('approved_external',"SELECT value symbol FROM request,json_each(data,'$.approved')"),
      S('verified',"SELECT CAST(key AS INTEGER) ordinal,json_extract(value,'$.symbol') symbol,json_extract(value,'$.line') line,verify_ed25519("+q(public)+",json_object('allowlist',json_array(json_extract(value,'$.symbol')),'linter',"+q(name)+",'version',1),coalesce(json_extract(value,'$.signature'),'')) valid FROM request,json_each(data,'$.entries')")]
    return metadata(stages,'('+ignored_sql('r.path','r.pattern','r.symbol','r.text',vals,prefixes)+") ignored,(SELECT json_group_array(json_object('symbol',symbol,'line',line,'valid',valid)) FROM (SELECT * FROM verified ORDER BY ordinal)) entries FROM request r")


def lazy_graph():
    stages=[S('request','SELECT data FROM parameters'),
      S('local_facts',"SELECT CAST(key AS INTEGER) ordinal,value data,json_extract(value,'$.symbol') symbol FROM request,json_each(data,'$.local')"),
      S('global_facts',"SELECT value data,json_extract(value,'$.symbol') symbol FROM request,json_each(data,'$.global_')"),
      S('resolved',"SELECT l.symbol source,c.key ordinal,json_extract(c.value,'$[0]') line,coalesce((SELECT symbol FROM local_facts f WHERE f.symbol=json_extract(c.value,'$[1]')),(SELECT symbol FROM global_facts f WHERE f.symbol=json_extract(c.value,'$[1]')),(SELECT g.symbol FROM local_facts f JOIN global_facts g ON g.symbol=f.symbol WHERE json_array_length(json_extract(f.data,'$.scope'))=0 AND json_extract((SELECT data FROM request),'$.module')||'.'||json_extract(f.data,'$.name')=json_extract(c.value,'$[1]') ORDER BY f.ordinal DESC LIMIT 1)) target FROM local_facts l,json_each(l.data,'$.calls') c"),
      S('target_rows',"SELECT l.symbol,json((SELECT json_group_array(json_array(line,target)) FROM (SELECT line,target FROM resolved r WHERE r.source=l.symbol AND r.target IS NOT NULL AND r.target!=l.symbol ORDER BY r.ordinal))) calls FROM local_facts l"),
      S('edges',"SELECT s.key source,CAST(c.key AS INTEGER) ordinal,json_extract(c.value,'$[0]') line,json_extract(c.value,'$[1]') target FROM request,json_each(data,'$.calls') s,json_each(s.value) c"),
      S('reach',"WITH RECURSIVE walk(node,first,order_,seen) AS (SELECT json_extract(data,'$.root'),NULL,'',json_array(json_extract(data,'$.root')) FROM request UNION ALL SELECT e.target,coalesce(w.first,e.line),w.order_||printf('%010d.',e.ordinal),substr(w.seen,1,length(w.seen)-1)||','||substr(json_array(e.target),2) FROM walk w JOIN edges e ON e.source=w.node JOIN request r WHERE NOT EXISTS(SELECT 1 FROM json_each(w.seen) WHERE value=e.target) AND NOT EXISTS(SELECT 1 FROM json_each(r.data,'$.visiting') WHERE value=e.target) AND NOT EXISTS(SELECT 1 FROM json_each(r.data,'$.targets') WHERE value=w.node) AND EXISTS(SELECT 1 FROM json_each(r.data,'$.facts') WHERE value=w.node)) SELECT * FROM walk")]
    return metadata(stages,"(SELECT json_group_object(symbol,json(calls)) FROM target_rows) targets,(SELECT first FROM reach,request r WHERE first IS NOT NULL AND EXISTS(SELECT 1 FROM json_each(r.data,'$.targets') WHERE value=reach.node) ORDER BY order_ LIMIT 1) line FROM request")


def lazy_analyzed(source):
    from data_layer_eight.analyzed_facts import program
    stages,query=program(source)
    result=metadata(stages,"0 unused")
    result['clauses'][1]['relational']['query']=query
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps({'plan-metadata':plan_metadata(args.source),'lazy-facts':lazy_facts(args.source),'memoized-discovery':memoized_facts(args.source),'runtime-fingerprint':runtime_facts(args.source),'adapter-facts':adapter_facts(args.source),'eager-facts':eager_facts(args.source),'eager-metadata':eager_metadata(args.source),'lazy-graph':lazy_graph(),'lazy-analyzed':lazy_analyzed(args.source)},indent=2)+'\n')


if __name__=='__main__':main()
