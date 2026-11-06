# SPDX-License-Identifier: Apache-2.0
"""Carry exact purity diagnostic payloads and stable scopes in SQL."""
from data_layer_six.common import literals


def apply(stages, values):
    get=lambda name:next(stage for stage in stages if stage['name']==name)
    immutable=literals(values('IMMUTABLE_GLOBAL_CALLS'))
    globals_=get('global_state')
    globals_['query']="""WITH RECURSIVE referenced_names(__path,name) AS (
      SELECT used.__path,used.text FROM nodes used
      CROSS JOIN module_bindings a ON a.__path=used.__path AND a.name=used.text
      CROSS JOIN binding_values v ON v.__path=a.__path AND v.binding=a.id
      CROSS JOIN nodes value ON value.__path=v.__path AND value.id=v.value
      WHERE used.kind='identifier' AND value.kind IN ('dictionary','list','set')
        AND EXISTS(SELECT 1 FROM active_functions f WHERE f.__path=used.__path AND used.__start>=f.body_start AND used.__end<=f.body_end)
      GROUP BY used.__path,used.text),
      immutable_walk(__path,binding,root,node) AS (
      SELECT a.__path,a.id,v.value,v.value FROM module_bindings a JOIN binding_values v ON v.__path=a.__path AND v.binding=a.id JOIN nodes n ON n.__path=v.__path AND n.id=v.value WHERE n.kind IN ('dictionary','list','set') AND (a.__path,a.name) IN (SELECT __path,name FROM referenced_names)
      UNION ALL SELECT w.__path,w.binding,w.root,child.id FROM immutable_walk w
      JOIN nodes n ON n.__path=w.__path AND n.id=w.node AND (n.id=w.root OR n.kind IN ('pair','tuple','unary_operator','binary_operator','parenthesized_expression'))
      JOIN nodes child ON child.__path=n.__path AND child.parent=n.id WHERE child.kind!='comment'
      UNION ALL SELECT w.__path,w.binding,w.root,child.id FROM immutable_walk w
      JOIN nodes n ON n.__path=w.__path AND n.id=w.node AND n.kind='call'
      JOIN calls c ON c.__path=n.__path AND c.id=n.id AND c.callee IN """+immutable+""" AND c.callee NOT IN ('tuple','frozenset')
      JOIN nodes child ON child.__path=n.__path AND child.id IN (SELECT a.id FROM arguments a WHERE a.__path=c.__path AND a.call=c.id) WHERE child.kind!='comment'),
      static_lookups AS (SELECT w.__path,w.binding FROM immutable_walk w JOIN nodes n ON n.__path=w.__path AND n.id=w.node LEFT JOIN calls c ON c.__path=n.__path AND c.id=n.id
        GROUP BY w.__path,w.binding HAVING sum(CASE WHEN n.id=w.root OR n.kind IN ('pair','tuple','unary_operator','binary_operator','parenthesized_expression','integer','float','true','false','none','ellipsis','concatenated_string') OR n.kind='string' AND NOT rx_matches(n.text,'(?i)^[rbu]*f') OR n.kind='call' AND c.callee IN """+immutable+""" THEN 0 ELSE 1 END)=0 AND count(*)>1)
      """+globals_['query'].replace('a.name,CASE',"a.name,CASE WHEN c.id IS NOT NULL THEN CASE WHEN c.managed THEN 'managed-memoization-cache' ELSE 'runtime-object' END WHEN m.name IS NOT NULL THEN 'mutated-cache-or-registry' WHEN static.binding IS NOT NULL THEN 'mutable-static-table' ELSE 'mutable-cache-or-registry' END category,CASE",1).replace(' WHERE m.name IS NOT NULL',' LEFT JOIN static_lookups static ON static.__path=a.__path AND static.binding=a.id WHERE m.name IS NOT NULL',1)
    opaque=get('opaque_calls')
    opaque['query']=opaque['query'].replace('c.__path,c.id FROM calls',"c.__path,c.id,coalesce((SELECT field.name FROM keywords k JOIN calls payload ON payload.__path=k.__path AND payload.id=k.value JOIN keywords field ON field.__path=payload.__path AND field.call=payload.id WHERE k.__path=c.__path AND k.call=c.id AND k.name='payload' AND field.name IN "+literals(values('OPAQUE_LAZY_OPERATION_PAYLOAD_FIELDS'))+" ORDER BY field.__start LIMIT 1),'handle') payload_field FROM calls",1)
    locals_=get('opaque_locals')
    locals_['query']=locals_['query'].replace('name,since)', 'name,since,payload_field)',1).replace('b.name,a.__start FROM assignments', 'b.name,a.__start,o.payload_field FROM assignments',1)
    locals_['query']=locals_['query'].replace('UNION SELECT a.__path,a.fn,b.name,a.__start FROM assignments','UNION SELECT a.__path,a.fn,b.name,a.__start,old.payload_field FROM assignments',1)
    opaque_values=get('opaque_values')
    opaque_values['query']=opaque_values['query'].replace('SELECT __path,id FROM opaque_calls','SELECT __path,id,payload_field FROM opaque_calls').replace('SELECT n.__path,n.id FROM nodes','SELECT n.__path,n.id,o.payload_field FROM nodes').replace('SELECT c.__path,c.id FROM calls','SELECT c.__path,c.id,o.payload_field FROM calls')
    returns=get('return_events')
    returns['query']=returns['query'].replace("coalesce(dynamic.detail,'return') detail","CASE WHEN opaque.id IS NOT NULL THEN opaque.payload_field ELSE coalesce(dynamic.detail,'return') END detail")
    diagnostics=get('diagnostics')
    diagnostics['query']=diagnostics['query'].replace('detail,qualname FROM visible_events','detail,qualname,fn,id node FROM visible_events').replace("'contract',f.qualname FROM", "'contract',f.qualname,f.id,f.id FROM").replace("'export',qualname FROM", "'export',qualname,id,id FROM")


