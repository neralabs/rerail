# SPDX-License-Identifier: Apache-2.0
"""Ordered policy comparisons shared by source discovery and supplied-fact APIs."""
from data_layer_six.common import S,q

TYPE='l2070_batching.optimizer_run_cache.OptimizerRunCache'


def stages(kind):
    memo=kind=='memoized';prefix='memoized ' if memo else ''
    result=[S('effective_actual',"SELECT * FROM actual_records a WHERE NOT EXISTS(SELECT 1 FROM actual_records later WHERE later.function=a.function AND later.ordinal>a.ordinal)",('function',)),
      S('policy_args',"SELECT p.function,CAST(x.key AS INTEGER) ordinal,x.value name FROM policy_entries p,json_each(p.data,'$.cache_args') x",('function',)),
      S('policy_types',"SELECT p.function,CAST(x.key AS INTEGER) ordinal,json_extract(x.value,'$[0]') name,json_extract(x.value,'$[1]') type FROM policy_entries p,json_each(p.data,'$.cache_types') x",('function',)),
      S('policy_members',"SELECT p.function,CAST(x.key AS INTEGER) ordinal,json_extract(x.value,'$[0]') member,json_extract(x.value,'$[1]') mode FROM policy_entries p,json_each(p.data,'$.allowed_cache_members') x",('function',)),
      S('member_reprs',"SELECT f.function,coalesce((SELECT '('||group_concat(pair,', ')||CASE WHEN count(*)=1 THEN ',' ELSE '' END||')' FROM (SELECT '('||quote(member)||', '||quote(mode)||')' pair FROM actual_members m WHERE m.record_id=f.record_id ORDER BY ordinal)),'()') actual,coalesce((SELECT '('||group_concat(pair,', ')||CASE WHEN count(*)=1 THEN ',' ELSE '' END||')' FROM (SELECT '('||quote(member)||', '||quote(mode)||')' pair FROM policy_members m WHERE m.function=f.function ORDER BY ordinal)),'()') policy FROM effective_actual f",('function',))]
    if not memo:
        from data_layer_eight.python_repr import annotations
        result.append(annotations())
    rows=[]
    def add(phase,step,query,ordinal='0'):
        rows.append("SELECT "+str(phase)+" phase,"+str(step)+" step,"+ordinal+" ordinal,x.* FROM ("+query+") x")
    for rank,(table,label) in enumerate([('actual_records','decorator'),('policy_records','policy')]):
        add(0,rank,"SELECT function,"+q('duplicate '+prefix+label+' entry: ')+"||function issue FROM "+table+" GROUP BY function HAVING count(*)>1")
    if memo:
        add(1,0,"SELECT a.function,a.function||': '||error issue,a.ordinal record_ordinal,e.ordinal error_ordinal FROM actual_errors e JOIN actual_records a ON a.record_id=e.record_id",'0')
        # Project the same columns for every arm; ordering data stays typed.
        rows[-1]="SELECT 1 phase,0 step,printf('%020d/%020d',a.ordinal,e.ordinal) ordinal,a.function,a.function||': '||e.error issue FROM actual_errors e JOIN actual_records a ON a.record_id=e.record_id ORDER BY a.ordinal,e.ordinal"
    unsigned_phase=2 if memo else 1
    add(unsigned_phase,0,"SELECT a.function,"+q('unsigned '+prefix+'cached pure-lazy function: ' if not memo else 'unsigned memoized pure-lazy function: ')+"||a.function issue FROM effective_actual a WHERE NOT EXISTS(SELECT 1 FROM policy_entries p WHERE p.function=a.function)")
    add(unsigned_phase+1,0,"SELECT p.function,"+q('stale '+prefix+'cached pure-lazy policy entry: ' if not memo else 'stale memoized pure-lazy policy entry: ')+"||p.function issue FROM policy_entries p WHERE NOT EXISTS(SELECT 1 FROM effective_actual a WHERE a.function=p.function)")
    phase=unsigned_phase+2
    def compare(step,message,condition,extra='',ordinal='0'):
        add(phase,step,"SELECT a.function,a.function||': '||("+message+") issue"+(" ,"+ordinal+" order_ordinal" if ordinal!='0' else '')+" FROM effective_actual a JOIN policy_entries p ON p.function=a.function "+extra+" WHERE "+condition,
            'x.order_ordinal' if ordinal!='0' else '0')
        if ordinal!='0':rows[-1]=rows[-1].replace('x.*','x.function,x.issue')
    if memo:
        compare(0,q('memoization bound changed'),"a.max_entries IS NOT json_extract(p.data,'$.max_entries')")
        for index,(field,value,message) in enumerate([
            ('cache_lifetime','process','cache lifetime is not approved'),('key_policy','bound_explicit_immutable_v1','key policy is not approved'),
            ('return_policy','immutable_no_alias_v1','return policy is not approved'),('exception_policy','successful_results_only_v1','exception policy is not approved')],1):
            compare(index,q(message),"json_extract(p.data,"+q('$.'+field)+") IS NOT "+q(value))
        end=5
    else:
        add(phase,0,"SELECT a.function,a.function||': '||e.error issue,e.ordinal error_ordinal FROM effective_actual a JOIN policy_entries p ON p.function=a.function JOIN actual_errors e ON e.record_id=a.record_id",'x.error_ordinal')
        rows[-1]=rows[-1].replace('x.*','x.function,x.issue')
        compare(1,q('decorator cache arguments changed'),"EXISTS(SELECT ordinal,name FROM actual_args WHERE record_id=a.record_id EXCEPT SELECT ordinal,name FROM policy_args WHERE function=a.function) OR EXISTS(SELECT ordinal,name FROM policy_args WHERE function=a.function EXCEPT SELECT ordinal,name FROM actual_args WHERE record_id=a.record_id)")
        compare(2,q('cache type metadata must name each cache argument exactly once'),"EXISTS(SELECT name FROM policy_types WHERE function=a.function GROUP BY name HAVING count(*)>1) OR EXISTS(SELECT name FROM policy_args WHERE function=a.function EXCEPT SELECT name FROM policy_types WHERE function=a.function) OR EXISTS(SELECT name FROM policy_types WHERE function=a.function EXCEPT SELECT name FROM policy_args WHERE function=a.function)")
        compare(3,"'cache type '||quote(t.type)||' is not approved'","t.type!="+q(TYPE),'JOIN policy_types t ON t.function=a.function', 't.ordinal*2')
        annotation="(SELECT annotation FROM actual_annotations annotation WHERE annotation.record_id=a.record_id AND annotation.name=t.name ORDER BY annotation.ordinal DESC LIMIT 1)"
        compare(3,"'cache argument '||quote(t.name)||' annotation '||CASE WHEN "+annotation+" IS NULL THEN 'None' ELSE (SELECT repr FROM annotation_reprs repr WHERE repr.record_id=a.record_id AND repr.ordinal=(SELECT annotation.ordinal FROM actual_annotations annotation WHERE annotation.record_id=a.record_id AND annotation.name=t.name ORDER BY annotation.ordinal DESC LIMIT 1)) END||' does not match cache type '||quote(t.type)",annotation+" IS NOT rx_replace(t.type,'^.*\\.','')",'JOIN policy_types t ON t.function=a.function','t.ordinal*2+1')
        compare(4,"'owner scope '||quote(json_extract(p.data,'$.owner_scope'))||' is not approved'","json_extract(p.data,'$.owner_scope')!='optimizer_run'")
        end=5
    compare(end,q('review rationale is required'),"trim(p.rationale)=''")
    if not memo:
        compare(6,"'cache member access modes are invalid: ['||(SELECT group_concat(quote(mode),', ') FROM (SELECT DISTINCT mode FROM policy_members m WHERE m.function=a.function AND mode NOT IN ('call','read','resolver') ORDER BY mode))||']'","EXISTS(SELECT 1 FROM policy_members m WHERE m.function=a.function AND mode NOT IN ('call','read','resolver'))")
    compare(7,q('function body hash changed'),'a.body_hash!=p.body_hash')
    compare(8,q('function signature hash changed'),'a.signature_hash!=p.signature_hash')
    if not memo:compare(9,"'cache member access changed: actual='||r.actual||' policy='||r.policy",'r.actual!=r.policy','JOIN member_reprs r ON r.function=a.function')
    # Parenthesize individual ordered arms so each produces the same schema.
    result.append(S('ordered_policy_issues',' UNION ALL '.join('SELECT * FROM ('+row+')' for row in rows)))
    result.append(S('diagnostics','SELECT issue FROM ordered_policy_issues ORDER BY phase,CASE WHEN phase=1 AND '+str(int(memo))+" THEN '' ELSE function END,step,ordinal"))
    return result


