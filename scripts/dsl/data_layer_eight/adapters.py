# SPDX-License-Identifier: Apache-2.0
"""Lazy adapter ownership: direct declarations, shared bodies and signed names."""
from data_layer_six.common import A,O,K,F,H,I,P,E,S,q,literals
from data_layer_generate_batch_seven import constant
from data_layer_eight.relations import clause

def candidate(source,policy):
    module=source/'scripts/lint_lazy_adapter_ownership.py'
    p=policy('lazy_adapter_ownership',[directory+'/**/*.py' for directory in ('data_layer','impl/common','l9750_graphql','l9900_auto_api','l3200_metadata','l2000_connection')])
    p['clauses'].append({'id':'invalid-source','forbid':{'ast':{'language':'python','syntax':'invalid','rule':{'kind':'module'}}},'violation':{'code':'LAO000','message':'Cannot inspect Python source','anchor':'witness.span'}})
    p['clauses'].append({'id':'entry','define':{'options':{'entryPoint':{'type':'string','default':'scan_source'}}}})
    legacy=literals(constant(module,'LEGACY_DECORATORS'));external=literals(constant(module,'EXTERNAL_BOUNDARY_DECORATORS'));eager=literals(constant(module,'EAGER_TERMINALS'))
    stages=[
      S('declaration_parents',"SELECT n.__path,n.id,CASE WHEN p.kind='decorated_definition' THEN p.parent ELSE n.parent END parent,CASE WHEN p.kind='decorated_definition' THEN p.__start ELSE n.__start END region_start FROM nodes n LEFT JOIN nodes p ON p.__path=n.__path AND p.id=n.parent WHERE n.kind IN ('function_definition','class_definition')",('__path','id')),
      S('owned_functions',"""SELECT f.*,c.name class_name,d.region_start FROM functions f JOIN declaration_parents d ON d.__path=f.__path AND d.id=f.id
        JOIN nodes parent ON parent.__path=d.__path AND parent.id=d.parent LEFT JOIN classes c ON c.__path=parent.__path AND c.body_kind||':'||c.body_start||':'||c.body_end=parent.id LEFT JOIN declaration_parents cd ON cd.__path=c.__path AND cd.id=c.id LEFT JOIN nodes cp ON cp.__path=cd.__path AND cp.id=cd.parent
        WHERE parent.kind='module' OR c.id IS NOT NULL AND cp.kind='module'""",('__path','id'),('__path','class_name','name')),
      S('decorators',"""SELECT f.__path,f.id fn,d.id decorator,d.__start,coalesce(c.callee,a.attr,x.text) name FROM owned_functions f JOIN nodes wrapper ON wrapper.__path=f.__path AND wrapper.id=f.parent AND wrapper.kind='decorated_definition'
        JOIN nodes d ON d.__path=wrapper.__path AND d.parent=wrapper.id AND d.kind='decorator' JOIN nodes x ON x.__path=d.__path AND x.parent=d.id LEFT JOIN calls c ON c.__path=x.__path AND c.id=x.id LEFT JOIN attributes a ON a.__path=x.__path AND a.id=x.id""",('__path','fn','name')),
      S('legacy_decorators',f"SELECT * FROM (SELECT d.*,row_number() OVER(PARTITION BY __path,fn ORDER BY __start) chosen FROM decorators d WHERE name IN {legacy}) WHERE chosen=1",('__path','fn')),
      S('function_contracts',f"""SELECT f.*,d.name legacy_decorator,d.decorator,
        CASE WHEN d.name IN ('legacy_async','legacy_sync') THEN CASE WHEN f.name GLOB '*_async' THEN substr(f.name,1,length(f.name)-6) WHEN f.name GLOB '*_sync' THEN substr(f.name,1,length(f.name)-5) ELSE f.name END
        WHEN d.name='legacy_module_async' AND f.name GLOB '*_async' THEN substr(f.name,1,length(f.name)-6) ELSE f.name END||'_lazy' default_target,
        EXISTS(SELECT 1 FROM decorators x WHERE x.__path=f.__path AND x.fn=f.id AND x.name IN {external}) external,
        EXISTS(SELECT 1 FROM decorators x WHERE x.__path=f.__path AND x.fn=f.id AND x.name IN ('pure_lazy','lazy')) pure FROM owned_functions f LEFT JOIN legacy_decorators d ON d.__path=f.__path AND d.fn=f.id""",('__path','id')),
      S('function_targets',"SELECT f.*,coalesce((SELECT target FROM target_values t WHERE t.__path=f.__path AND t.decorator_start=(SELECT __start FROM legacy_decorators d WHERE d.__path=f.__path AND d.fn=f.id) ORDER BY t.__start LIMIT 1),default_target) target FROM function_contracts f",('__path','class_name','name')),
      S('last_functions',"SELECT * FROM (SELECT f.*,row_number() OVER(PARTITION BY __path,class_name,name ORDER BY __end DESC) chosen FROM function_targets f) WHERE chosen=1",('__path','class_name','name')),
      S('body_statements',"""SELECT f.__path,f.id fn,n.id,n.kind,v.kind value_kind,v.text value_text,c.callee,c.callee_kind FROM owned_functions f JOIN nodes n ON n.__path=f.__path AND n.parent=f.body_kind||':'||f.body_start||':'||f.body_end LEFT JOIN nodes v ON v.__path=n.__path AND v.parent=n.id AND v.ordinal=0 LEFT JOIN calls c ON c.__path=v.__path AND c.id=v.id WHERE n.kind!='comment'""",('__path','fn')),
      S('stubs',"""SELECT f.__path,f.id,
        EXISTS(SELECT 1 FROM body_statements b WHERE b.__path=f.__path AND b.fn=f.id) AND NOT EXISTS(SELECT 1 FROM body_statements b WHERE b.__path=f.__path AND b.fn=f.id AND NOT (b.kind IN ('pass_statement','raise_statement','delete_statement') OR b.kind='expression_statement' AND (b.value_kind IN ('string','concatenated_string') AND rx_matches(b.value_text,'(?i)^[ru]*[\x22\x27\x27]') OR b.value_kind='ellipsis' OR b.callee_kind='identifier' AND b.callee IN ('_legacy_delegate_stub','_legacy_delegate_stub_async')))) adapter,
        NOT EXISTS(SELECT 1 FROM body_statements b WHERE b.__path=f.__path AND b.fn=f.id AND NOT (b.kind IN ('pass_statement','raise_statement') OR b.kind='expression_statement' AND (b.value_kind IN ('string','concatenated_string') AND rx_matches(b.value_text,'(?i)^[ru]*[\x22\x27\x27]') OR b.value_kind='ellipsis'))) abstract FROM owned_functions f""",('__path','id')),
      S('parameter_pieces',"""WITH RECURSIVE pieces(__path,fn,name,rest,piece) AS (
        SELECT p.__path,p.fn,p.name,replace(replace(n.text,'[','|'),']','|')||'|','' FROM parameters p JOIN nodes n ON n.__path=p.__path AND n.id=p.annotation WHERE p.kind IN ('typed_parameter','typed_default_parameter')
        UNION ALL SELECT __path,fn,name,substr(rest,instr(rest,'|')+1),trim(substr(rest,1,instr(rest,'|')-1)) FROM pieces WHERE rest!='') SELECT __path,fn,name,rx_extract(piece,'([^.]*)$',1) AS type FROM pieces WHERE piece!=''"""),
      S('transaction_parameters',"SELECT DISTINCT __path,fn,name FROM parameter_pieces WHERE type GLOB '*PlanScope' OR type GLOB '*Transaction' AND rx_matches(type,'Read|Write|Schema')",('__path','fn','name')),
      S('owned_calls',"SELECT f.__path,f.id fn,c.* FROM owned_functions f JOIN calls c ON c.__path=f.__path AND c.__start>=f.region_start AND c.__end<=f.__end WHERE c.callee_kind='attribute'",('__path','fn')),
    ]
    emit=lambda where,code:f"SELECT f.__path path,f.__start start_byte,f.__end end_byte,{q(code)} issue,f.id identity FROM function_targets f WHERE {where}"
    rows=[emit("legacy_decorator IS NOT NULL AND NOT (SELECT adapter FROM stubs s WHERE s.__path=f.__path AND s.id=f.id)",'LAO001'),
      emit("legacy_decorator IS NOT NULL AND class_name IS NULL AND NOT EXISTS(SELECT 1 FROM last_functions t WHERE t.__path=f.__path AND t.class_name IS f.class_name AND t.name=f.target)",'LAO002'),
      emit("legacy_decorator IS NOT NULL AND EXISTS(SELECT 1 FROM last_functions t WHERE t.__path=f.__path AND t.class_name IS f.class_name AND t.name=f.target AND NOT t.pure)",'LAO003'),
      emit(f"pure AND EXISTS(SELECT 1 FROM owned_calls c WHERE c.__path=f.__path AND c.fn=f.id AND c.callee IN {eager})",'LAO004'),
      emit("legacy_decorator IS NULL AND NOT external AND NOT pure AND EXISTS(SELECT 1 FROM owned_calls c JOIN nodes object ON object.__path=c.__path AND object.id=c.receiver JOIN transaction_parameters p ON p.__path=c.__path AND p.fn=c.fn AND p.name=object.text WHERE c.__path=f.__path AND c.fn=f.id AND object.kind='identifier' AND c.callee NOT GLOB '*_lazy')",'LAO005'),
      emit("rx_matches(f.text,'^async\\b') AND legacy_decorator IS NULL AND NOT external AND NOT (SELECT abstract FROM stubs s WHERE s.__path=f.__path AND s.id=f.id) AND EXISTS(SELECT 1 FROM transaction_parameters p WHERE p.__path=f.__path AND p.fn=f.id) AND NOT EXISTS(SELECT 1 FROM nodes n WHERE n.__path=f.__path AND n.kind='await' AND n.__start>=f.region_start AND n.__end<=f.__end)",'LAO006')]
    string=A(O(K('string'),K('concatenated_string')),P('STRING'),{'regex':r'''(?i)^[ru]*["']'''})
    target_values=E(A(K('keyword_argument'),F('name',{'regex':'^lazy_name$'}),F('value',string),I(A(K('decorator'),P('DEC')))),
        {'decorator_start':'capture.DEC.startByte','target':'capture.DECODED.text'},transform={'DECODED':{'substring':{'source':'$STRING','value':True,'startChar':0}}})
    messages="""CASE d.issue
      WHEN 'LAO001' THEN f.name||' is a legacy adapter but its body contains executable planning or I/O'
      WHEN 'LAO002' THEN f.name||' delegates to missing module lazy target '||f.target
      WHEN 'LAO003' THEN f.name||' delegates to '||f.target||', which is not @pure_lazy'
      WHEN 'LAO004' THEN '@pure_lazy surface '||f.name||' reaches eager terminal '||(SELECT min(c.callee) FROM owned_calls c WHERE c.__path=f.__path AND c.fn=f.id AND c.callee IN """+eager+""" )
      WHEN 'LAO005' THEN 'transaction-facing function '||f.name||' owns eager calls '||(SELECT group_concat(callee,', ') FROM (SELECT DISTINCT c.callee FROM owned_calls c JOIN nodes receiver ON receiver.__path=c.__path AND receiver.id=c.receiver AND receiver.kind='identifier' JOIN transaction_parameters p ON p.__path=c.__path AND p.fn=c.fn AND p.name=receiver.text WHERE c.__path=f.__path AND c.fn=f.id AND c.callee NOT GLOB '*_lazy' ORDER BY c.callee))||' instead of delegating to one @pure_lazy surface'
      WHEN 'LAO006' THEN 'async transaction-facing function '||f.name||' has no awaited I/O and is not a legacy adapter' END"""
    query="SELECT d.*,"+messages+" message FROM ("+' UNION ALL '.join(rows)+") d JOIN function_targets f ON f.__path=d.path AND f.id=d.identity"
    item=clause('adapter-ownership',stages,query,{'target_values':target_values});item['relational']['message']='message';p['clauses'].append(item);return p