def query():
    name="rx_extract(d.detail,'^[^.\\[]+',0)"
    access="CASE WHEN substr(d.detail,-2)='[]' THEN '[]' WHEN instr(d.detail,'.')>0 THEN substr(d.detail,instr(d.detail,'.')) ELSE 'read' END"
    return """WITH global_context AS (
      SELECT d.__path,d.identity,g.name,k.name sink,row_number() OVER(PARTITION BY d.__path,d.identity ORDER BY g.__start,k.keyword_start) chosen
      FROM diagnostics d JOIN global_uses g ON g.__path=d.__path AND g.fn=d.fn AND g.__start>=d.__start AND g.__end<=d.__end
      LEFT JOIN keywords k ON k.__path=d.__path AND k.call=d.node AND g.__start>=k.__start AND g.__end<=k.__end)
      SELECT d.__path path,d.__start start_byte,d.__end end_byte,d.issue,d.identity,d.qualname,
      CASE d.issue
        WHEN 'PLP-001' THEN 'pure-lazy functions must not access runtime attribute '||quote(d.detail)
        WHEN 'PLP-002' THEN 'pure-lazy functions must not dynamically access runtime attribute '||quote(d.detail)
        WHEN 'PLP-003' THEN 'pure-lazy functions must not return runtime-bound handles or containers'
        WHEN 'PLP-004' THEN 'pure-lazy functions must not return resolved lazy wrappers containing runtime-bound state'
        WHEN 'PLP-005' THEN 'pure-lazy functions must not return locals derived from runtime-bound state'
        WHEN 'PLP-006' THEN 'pure-lazy functions must not declare runtime-handle return types'
        WHEN 'PLP-007' THEN 'exported public *_lazy functions must be decorated with @pure_lazy'
        WHEN 'PLP-008' THEN CASE WHEN n.kind='return_statement' THEN 'pure-lazy functions must not return module-level runtime state '||quote(context.name)
          WHEN context.sink IS NOT NULL THEN 'pure-lazy functions must not pass module-level runtime state '||quote(context.name)||' into runtime sink '||quote(context.sink)
          ELSE 'pure-lazy functions must not read ambient module-level runtime state '||quote(context.name) END
        WHEN 'PLP-009' THEN 'pure-lazy global cache/state access requires signed allowlist for '||quote("""+name+")||' ('||coalesce(global.category,'unknown-global-state')||') via '||quote("+access+")\n        WHEN 'PLP-010' THEN 'pure-lazy functions must not construct LazyOperation payloads with runtime field '||quote(d.detail) END message,\n      CASE WHEN d.issue='PLP-009' THEN 'global='||"+name+"||' category='||coalesce(global.category,'unknown-global-state')||' access='||"+access+" END stable_scope\n      FROM diagnostics d JOIN nodes n ON n.__path=d.__path AND n.id=d.node LEFT JOIN global_context context ON context.__path=d.__path AND context.identity=d.identity AND context.chosen=1 LEFT JOIN global_state global ON global.__path=d.__path AND global.name="+name
