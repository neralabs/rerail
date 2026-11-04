# SPDX-License-Identifier: Apache-2.0
"""The production post-op policy: signed inventories and source governance.

The old behavior visitor is not called by this policy. Its standalone tests
remain recorded helper observations and do not justify a new taint engine.
"""
import ast
from data_layer_six.common import A,O,N,K,F,H,I,P,E,S,names,call,optional,q,literals

POST='l2070_batching/post_op.py'
SPECS='l2070_batching/post_op_specs.py'
DECODERS='l2070_batching/post_op_domain_decoders.py'
REGISTRY='l2070_batching/post_op_allowlist.py'


def fates(path):
    tree=ast.parse(path.read_text())
    values={t.id:node.value.value for cls in tree.body if isinstance(cls,ast.ClassDef) and cls.name=='PostOpFate' for node in cls.body
            if isinstance(node,ast.Assign) and isinstance(node.value,ast.Constant) for t in node.targets if isinstance(t,ast.Name)}
    for node in tree.body:
        if isinstance(node,ast.AnnAssign) and isinstance(node.target,ast.Name) and node.target.id=='POST_OP_FATE_OVERRIDES':
            return sorted(k.value+'='+values[v.attr] for k,v in zip(node.value.keys,node.value.values))
    raise ValueError('Missing fate map')


