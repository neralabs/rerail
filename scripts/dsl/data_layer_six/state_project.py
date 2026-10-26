# SPDX-License-Identifier: Apache-2.0
"""Cross-file runtime builder projection contracts for scan_project_sources().

The contracts join builder-name tokens across the repository: a materializer that compares
`spec.builder_name` with a token and has a sequence projection makes that token semantic; a
planning scope that reaches a builder site (or a call to a helper holding one) naming a semantic
token, without a semantic projection of its own, is SPT-028. Token positions, callee names and
the call's own argument shape are local AST conditions on narrow candidate rows; the relational
part reuses the flow clause's contexts, direct sinks and local call graph. There is no CST copy.
"""
from .common import *

BUILDER_CALLEES=['RuntimeBuilderSpec','runtime_builder_post_op']

def project_inputs(entries,inside,positional):
 """Project-mode-only candidate rows: builder-name tokens at the positions the contracts read."""
 on={'$option':'entryPoint','cases':{'scan_project_sources':TRUE},'default':FALSE}
 # The builder call: callee name (identifier or attribute leaf), a builder_name keyword, and a
 # second positional argument (the old `arguments.position=1`).
 site=A(K('call'),P('S'),F('function',O(A(names(BUILDER_CALLEES),P('SN')),A(K('attribute'),F('attribute',A(names(BUILDER_CALLEES),P('SN')))))),
  optional(F('arguments',H(A(K('keyword_argument'),F('name',names(['builder_name'])),P('SKW'))))),
  optional(F('arguments',H(A({'nthChild':{'position':2,'ofRule':positional}},positional,P('STWO'))))))
 in_args=I(A(K('argument_list'),I(site,'neighbor','arguments')),'neighbor')
 # A token is a direct operand of a comparison that also has a `spec.builder_name` operand, the
 # second positional argument of a builder call, or the value of its builder_name keyword.
 role=O(I(A(K('comparison_operator'),P('CMP'),H(A(K('attribute'),F('object',names(['spec'])),F('attribute',names(['builder_name']))))),'neighbor'),
  A({'nthChild':{'position':2,'ofRule':positional}},positional,in_args),
  I(A(K('keyword_argument'),P('KWP'),F('name',names(['builder_name'])),in_args),'neighbor','value'))
 token={'cmp':'capture.CMP.startByte','s_start':'capture.S.startByte','s_end':'capture.S.endByte','sn':'capture.SN','skw':'capture.SKW.startByte','stwo':'capture.STWO.startByte','kwp':'capture.KWP.startByte',
  'top_start':'capture.TOP.startByte','top_end':'capture.TOP.endByte'}
 strings=O(K('string'),K('concatenated_string'))
 # Module-level string constants (constant folding of identifier tokens).
 constant=I(A(K('assignment'),F('left',A(K('identifier'),P('MT'))),I(A(K('expression_statement'),I(K('module'),'neighbor')),'neighbor')),'neighbor','right')
 return {
  'builder_strs':E(A(strings,on,P('STRING'),N(H(K('interpolation'),stop='end')),O(A(inside,role),constant),gate(entries)),
   {'value':'capture.DECODED.text','mt':'capture.MT',**token},transform={'DECODED':{'replace':{'source':'$STRING','value':True,'replace':r'\b\B','by':''}}}),
  'builder_refs':E(A(O(K('identifier'),K('attribute')),on,O(A(K('identifier'),P('ID')),A(K('attribute'),F('object',names(['RuntimeBuilderName'])),F('attribute',P('AT')))),inside,role,gate(entries)),
   {'text':'capture.ID','attr':'capture.AT',**token}),
 }

