# SPDX-License-Identifier: Apache-2.0
"""Runtime-reference return summaries from expression and binding edges."""
from .common import *

ENTRIES=('check_runtime_ref_closed_post_op_consumption_guardrail','check_helper_anchored_runtime_ref_post_op_guardrail','check_runtime_refs_post_op_requires_runtime_refs_guardrail')
def return_clause():
 funcs=O(K('function_definition'),K('lambda'))
 fields=lambda capture,prefix:{prefix+'_start':f'capture.{capture}.startByte',prefix+'_end':f'capture.{capture}.endByte',prefix+'_kind':f'capture.{capture}.kind',prefix+'_text':f'capture.{capture}'}
 expr=O(*(K(k) for k in ['identifier','call','tuple','list','set','dictionary','expression_list','parenthesized_expression']))
 empty=O(K('none'),A(O(A(K('string'),RX(r'''(?i)^[ru]*["']''')),A(K('concatenated_string'),N(H(K('interpolation'),stop='end')))),{'value':{'eq':''}}),A(O(K('tuple'),K('list'),K('set')),N(H(N(K('comment'))))),A(K('parenthesized_expression'),H({'matches':'empty-refs'})))
 declared=F('arguments',H(A(K('keyword_argument'),F('name',names(['runtime_ref_ids','source_refs'])),F('value',N({'matches':'empty-refs'})))))
 callrule=A(K('call'),P('CALL'),F('function',P('FUNC')),optional(F('function',O(A(K('identifier'),P('TERM')),A(K('attribute'),F('attribute',P('TERM')))))),optional(A(declared,P('DECLARED'))),optional(A(F('arguments',H(A(K('keyword_argument'),F('name',names(['runtime_ref_ids'])),F('value',N({'matches':'empty-refs'}))))),P('RUNTIME_REFS'))))
 arglist=A(K('argument_list'),I(A(K('call'),P('DST')),'neighbor'))
 inputs={
 'functions':E(A(K('function_definition'),F('name',P('NAME'))),{'name':'capture.NAME'}),
 'calls':E(callrule,{'callee':'capture.TERM','full_callee':'capture.FUNC','refs':'capture.DECLARED.startByte','runtime_refs':'capture.RUNTIME_REFS.startByte'}),
 'positional_edges':E(A(expr,P('SRC'),I(arglist,'neighbor')),fields('SRC','src')|fields('DST','dst')),
 'keyword_edges':E(A(K('keyword_argument'),F('value',A(expr,P('SRC'))),I(arglist,'neighbor')),fields('SRC','src')|fields('DST','dst')),
 'container_edges':E(A(expr,P('SRC'),I(A(O(K('tuple'),K('list'),K('set'),K('expression_list'),K('parenthesized_expression')),P('DST')),'neighbor')),fields('SRC','src')|fields('DST','dst')),
 'dictionary_edges':E(A(K('pair'),F('value',A(expr,P('SRC'))),I(A(K('dictionary'),P('DST')),'neighbor')),fields('SRC','src')|fields('DST','dst')),
 'spread_edges':E(A(K('dictionary_splat'),H(A(expr,P('SRC'))),O(I(arglist,'neighbor'),I(A(K('dictionary'),P('DST')),'neighbor'))),fields('SRC','src')|fields('DST','dst')),
 'returns':E(A(K('return_statement'),H(A(P('VALUE'),N(K('comment')))),I(A(K('function_definition'),P('FN')),funcs)),fields('VALUE','value')|{'fn':'capture.FN.startByte'}),
 'bindings':E(A(K('assignment'),F('left',A(K('identifier'),P('NAME'))),F('right',P('VALUE'))),fields('VALUE','value')|{'name':'capture.NAME'}),
 'postops':E(A(K('keyword_argument'),P('KW'),F('name',names(['closed_post_op'])),F('value',P('VALUE')),I(A(K('argument_list'),I(A(K('call'),P('CALL')),'neighbor')),'neighbor')),fields('VALUE','value')|{'call':'capture.CALL.startByte','keyword':'capture.KW.startByte'},anchor='capture.VALUE'),
 }
 for input_ in inputs.values():
  ast=input_['evidence']['query']['ast'];ast['utils']={'empty-refs':empty};ast['rule']=A(gate([*ENTRIES,'_scan_source','_scan_file']),ast['rule'])
 stages=[
 S('edges', ' UNION '.join("SELECT __path,src_kind||':'||src_start||':'||src_end AS src,src_start,src_end,src_kind,src_text,dst_kind||':'||dst_start||':'||dst_end AS dst,dst_start,dst_end,dst_kind,dst_text FROM "+table for table in ['positional_edges','keyword_edges','container_edges','dictionary_edges','spread_edges']),('__path','src'),('__path','dst')),
 S('nodes',"SELECT __path,id,start,end,kind,max(text) AS text FROM (SELECT DISTINCT __path,src AS id,src_start AS start,src_end AS end,src_kind AS kind,src_text AS text FROM edges UNION SELECT DISTINCT __path,dst,dst_start,dst_end,dst_kind,dst_text FROM edges UNION SELECT __path,'call:'||__start||':'||__end,__start,__end,'call','' FROM calls UNION SELECT __path,value_kind||':'||value_start||':'||value_end,value_start,value_end,value_kind,value_text FROM returns UNION SELECT __path,value_kind||':'||value_start||':'||value_end,value_start,value_end,value_kind,value_text FROM bindings) GROUP BY __path,id,start,end,kind",('__path','id'),('__path','start')),
 S('contexts',"SELECT n.__path,n.id,f.__start AS fn FROM nodes n JOIN functions f ON f.__path=n.__path AND f.__start<=n.start AND f.__end>=n.end",('__path','fn','id'),('__path','id','fn')),
 S('locals',"SELECT __path,fn,name,value FROM (SELECT f.__path,f.__start AS fn,b.name,b.value_kind||':'||b.value_start||':'||b.value_end AS value,row_number() OVER(PARTITION BY f.__path,f.__start,b.name ORDER BY b.__start DESC) AS selected FROM functions f JOIN bindings b ON b.__path=f.__path AND b.__start>f.__start AND b.__end<=f.__end) WHERE selected=1",('__path','fn','name')),
 S('flows',"SELECT e.__path,ctx.fn,e.src,e.dst FROM edges e JOIN contexts ctx ON ctx.__path=e.__path AND ctx.id=e.dst UNION SELECT n.__path,c.fn,b.value,n.id FROM nodes n JOIN contexts c ON c.__path=n.__path AND c.id=n.id JOIN locals b ON b.__path=c.__path AND b.fn=c.fn AND b.name=n.text WHERE n.kind='identifier'",('__path','fn','src')),
 ]
 for table,refless in [('consumers',False),('refless',True)]:
  seed="c.callee='runtime_refs_post_op' AND c.refs IS NULL" if refless else 'c.refs IS NOT NULL'
  stop="WHERE NOT EXISTS(SELECT 1 FROM calls c WHERE c.__path=e.__path AND c.__start||':'||c.__end=substr(e.dst,6) AND c.callee='runtime_refs_post_op')" if refless else ''
  name_fallback="""UNION SELECT r.__path,ctx.fn,n.id FROM reaching r JOIN returns ret ON ret.__path=r.__path AND ret.fn=r.fn AND ret.value_kind||':'||ret.value_start||':'||ret.value_end=r.id JOIN functions f ON f.__path=ret.__path AND f.__start=ret.fn JOIN nodes n ON n.__path=f.__path AND n.kind='identifier' AND n.text=f.name JOIN contexts ctx ON ctx.__path=n.__path AND ctx.id=n.id WHERE NOT EXISTS(SELECT 1 FROM locals local WHERE local.__path=ctx.__path AND local.fn=ctx.fn AND local.name=n.text)""" if refless else ''
  stages.append(S(table,"""WITH RECURSIVE reaching(__path,fn,id) AS (
 SELECT c.__path,ctx.fn,'call:'||c.__start||':'||c.__end FROM calls c JOIN contexts ctx ON ctx.__path=c.__path AND ctx.id='call:'||c.__start||':'||c.__end WHERE """+seed+"""
 UNION SELECT r.__path,r.fn,e.dst FROM reaching r JOIN flows e ON e.__path=r.__path AND e.fn=r.fn AND e.src=r.id """+stop+"""
 UNION SELECT r.__path,ctx.fn,'call:'||c.__start||':'||c.__end FROM reaching r JOIN returns ret ON ret.__path=r.__path AND ret.fn=r.fn AND ret.value_kind||':'||ret.value_start||':'||ret.value_end=r.id JOIN functions f ON f.__path=ret.__path AND f.__start=ret.fn JOIN calls c ON c.__path=f.__path AND c.full_callee=f.name JOIN contexts ctx ON ctx.__path=c.__path AND ctx.id='call:'||c.__start||':'||c.__end
 """+name_fallback+") SELECT * FROM reaching",('__path','fn','id')))
 ref=lambda yes,code,entry:"""SELECT p.__path AS path,p.__start AS start_byte,p.__end AS end_byte,"""+q(code)+" AS issue,"+q(entry)+" AS detector,ctx.fn||':'||p.keyword AS identity FROM postops p JOIN calls call ON call.__path=p.__path AND call.__start=p.call JOIN contexts ctx ON ctx.__path=p.__path AND ctx.id=p.value_kind||':'||p.value_start||':'||p.value_end JOIN consumers c ON c.__path=ctx.__path AND c.fn=ctx.fn AND c.id=ctx.id WHERE "+('call.runtime_refs IS NOT NULL' if yes else 'call.runtime_refs IS NULL')
 a=ref(True,'STEP-033',ENTRIES[0]);b=ref(False,'STEP-035',ENTRIES[1])
 c="""SELECT f.__path AS path,f.__start AS start_byte,f.__end AS end_byte,'STEP-034' AS issue,"""+q(ENTRIES[2])+""" AS detector,f.__start AS identity FROM functions f WHERE EXISTS(SELECT 1 FROM functions alias JOIN returns r ON r.__path=alias.__path AND r.fn=alias.__start JOIN refless v ON v.__path=r.__path AND v.fn=r.fn AND v.id=r.value_kind||':'||r.value_start||':'||r.value_end WHERE alias.__path=f.__path AND alias.name=f.name)"""
 query=' UNION ALL '.join([a,b,c])
 return {'id':'runtime-reference-summaries','relational':{'inputs':inputs,'stages':stages,'query':{'$option':'entryPoint','cases':{**{e:query for e in ['all','_scan_source','_scan_file']},ENTRIES[0]:a,ENTRIES[1]:b,ENTRIES[2]:c},'default':"SELECT '' AS path,0 AS start_byte,0 AS end_byte,'' AS issue,'' AS detector,'' AS identity WHERE 0"},'key':['detector','identity'],'code':'issue'},'violation':{'code':'runtime_reference','message':'Runtime-reference materialization contract','anchor':'witness.span'}}