def candidate(source,policy,constant):
    module=source/'scripts/lint_post_op_side_effects.py'
    p=policy('post_op_side_effects',[POST,SPECS,DECODERS,REGISTRY])
    p['clauses'].append({'id':'contract','define':{'options':{
        'entryPoint':{'type':'string','default':'plugin.reporter'},
        'fatesSignature':{'type':'string','default':constant(module,'POST_OP_FATES_SIGNATURE')},
        'surfaceSignature':{'type':'string','default':constant(module,'POST_OP_SURFACE_SIGNATURE')},
        'fateOverrides':{'type':'array','items':'string','default':fates(module)},
    }}})
    top=O(I(K('module'),'neighbor'), I(A(K('decorated_definition'),I(K('module'),'neighbor')),'neighbor'))
    assignment_top=I(A(K('expression_statement'),I(K('module'),'neighbor')),'neighbor')
    function=A(K('function_definition'),P('FN'),F('name',P('NAME')),top)
    parent_class=A(K('class_definition'),P('CLASS'),F('name',P('CLASS_NAME')),top)
    declared=A(K('assignment'),F('left',A(K('identifier'),P('NAME'))),I(parent_class))
    string=A(O(K('string'),K('concatenated_string')),P('STRING'),{'regex':r'''(?i)^[ru]*["']'''})
    def decoded(rule,project=None,each=None):
        return E(rule,{'value':'capture.DECODED.text',**(project or {})},each=each,
                 transform={'DECODED':{'substring':{'source':'$STRING','value':True,'startChar':0}}})
    registry_assignment=A(K('assignment'),F('left',names(['SIGNED_POST_OP_SYMBOLS'])),assignment_top)
    registry_string=A(string,I(registry_assignment))
    signature_assignment=A(K('assignment'),F('left',names(['ALLOWLIST_SIGNATURE'])),F('type',P('TYPE')),assignment_top)
    tokens=constant(module,'FORBIDDEN_RUNTIME_REF_TOKENS')
    # Name and arg nodes share identifier syntax. Declaration/import/keyword and
    # attribute labels are not ast.Name; the Attribute node handles its own label.
    reference_name=A(names(tokens),N(I(O(K('import_statement'),K('import_from_statement')))),
        *[N(I(K(k),'neighbor',field=f)) for k,f in [('function_definition','name'),('class_definition','name'),('keyword_argument','name'),('attribute','attribute')]])
    reference=O(reference_name,A(K('attribute'),F('attribute',names(tokens))),A(string,{'value':{'regex':'^(?:'+'|'.join(sorted(tokens))+')$'}}))
    inputs={
        'modules':E(K('module'),{'path':'file.path'},anchor='file.start'),
        'functions':E(function,{'name':'capture.NAME','sync':{'source':'capture.FN','regex':'^(def)\\b','group':1}}),
        'members':E(declared,{'class_name':'capture.CLASS_NAME','name':'capture.NAME'}),
        'specs':E(A(parent_class,F('name',{'regex':'Spec$'}),I(A(K('decorated_definition'),H(A(K('decorator'),O(H(names(['dataclass'])),H(A(K('call'),F('function',names(['dataclass'])))))))), 'neighbor')),{'name':'capture.CLASS_NAME'}),
        'strings':decoded(string),
        'registry':decoded(registry_string),
        'signature':decoded(A(string,I(signature_assignment))),
        'runtime_refs':E(reference,{'path':'file.path'}),
        'flags':E(A(K('assignment'),F('left',names(['ENFORCE_POST_OP_RUNTIME_ALLOWLIST'])),assignment_top),{'path':'file.path'}),
        'verification':E(A(call(['_post_op_allowlist_signature_valid','verify_allowlist']),P('NODE')),{'line':'capture.NODE.startLine'}),
        'returns':E(A(K('return_statement'),P('NODE')),{'line':'capture.NODE.startLine'}),
    }
    pub=constant(source/'scripts/lint_lib/signing.py','PUBLIC_KEY_HEX')
    # Configuration is data: hexadecimal UTF-8 SQL text expressions avoid
    # interpreting quotes, NULs or Unicode as query syntax.
    configured=lambda name,template:{'$option':name,'escape':'sql','template':template}
    stages=[
        S('overrides',{'$option':'fateOverrides','escape':'sql','map':' UNION ALL SELECT {} AS entry','join':'','template':"SELECT '' AS entry WHERE 0{}"}),
        S('memberships',f"SELECT DISTINCT class_name||'.'||name AS symbol FROM members WHERE (__path={q(POST)} OR __path={q(SPECS)}) AND class_name IN ('PostOpKind','PostOpBehavior')"),
        S('fates',"SELECT m.symbol,coalesce((SELECT substr(o.entry,instr(o.entry,'=')+1) FROM overrides o WHERE substr(o.entry,1,instr(o.entry,'=')-1)=m.symbol),'audit_required') AS fate FROM memberships m"),
        S('surface',f'''SELECT DISTINCT class_name||'.'||name AS symbol FROM members WHERE (__path={q(POST)} AND class_name IN ('PostOpKind','PostOpBehavior')) OR (__path={q(SPECS)} AND class_name='DomainPostOpDecoderKind')
          UNION SELECT 'post_op_factory.'||name FROM functions WHERE __path={q(POST)} AND name GLOB '*_post_op' AND sync='def'
          UNION SELECT 'post_op_spec.'||name FROM specs WHERE __path={q(SPECS)}'''),
        S('reachable',f'''SELECT DISTINCT s.value AS symbol FROM strings s WHERE s.__path={q(POST)} AND (s.value GLOB 'assertion.*' OR s.value GLOB 'materializer.*')
            AND EXISTS(SELECT 1 FROM functions f WHERE f.__path=s.__path AND f.name='post_op_runtime_symbol' AND f.__start<s.__start AND f.__end>=s.__end
                AND f.__start=(SELECT min(x.__start) FROM functions x WHERE x.__path=f.__path AND x.name=f.name))
          UNION SELECT 'decoder.'||class_name||'.'||name FROM members WHERE __path={q(SPECS)} AND class_name='DomainPostOpDecoderKind' ''')]
    def message(name,select):
        return "'{\"allowlist\":'||coalesce((SELECT json_group_array(value) FROM ("+select+" ORDER BY value)),'[]')||',\"linter\":\""+name+"\",\"version\":1}'"
    fate_message=message('post_op_side_effects',"SELECT symbol||'='||fate AS value FROM fates")
    surface_message=message('post_op_surface','SELECT symbol AS value FROM surface')
    runtime_message=message('post_op_runtime_allowlist',f'SELECT value FROM registry WHERE __path={q(REGISTRY)}')
    stages += [
        S('fate_valid',configured('fatesSignature',f"SELECT verify_ed25519({q(pub)},{fate_message},rx_replace({{}},'[^A-Za-z0-9+/=]','')) AS valid")),
        S('surface_valid',configured('surfaceSignature',f"SELECT verify_ed25519({q(pub)},{surface_message},rx_replace({{}},'[^A-Za-z0-9+/=]','')) AS valid")),
        S('runtime_valid',f"SELECT verify_ed25519({q(pub)},{runtime_message},rx_replace(coalesce((SELECT value FROM signature WHERE __path={q(REGISTRY)} ORDER BY __start LIMIT 1),''),'[^A-Za-z0-9+/=]','')) AS valid"),
        S('verifiers',"SELECT f.*,(SELECT min(v.line) FROM verification v WHERE v.__path=f.__path AND v.__start>=f.__start AND v.__end<=f.__end) AS first_verify FROM functions f WHERE f.name='assert_post_op_symbol_signed' AND f.__start=(SELECT min(x.__start) FROM functions x WHERE x.__path=f.__path AND x.name=f.name)"),
        S('bypass',"SELECT m.__path FROM modules m WHERE EXISTS(SELECT 1 FROM flags f WHERE f.__path=m.__path) OR NOT EXISTS(SELECT 1 FROM verifiers f WHERE f.__path=m.__path) OR EXISTS(SELECT 1 FROM verifiers f WHERE f.__path=m.__path AND (f.first_verify IS NULL OR EXISTS(SELECT 1 FROM returns r WHERE r.__path=f.__path AND r.__start>=f.__start AND r.__end<=f.__end AND r.line<f.first_verify)))"),
    ]
    row=lambda code,identity,table,where='1':f"SELECT {q(POST)} AS path,0 AS start_byte,0 AS end_byte,{q(code)} AS issue,{identity} AS identity FROM {table} WHERE {where}"
    broad="value NOT GLOB 'assertion.*' AND value NOT GLOB 'decoder.DomainPostOpDecoderKind.*' AND value NOT GLOB 'materializer.*' OR value IN ('materializer.generic','materializer.materialize_result','materializer.terminal_array','materializer.terminal_object','materializer.terminal_scalar')"
    rows=[
        row('POSTOP200',"__path||':'||__start",'runtime_refs',f'__path IN ({q(POST)},{q(SPECS)},{q(DECODERS)})'),
        row('POSTOP203',"'bypass'",'bypass',f'__path={q(REGISTRY)}'),
        row('POSTOP204',"__start",'registry',f'__path={q(REGISTRY)} AND ({broad})'),
        row('POSTOP205','symbol','reachable',f'NOT EXISTS(SELECT 1 FROM registry r WHERE r.__path={q(REGISTRY)} AND r.value=reachable.symbol)'),
        row('POSTOP206','value','registry',f'__path={q(REGISTRY)} AND NOT EXISTS(SELECT 1 FROM reachable e WHERE e.symbol=registry.value)'),
        row('POSTOP202',"'runtime-signature'",'runtime_valid','NOT valid'),
        row('POSTOP201',"'surface-signature'",'surface_valid','NOT valid'),
    ]
    # Only the production reporter discards unsigned fate authority. The raw
    # helper receives the explicit overrides in its own original contract.
    stages.append(S('trusted_overrides',{'$option':'entryPoint','cases':{'_collect_violations':'SELECT entry FROM overrides'},'default':'SELECT entry FROM overrides WHERE (SELECT valid FROM fate_valid)'}))
    rows += [row('POSTOP001','entry','trusted_overrides',"NOT EXISTS(SELECT 1 FROM memberships m WHERE m.symbol=substr(entry,1,instr(entry,'=')-1))"),
             row('POSTOP002','symbol','memberships',"coalesce((SELECT substr(entry,instr(entry,'=')+1) FROM trusted_overrides o WHERE substr(entry,1,instr(entry,'=')-1)=memberships.symbol),'audit_required') IN ('audit_required','delete_after_inline','move_to_domain_step','move_to_plan_effect','move_to_sql')")]
    stages.append(S('findings',' UNION ALL '.join(rows)))
    all_query="SELECT * FROM findings UNION ALL SELECT "+q(POST)+",0,0,'POSTOP000','fates-signature' WHERE NOT (SELECT valid FROM fate_valid) AND NOT EXISTS(SELECT 1 FROM findings)"
    direct="SELECT m.__path AS path,0 AS start_byte,0 AS end_byte,'POSTOP203' AS issue,'bypass' AS identity FROM modules m JOIN bypass b ON b.__path=m.__path"
    query={'$option':'entryPoint','cases':{
        '_collect_runtime_allowlist_bypass_violations_from_tree':direct,
        '_collect_violations':'SELECT * FROM findings',
        '_collect_broad_runtime_authority_violations':"SELECT * FROM findings WHERE issue='POSTOP204'",
        '_collect_exact_runtime_symbol_coverage_violations':"SELECT * FROM findings WHERE issue IN ('POSTOP205','POSTOP206')",
    },'default':all_query}
    p['clauses'].append({'id':'signed-surface','relational':{'inputs':inputs,'stages':stages,'locations':[POST],'query':query,'key':['issue','identity'],'code':'issue'},
        'violation':{'code':'POSTOP203','anchor':'witness.span','message':'Signed post-op boundary'}})
    return p