def project_stages(own):
 cols="cmp,s_start,s_end,sn,skw,stwo,kwp"
 return [
 S('builder_tokens',"SELECT __path,"+own('t')+" AS fn,value AS name,"+cols+" FROM builder_strs t WHERE t.top_start IS NOT NULL AND t.value!='' "
  "UNION ALL SELECT __path,"+own('r')+",attr,"+cols+" FROM builder_refs r WHERE r.attr IS NOT NULL "
  "UNION ALL SELECT r.__path,"+own('r')+",k.value,"+','.join('r.'+c for c in cols.split(','))+" FROM builder_refs r JOIN builder_strs k ON k.__path=r.__path AND k.mt=r.text WHERE r.text IS NOT NULL AND k.value!=''",('__path','fn')),
 S('semantic_builders',"SELECT DISTINCT t.name FROM builder_tokens t WHERE t.cmp IS NOT NULL AND EXISTS(SELECT 1 FROM direct_sinks s WHERE s.__path=t.__path AND s.fn=t.fn AND s.kind='runtime_sequence_projection')",('name',)),
 S('builder_sites',"SELECT DISTINCT __path,fn,(s_start*16777216+s_end) AS node,s_start AS __start,s_end AS __end,name FROM builder_tokens WHERE s_start IS NOT NULL AND (kwp IS NULL AND (sn='runtime_builder_post_op' OR sn='RuntimeBuilderSpec' AND skw IS NULL) OR kwp IS NOT NULL AND (sn='RuntimeBuilderSpec' OR sn='runtime_builder_post_op' AND stwo IS NULL))",('__path','fn'),('name',)),
 S('helper_names',"SELECT DISTINCT c.name FROM contexts c JOIN builder_sites s ON s.__path=c.__path AND s.fn=c.id WHERE s.name IN (SELECT name FROM semantic_builders)",('name',)),
 S('project_seeds',"SELECT __path,fn,node,__start,__end,'builder' AS kind,0 AS priority FROM builder_sites WHERE name IN (SELECT name FROM semantic_builders) "
  "UNION SELECT c.__path,c.fn,c.id,c.__start,c.__end,'builder',1 FROM b_calls c WHERE EXISTS(SELECT 1 FROM helper_names) AND c.callee IN (SELECT name FROM helper_names) AND NOT EXISTS(SELECT 1 FROM call_graph g WHERE g.__path=c.__path AND g.fn=c.fn AND g.node=c.id) "
  "UNION SELECT __path,fn,node,__start,__end,'semantic',0 FROM direct_sinks WHERE EXISTS(SELECT 1 FROM project_mode) AND kind='runtime_semantic_projection'",('__path','fn','kind')),
 S('project_sinks',"WITH RECURSIVE sink(__path,fn,node,__start,__end,kind,priority,route) AS (SELECT __path,fn,node,__start,__end,kind,priority,'|'||fn||'|' FROM project_seeds "
  "UNION ALL SELECT g.__path,g.fn,s.node,s.__start,s.__end,s.kind,s.priority,s.route||g.fn||'|' FROM sink s JOIN call_graph g ON g.__path=s.__path AND g.target=s.fn WHERE instr(s.route,'|'||g.fn||'|')=0 AND NOT EXISTS(SELECT 1 FROM project_seeds direct WHERE direct.__path=g.__path AND direct.fn=g.fn AND direct.kind=s.kind)) "
  "SELECT __path,fn,node,__start,__end,kind FROM (SELECT s.*,row_number() OVER(PARTITION BY s.__path,s.fn,s.kind ORDER BY length(s.route),s.priority,s.__start) AS chosen FROM sink s) WHERE chosen=1",('__path','fn','kind')),
 ]

PROJECT_QUERY=("SELECT s.__path AS path,s.__start AS start_byte,s.__end AS end_byte,'SPT-028' AS issue,c.id AS identity,'project-builder' AS detector FROM contexts c "
 "JOIN project_sinks s ON s.__path=c.__path AND s.fn=c.id AND s.kind='builder' WHERE c.planning AND NOT EXISTS(SELECT 1 FROM project_sinks semantic WHERE semantic.__path=c.__path AND semantic.fn=c.id AND semantic.kind='semantic')")