def source_inputs(kind):
    record="f.__path||':'||f.id"
    if kind=='cached':
        return [
          S('actual_records',"SELECT "+record+" record_id,row_number() OVER(ORDER BY f.__path,n.__start)-1 ordinal,f.function,f.body_sha256 body_hash,f.signature_sha256 signature_hash,NULL max_entries FROM actual_functions f JOIN nodes n ON n.__path=f.__path AND n.id=f.id",('function',)),
          S('actual_args',"SELECT a.__path||':'||a.fn record_id,a.ordinal,a.name FROM cache_arguments a",('record_id',)),
          S('actual_annotations',"SELECT p.__path||':'||p.fn record_id,row_number() OVER(PARTITION BY p.__path,p.fn ORDER BY p.name)-1 ordinal,p.name,v.unparse annotation FROM parameters p JOIN cache_arguments a ON a.__path=p.__path AND a.fn=p.fn AND a.name=p.name JOIN annotation_values v ON v.__path=p.__path AND v.id=p.annotation",('record_id',)),
          S('actual_members',"SELECT m.__path||':'||m.fn record_id,row_number() OVER(PARTITION BY m.__path,m.fn ORDER BY member,mode)-1 ordinal,member,mode FROM cache_members m",('record_id',)),
          S('actual_errors',"SELECT __path||':'||fn record_id,row_number() OVER(PARTITION BY __path,fn ORDER BY error)-1 ordinal,error FROM (SELECT __path,fn,error FROM cached_errors UNION SELECT __path,fn,error FROM extra_cached_errors)",('record_id',))]
    return [
      S('actual_records',"SELECT r.*,row_number() OVER(ORDER BY function)-1 ordinal FROM (SELECT "+record+" record_id,f.function,f.body_sha256 body_hash,f.signature_sha256 signature_hash,b.max_entries FROM actual_functions f JOIN memoized_bounds b ON b.__path=f.__path AND b.id=f.id UNION ALL SELECT a.__path||':'||a.id,a.function,h.dump_sha256,h.dump_sha256,NULL FROM memoized_aliases a JOIN alias_fingerprints h ON h.__path=a.__path AND h.id=a.id) r",('function',)),
      S('actual_errors',"SELECT record_id,row_number() OVER(PARTITION BY record_id ORDER BY error)-1 ordinal,error FROM (SELECT f.__path||':'||f.id record_id,e.error FROM actual_functions f JOIN memoized_errors e ON e.__path=f.__path AND e.fn=f.id UNION SELECT a.__path||':'||a.id,a.error FROM memoized_aliases a)",('record_id',))]


