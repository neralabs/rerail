# SPDX-License-Identifier: Apache-2.0
"""Typed lazy-architecture facts and diagnostic payloads shared by consumers."""
from data_layer_six.common import S,q


def apply(stages, final, vals):
    get=lambda name:next(stage for stage in stages if stage['name']==name)
    events=get('function_events')
    events['query']="""SELECT e.__path,e.fn,e.id,e.__start,e.__end,e.category,e.uses_conn,
      CASE e.category
      WHEN 'forbidden_pure' THEN 'forbidden @pure_lazy operation '||e.detail
      WHEN 'bootstrap' THEN CASE WHEN a.id IS NOT NULL THEN CASE WHEN a.target_kind='attribute' THEN 'attribute' ELSE 'subscript' END||' assignment to '||a.target_text ELSE 'mutating call '||coalesce(c.callee,e.detail)||'(...)' END
      WHEN 'payload' THEN 'callable-tainted payload passed into a ServerPlan service'
      WHEN 'dynamic' THEN CASE WHEN c.resolved IN ('callable','builtins.callable') THEN 'callable() applied to dynamically looked-up behavior' ELSE 'invokes dynamically looked-up behavior inside a ServerPlan service' END
      WHEN 'absence' THEN CASE WHEN a.id IS NOT NULL THEN 'schema fact absence fallback assigns '||a.expr_text||' to '||a.target_text WHEN c.id IS NOT NULL THEN 'schema fact absence fallback calls _record_table_exists_result(..., False)' ELSE 'schema fact absence fallback returns LazyResult.of(False)' END
      WHEN 'internal' THEN (SELECT s.value FROM arguments a JOIN strings s ON s.__path=a.__path AND s.__start=a.__start WHERE a.__path=c.__path AND a.call=c.id AND a.position=1 LIMIT 1)
      WHEN 'state' THEN (SELECT s.value FROM arguments a JOIN strings s ON s.__path=a.__path AND s.__start=a.__start WHERE a.__path=c.__path AND a.call=c.id AND a.position=1 LIMIT 1)
      WHEN 'private' THEN CASE WHEN c.resolved IN ('getattr','builtins.getattr') THEN 'getattr(..., '||quote('_conn')||')' WHEN c.resolved IN ('vars','builtins.vars') THEN 'vars(...)' ELSE e.detail END
      ELSE e.detail END detail,
      CASE WHEN e.category='reconciliation' THEN CASE WHEN e.detail IN """+vals('RECONCILIATION_SCHEMA_DISCOVERY_NAMES')+" THEN 'schema discovery' WHEN e.detail IN "+vals('RECONCILIATION_DIRECT_DDL_NAMES')+" THEN 'direct DDL or schema plan flush' ELSE 'lazy callback trampoline' END END event_kind\n      FROM ("+events['query']+") e LEFT JOIN assignments a ON a.__path=e.__path AND a.id=e.id LEFT JOIN classified_calls c ON c.__path=e.__path AND c.id=e.id JOIN function_flags flags ON flags.__path=e.__path AND flags.id=e.fn WHERE e.category NOT IN ('bootstrap','forbidden_pure') OR flags.pure"
    # Keep the reason and selected function alongside the reachability result.
    intrinsic=get('intrinsic')
    intrinsic['query']="""SELECT __path,id,location,reason FROM (
      SELECT f.__path,f.id,e.__start location,
        CASE e.category WHEN 'context' THEN 'ExecutionContext terminal ' WHEN 'internal' THEN 'reads LazyResult internals ' WHEN 'state' THEN 'writes execution-derived planner state ' WHEN 'private' THEN 'reaches through private runtime state via ' WHEN 'plan' THEN 'direct eager sink ' ELSE 'imperative native call ' END||e.detail reason,
        row_number() OVER(PARTITION BY f.__path,f.id ORDER BY CASE e.category WHEN 'context' THEN 0 WHEN 'internal' THEN 1 WHEN 'state' THEN 2 WHEN 'private' THEN 3 WHEN 'plan' THEN 4 ELSE 5 END,e.__start,e.__end DESC) chosen
      FROM function_flags f JOIN function_events e ON e.__path=f.__path AND e.fn=f.id WHERE e.category IN ('plan','native') OR f.sensitive AND e.category IN ('context','state','private') OR f.sensitive AND e.category='internal' AND f.name NOT IN """+vals('SANCTIONED_LAZY_INTERNAL_READ_HELPERS')+"""
      ) WHERE chosen=1 UNION ALL SELECT f.__path,f.id,f.__start,CASE WHEN f.legacy THEN 'legacy eager wrapper' WHEN rx_matches(f.text,'^async\\b') AND NOT f.pure THEN 'async function' ELSE 'name ends with _async' END FROM function_flags f
      WHERE (f.legacy OR rx_matches(f.text,'^async\\b') AND NOT f.pure OR f.name GLOB '*_async') AND NOT EXISTS(SELECT 1 FROM function_events e WHERE e.__path=f.__path AND e.fn=f.id AND (e.category IN ('plan','native') OR f.sensitive AND e.category IN ('context','state','private') OR f.sensitive AND e.category='internal' AND f.name NOT IN """+vals('SANCTIONED_LAZY_INTERNAL_READ_HELPERS')+"))"
    # The displayed target follows source call order, including ties beneath
    # one first hop. Limit ordered walks to roots with an actual diagnostic
    # demand. Stop at matching sinks and guard cycles with numeric node IDs.
    index=stages.index(get('transitive_matches'))
    stages[index:index]=[
      S('graph_nodes',"SELECT f.__path,f.id,f.symbol,row_number() OVER(ORDER BY f.__path,f.__start,f.id) token FROM function_flags f",('__path','id')),
      S('ordered_demands',"SELECT f.__path,f.id,'eager' mode FROM function_flags f WHERE f.pure AND EXISTS(SELECT 1 FROM reachable r JOIN intrinsic i ON i.__path=r.path AND i.id=r.node WHERE r.__path=f.__path AND r.root=f.id) UNION ALL SELECT f.__path,f.id,'nonlazy' FROM function_flags f WHERE (f.pure OR f.name GLOB '*_lazy') AND EXISTS(SELECT 1 FROM reachable r JOIN function_flags target ON target.__path=r.path AND target.id=r.node WHERE r.__path=f.__path AND r.root=f.id AND r.first IS NOT NULL AND NOT target.pure AND target.name NOT GLOB '*_lazy' AND target.symbol NOT IN "+vals('SANCTIONED_PURE_HELPER_SYMBOLS')+")",('__path','id','mode')),
      S('ordered_reach',"""WITH RECURSIVE walk(__path,root,mode,path,node,first,order_,seen) AS (
        SELECT d.__path,d.id,d.mode,d.__path,d.id,NULL,'',','||n.token||',' FROM ordered_demands d JOIN graph_nodes n ON n.__path=d.__path AND n.id=d.id
        UNION ALL SELECT r.__path,r.root,r.mode,e.target_path,e.target,coalesce(r.first,e.__start),r.order_||printf('%020d.',e.__start),r.seen||next.token||','
        FROM walk r JOIN function_flags current ON current.__path=r.path AND current.id=r.node
        LEFT JOIN intrinsic sink ON sink.__path=r.path AND sink.id=r.node
        JOIN architecture_edges e ON e.__path=r.path AND e.fn=r.node JOIN graph_nodes next ON next.__path=e.target_path AND next.id=e.target
        WHERE r.path=r.__path AND instr(r.seen,','||next.token||',')=0
          AND (r.mode='eager' AND sink.id IS NULL OR r.mode='nonlazy' AND (r.first IS NULL OR current.pure OR current.name GLOB '*_lazy'))
          AND (r.mode!='nonlazy' OR next.symbol NOT IN """+vals('SANCTIONED_PURE_HELPER_SYMBOLS')+")) SELECT * FROM walk",('__path','root','mode')),
    ]
    get('transitive_matches')['query']="""SELECT __path,root,eager,target,reason FROM (SELECT r.__path,r.root,CASE WHEN r.first IS NULL THEN -1000000000+i.location ELSE r.first END eager,f.symbol target,i.reason,
      row_number() OVER(PARTITION BY r.__path,r.root ORDER BY r.order_) chosen FROM ordered_reach r JOIN intrinsic i ON i.__path=r.path AND i.id=r.node JOIN function_flags f ON f.__path=r.path AND f.id=r.node WHERE r.mode='eager') WHERE chosen=1"""
    get('non_lazy_matches')['query']="""SELECT __path,root,location,target FROM (SELECT r.__path,r.root,r.first location,f.symbol target,row_number() OVER(PARTITION BY r.__path,r.root ORDER BY r.order_) chosen FROM ordered_reach r JOIN function_flags f ON f.__path=r.path AND f.id=r.node WHERE r.mode='nonlazy' AND r.first IS NOT NULL AND NOT f.pure AND f.name NOT GLOB '*_lazy') WHERE chosen=1"""
    # This projection is the actual diagnostic, not a Python-side message map.
    templates={
      'LA-001':'is lazy but performs eager plan/grammar I/O',
      'LA-002':'mixes intent/lazy construction with direct execution fallback',
      'LA-003':'is a public feature method performing direct plan/grammar I/O',
      'LA-004':'mixes plan I/O with imperative native connection side effects',
      'LA-005':'contains backend-native Dolt logic outside backend-local implementation',
      'LA-007':'performs prepare-time imperative I/O',
      'LA-010':"mixes 'lazy' with 'sync' or 'async' in its name",
      'LA-011':'performs server-plan pure-phase imperative I/O',
      'LA-012':'terminates a LazyResult outside a legacy boundary',
      'LA-015':'returns LazyResult.of(...) in bootstrap/DDL code; declare a ServerPlan ref, lazy primitive, or explicit terminal dependency instead',
    }
    message="CASE WHEN d.issue='LA-011' AND d.origin='hydration' THEN f.qualname||' uses forbidden terminal hydration/runtime-SQL helper' "+' '.join('WHEN d.issue='+q(code)+' THEN f.qualname||'+q(' '+text) for code,text in templates.items())+"""
      WHEN d.issue='LA-008' THEN f.qualname||'/'||f.qualname||'_async use different sync vs async execution machinery'
      WHEN d.issue='LA-006' THEN f.qualname||' passes _conn to '||d.detail||'() instead of the plan owner'
      WHEN d.origin='defer' THEN 'defer_lazy_result is forbidden; publish a real ServerPlan step/ref instead'
      WHEN d.origin='compat_import' THEN d.detail||' is forbidden in ServerPlan-sensitive files; use explicit steps/refs instead'
      WHEN d.origin='compat' THEN d.symbol||' is a forbidden lazy compatibility surface in ServerPlan-sensitive files'
      WHEN d.issue='LA-009' THEN f.qualname||CASE d.origin
        WHEN 'delegate' THEN ' delegates lazy execution to eager method '||(SELECT target.qualname FROM delegate_targets delegate JOIN function_flags target ON target.symbol=delegate.target WHERE delegate.__path=d.path AND delegate.fn=d.fn AND delegate.__start=d.start_byte AND (EXISTS(SELECT 1 FROM function_events event WHERE event.__path=target.__path AND event.fn=target.id AND event.category='plan') OR EXISTS(SELECT 1 FROM call_targets call WHERE call.__path=target.__path AND call.fn=target.id AND rx_extract(call.target,'[^.]+$',0) IN """+vals('PLAN_IO_NAMES')+""")) ORDER BY delegate.keyword_start LIMIT 1)||' (direct eager sink)'
        WHEN 'context' THEN ' hides eager execution behind an ExecutionContext terminal'
        WHEN 'internal' THEN ' peeks into LazyResult internals outside sanctioned runtime helpers'
        WHEN 'state' THEN ' writes execution-derived planner state instead of surfaced refs'
        WHEN 'private' THEN ' reaches through private runtime state instead of surfaced dependencies'
        ELSE ' reaches eager downstream function '||taint.target||' ('||taint.reason||')' END
      WHEN d.issue='LA-013' THEN f.qualname||CASE d.origin WHEN 'legacy' THEN ' reaches a legacy eager wrapper downstream' WHEN 'nonlazy' THEN ' reaches non-lazy helper '||nonlazy.target||'; every function in a lazy callchain must itself be *_lazy or @pure_lazy' ELSE ' marks runtime behavior as @pure_lazy: '||d.detail||'; @pure_lazy helpers must be pure planning code' END
      WHEN d.issue='LA-014' THEN f.qualname||CASE WHEN d.origin='payload' THEN ' passes ' ELSE ' performs ' END||d.detail
      WHEN d.issue='LA-016' THEN coalesce(f.qualname,substr(d.symbol,length(replace(substr(d.path,1,length(d.path)-3),'/','.'))+2))||' mutates state inside bootstrap/DDL @pure_lazy code ('||CASE WHEN d.origin='sidequeue' THEN 'forbidden side-queue declaration '||d.detail ELSE d.detail END||'); publish a ServerPlan step/ref and make mutation occur during plan execution'
      WHEN d.issue='LA-017' THEN f.qualname||CASE d.origin
        WHEN 'reconciliation' THEN ' uses reconciliation '||(SELECT event_kind FROM function_events e WHERE e.__path=d.path AND e.fn=d.fn AND e.__start=d.start_byte AND e.category=d.origin LIMIT 1)||' surface '||d.detail||'(); gather schema facts through a closed observation ServerPlan step and build reconciliation DDL from SchemaStateSnapshot'
        WHEN 'absence' THEN ' uses '||d.detail||'; table or schema absence must come from observed initial facts or computed eventual state'
        WHEN 'plan' THEN ' terminates a LazyResult inside reconciliation/schema code; only the wrapper may resolve observation or execution handles'
        ELSE ' restores deleted schema reconciliation cutover surface; remove the definition entirely, including fail-fast tombstones, and route callers through observation/planning facts' END
      END"""
    return "SELECT d.path,d.start_byte,d.end_byte,d.issue,d.identity,coalesce(d.symbol,f.symbol) symbol,"+message+" message FROM ("+final+") d LEFT JOIN function_flags f ON f.__path=d.path AND f.id=d.fn LEFT JOIN transitive_matches taint ON taint.__path=d.path AND taint.root=d.fn LEFT JOIN non_lazy_matches nonlazy ON nonlazy.__path=d.path AND nonlazy.root=d.fn"
