# SPDX-License-Identifier: Apache-2.0
"""Run the architecture consumer on a supplied, complete function-fact batch."""
import json,re
from data_layer_six.common import S,q
from data_layer_eight.lazy import architecture_program
from data_layer_eight.lazy_paths import classifications
from data_layer_eight.contracts import constants

EVENTS={
 'plan':'plan_io_calls','grammar':'grammar_calls','intent':'intent_calls','native':'imperative_native_calls','dolt':'dolt_strings',
 'context':'execution_context_terminals','internal':'lazy_internal_reads','state':'execution_state_writes','private':'private_runtime_reachthroughs',
 'forbidden_pure':'forbidden_pure_lazy_operations','payload':'serverplan_service_callable_payloads','dynamic':'serverplan_dynamic_behavior_dispatches',
 'of_return':'lazy_result_of_returns','bootstrap':'bootstrap_lazy_state_mutations','reconciliation':'reconciliation_eager_surface_calls','absence':'schema_fact_absence_fallbacks',
}


def program(source):
    stages,_,_,query=architecture_program(source)
    values=constants(source/'scripts/lint_lazy_architecture.py')
    flags="SELECT f.*,"+classifications('f',values)+" FROM supplied_functions f"
    events=[]
    for channel,(kind,field) in enumerate(EVENTS.items()):
        line='event.value' if kind=='of_return' else "json_extract(event.value,'$[0]')"
        detail=q('LazyResult.of') if kind=='of_return' else "json_extract(event.value,'$[1]')"
        events.append("SELECT f.__path,f.id fn,"+q(kind+':')+"||event.key id,"+line+" __start,"+line+" __end,"+q(kind)+" category,"+detail+" detail,"+str(channel)+" channel,printf('%020d',CAST(event.key AS INTEGER)) visit_key,"+("json_extract(event.value,'$[2]')" if kind=='plan' else '0')+" uses_conn,"+("json_extract(event.value,'$[2]')" if kind=='reconciliation' else 'NULL')+" event_kind FROM function_flags f,json_each(f.data,"+q('$.'+field)+") event")
    overrides={
      'function_flags':S('function_flags',flags,('__path','id'),('symbol',)),
      'function_events':S('function_events',' UNION ALL '.join(events),('__path','fn','category')),
      'call_targets':S('call_targets',"SELECT f.__path,f.id fn,'call:'||event.key id,json_extract(event.value,'$[0]') __start,json_extract(event.value,'$[1]') target,printf('%020d',CAST(event.key AS INTEGER)) visit_key FROM function_flags f,json_each(f.data,'$.local_callee_calls') event",('__path','fn')),
      'architecture_edges':S('architecture_edges',"SELECT c.__path,c.fn,c.__start,c.id,c.visit_key,target.__path target_path,target.id target FROM call_targets c JOIN function_flags owner ON owner.__path=c.__path AND owner.id=c.fn JOIN function_flags target ON target.symbol=c.target WHERE target.symbol!=owner.symbol AND NOT EXISTS(SELECT 1 FROM function_flags later WHERE later.symbol=target.symbol AND later.ordinal>target.ordinal)",('__path','fn')),
      'delegate_targets':S('delegate_targets',"SELECT f.__path,f.id fn,'delegate:'||event.key id,json_extract(event.value,'$[0]') __start,json_extract(event.value,'$[0]') __end,json_extract(event.value,'$[1]') target,CAST(event.key AS INTEGER) keyword_start FROM function_flags f,json_each(f.data,'$.deferred_lazy_delegate_calls') event",('__path','fn')),
      'side_queue_fields':S('side_queue_fields',"SELECT '' __path,'' id,0 __start,0 __end,'' scope,'' qualname,'' name,0 line WHERE 0"),
      'calls':S('calls',"SELECT '' __path,'' id,0 __start,0 __end,'' callee,'' callee_kind WHERE 0"),
      'imports':S('imports',"SELECT '' __path,'' id,0 __start,0 __end,'' module,'' name,'' local WHERE 0"),
    }
    typed=[S('supplied_rows',"SELECT CAST(file.key AS INTEGER) file_ordinal,json_extract(file.value,'$.path') __path,CAST(item.key AS INTEGER) ordinal,item.value data FROM parameters p,json_each(p.data,'$.files') file,json_each(file.value,'$.functions') item"),
      S('supplied_functions',"SELECT r.*,printf('%010d:%010d',file_ordinal,ordinal) id,json_extract(data,'$.line') __start,json_extract(data,'$.line') __end,json_extract(data,'$.line')-1 line,json_extract(data,'$.name') name,json_extract(data,'$.qualname') qualname,json_extract(data,'$.symbol') symbol,printf('%020d/%020d',file_ordinal,ordinal) fact_order,json_extract(data,'$.is_lazy') is_lazy,json_extract(data,'$.is_public') is_public,json_extract(data,'$.has_pure_lazy_decorator') pure,json_extract(data,'$.has_legacy_boundary_decorator') legacy,CASE WHEN json_extract(data,'$.is_async') THEN 'async ' ELSE '' END text FROM supplied_rows r")]
    catalog={stage['name']:overrides.get(stage['name'],stage) for stage in stages}
    catalog.update(overrides)
    catalog.update({stage['name']:stage for stage in typed})
    wanted=set();pending=[query]
    while pending:
        sql=pending.pop()
        for table in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',sql):
            if table in catalog and table not in wanted:
                wanted.add(table);pending.append(json.dumps(catalog[table].get('query',catalog[table].get('reachability'))))
    ordered=[];seen=set()
    def add(name):
        if name in seen:return
        seen.add(name)
        for table in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',json.dumps(catalog[name].get('query',catalog[name].get('reachability')))):
            if table in wanted:add(table)
        ordered.append(catalog[name])
    for name in catalog:
        if name in wanted:add(name)
    return ordered,"SELECT 'metadata' path,0 start_byte,0 end_byte,d.path||':'||d.identity identity,d.producer_phase,d.fact_order,d.diagnostic_order,json_object('file',d.path,'line',d.start_byte,'rule',d.issue,'symbol',d.symbol,'message',d.message) payload FROM ("+query+") d"


def candidate(source):
    stages,query=program(source)
    return {'format':'rerail.policy/v1','id':'data-layer-lazy-analyzed-facts','revision':1,'mode':'log','clauses':[
      {'id':'parameters','define':{'options':{'parameters':{'type':'string','default':'{}'}}}},
      {'id':'architecture','relational':{'inputs':{'empty':{'evidence':{'as':'file','query':{'ast':{'language':'python','rule':{'kind':'module'}}},'project':{'path':'file.path'}}}},
        'locations':['metadata'],'stages':[S('parameters',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}),*stages],
        'query':query,'key':['identity']},'violation':{'code':'ROW','message':'Function-fact diagnostic','anchor':'witness.span'}}]}