def apply(source_stages,kind):
    source_stages.extend(source_inputs(kind));source_stages.extend(stages(kind))


def supplied_candidate(kind):
    # Supplied DecoratedFunction records are the API input; source is not read.
    inputs=[
      S('actual_records',"SELECT CAST(key AS INTEGER) ordinal,CAST(key AS TEXT) record_id,json_extract(value,'$.function') function,json_extract(value,'$.body_hash') body_hash,json_extract(value,'$.signature_hash') signature_hash,json_extract(value,'$.max_entries') max_entries,value data FROM parameters,json_each(data,'$.functions')",('function',)),
      S('policy_records',"SELECT CAST(key AS INTEGER) ordinal,json_extract(value,'$.function') function,json_extract(value,'$.body_hash') body_hash,json_extract(value,'$.signature_hash') signature_hash,json_extract(value,'$.rationale') rationale,value data FROM parameters,json_each(data,'$.entries')",('function',)),
      S('policy_entries',"SELECT * FROM policy_records p WHERE NOT EXISTS(SELECT 1 FROM policy_records later WHERE later.function=p.function AND later.ordinal>p.ordinal)",('function',)),
      S('actual_errors',"SELECT a.record_id,CAST(e.key AS INTEGER) ordinal,e.value error FROM actual_records a,json_each(a.data,'$.errors') e",('record_id',))]
    if kind=='cached':inputs.extend([
      S('actual_args',"SELECT a.record_id,CAST(v.key AS INTEGER) ordinal,v.value name FROM actual_records a,json_each(a.data,'$.cache_args') v",('record_id',)),
      S('actual_annotations',"SELECT a.record_id,CAST(v.key AS INTEGER) ordinal,json_extract(v.value,'$[0]') name,json_extract(v.value,'$[1]') annotation FROM actual_records a,json_each(a.data,'$.cache_annotations') v",('record_id',)),
      S('actual_members',"SELECT a.record_id,CAST(v.key AS INTEGER) ordinal,json_extract(v.value,'$[0]') member,json_extract(v.value,'$[1]') mode FROM actual_records a,json_each(a.data,'$.cache_members') v",('record_id',))])
    selected=stages(kind)
    # Unused cache-only input relations are not evaluated for memoization.
    if kind=='memoized':selected=[s for s in selected if s['name'] not in ('policy_args','policy_types','policy_members','member_reprs')]
    query="SELECT 'metadata' path,0 start_byte,0 end_byte,CAST(row_number() OVER() AS TEXT) identity,issue,phase,step,ordinal,function FROM ordered_policy_issues ORDER BY phase,CASE WHEN phase=1 AND "+str(int(kind=='memoized'))+" THEN '' ELSE function END,step,ordinal"
    return {'format':'rerail.policy/v1','id':'data-layer-'+kind+'-policy-comparison','revision':1,'mode':'log','clauses':[
      {'id':'parameters','define':{'options':{'parameters':{'type':'string','default':'{}'}}}},
      {'id':'comparison','relational':{'inputs':{'empty':{'evidence':{'as':'file','query':{'ast':{'language':'python','rule':{'kind':'module'}}},'project':{'path':'file.path'}}}},
       'locations':['metadata'],'stages':[S('parameters',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}),*inputs,*selected],
       'query':query,'key':['identity'],'code':'issue'},'violation':{'code':'POLICY','message':'Policy comparison','anchor':'witness.span'}}]}
