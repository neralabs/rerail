"""Record complete State plugin metadata and native supplied-fact reports."""
from contextlib import redirect_stdout,redirect_stderr
from dataclasses import asdict
import io
from pathlib import Path
from types import SimpleNamespace

from contract_runner import main
from dto import canonical

CHECK='serverplan_state_tripwires'


def cases(module):
    generated={name:sorted(kinds) for name,kinds in module._load_generated_return_kinds().items()}
    result=[]
    def add(name,**extra):result.append(dict(id=name,kind='plugin',generated=generated,**extra))
    add('metadata-empty',metadataOnly=True)
    add('metadata-roots',metadataOnly=True,directories=['api','impl','data_layer','_internal','l1000_one','l9999_two','l0001_first','other','l123_too_short','lABCD_wrong'])
    add('empty-report')
    add('generated-missing',returns={})
    add('generated-extra-empty',extraReturn=True)
    add('generated-before-invalid-facts',returns={},container='dict')
    row=dict(code='SPT-003',file='impl/example.py',line=9,qualname='Step.compile',message='exact diagnostic')
    add('positive',rows=[row])
    add('duplicates',rows=[row,row])
    other=dict(row,code='SPT-005',line=11,qualname='Other.compile')
    add('key-group-order',rows=[row,other,dict(row,line=12)])
    key='SPT-003::Step.compile::impl/example.py'
    add('ratchet-suppresses-first',rows=[row,dict(row,line=12),other],allowlist=key+'\n')
    add('ratchet-stale-count',rows=[row],allowlist=key+'\n'+key+'\n')
    add('ratchet-stale-order',allowlist='Z::z::a.py\nA::a::z.py\nZ::z::a.py\n')
    add('ratchet-whitespace',rows=[row],allowlist=' # comment\r\n\u001f'+key+'\u001f\n')
    add('cross-bundle-key-order',rows=[row,other],secondRows=[dict(row,line=12)])
    for name,value in [('boolean',True),('negative',-1),('bigint',10**70),('float',1.5),('string','2'),('none',None)]:
        add('line-'+name,rows=[dict(row,line=value)])
    for name,path in [('empty',''),('relative-dots','./impl/./example.py'),('double-root','//elsewhere/impl/example.py'),('elsewhere','/elsewhere/impl/example.py')]:
        add('path-'+name,rows=[dict(row,file=path)])
    add('missing-field',rows=[{k:v for k,v in row.items() if k!='message'}])
    add('non-dict-rows',rows=[None,'text',1,False,[],row])
    add('bad-code',rows=[dict(row,code=1)])
    add('unused-json-values',rows=[dict(row,extra=dict(nested=[1,None,True,dict(x='y')]))])
    add('tuple-nested-json',rows=[row],nestedTuple=True)
    add('non-json-descendant',rows=[row],nonJson=True)
    add('non-string-key',rows=[row],nonStringKey=True)
    add('dict-container',container='dict')
    add('tuple-container',container='tuple')
    add('none-container',container='none')
    add('invalid-second-bundle',secondRows=[row],secondContainer='tuple')
    for name in ['UNSIGNED','NEEDS_SIGNING','']:
        add('production-signature-'+(name or 'empty'),productionSignature=name)
        add('work-signature-'+(name or 'empty'),workSignature=name)
    add('production-invalid-signature',productionSignature='AAAA')
    add('work-invalid-signature',workSignature='AAAA')
    add('production-before-work',productionSignature='UNSIGNED',workSignature='UNSIGNED')
    add('signatures-before-facts',productionSignature='UNSIGNED',container='dict')
    add('work-content-tamper',tamperWork=True)
    add('production-content-tamper',tamperProduction=True)
    add('missing-allowlist',rows=[row],missingAllowlist=True)
    add('directory-allowlist',allowlistDirectory=True)
    add('invalid-utf8-allowlist',allowlistHex='ff')
    add('unexpected-end-allowlist',allowlistHex='e282')
    return result


def exercise(module,case,scratch):
    root=scratch/'project';root.mkdir()
    for name in case.get('directories',[]):(root/name).mkdir()
    allow=scratch/'allowlist.txt'
    if case.get('allowlistDirectory'):allow.mkdir()
    elif 'allowlistHex' in case:allow.write_bytes(bytes.fromhex(case['allowlistHex']))
    elif not case.get('missingAllowlist'):allow.write_text(case.get('allowlist',''))
    import l2070_batching.production_ir_allowlist as production
    saved={key:getattr(module,key) for key in ['ALLOWLIST_PATH','PYTHON_WORK_ALLOWLIST_SIGNATURE','SERVERPLAN_NATIVE_PYTHON_WORK_ALLOWLIST']}
    old_signature=production.ALLOWLIST_SIGNATURE;old_entries=production.ALLOWED_PRODUCTION_IR_TYPES
    module.ALLOWLIST_PATH=allow
    if 'productionSignature' in case:production.ALLOWLIST_SIGNATURE=case['productionSignature']
    if 'workSignature' in case:module.PYTHON_WORK_ALLOWLIST_SIGNATURE=case['workSignature']
    if case.get('tamperWork'):module.SERVERPLAN_NATIVE_PYTHON_WORK_ALLOWLIST=frozenset({'tampered'})
    if case.get('tamperProduction'):production.ALLOWED_PRODUCTION_IR_TYPES=frozenset({'ForbiddenIR'})
    def rows(values,container=None):
        if container=='dict':return {'not':'a list'}
        if container=='tuple':return tuple(values)
        if container=='none':return None
        return list(values)
    data=[dict(row) if isinstance(row,dict) else row for row in case.get('rows',[])]
    if case.get('nestedTuple'):data[0]['extra']=(1,2,dict(x=(True,)))
    if case.get('nonJson'):data[0]['extra']=object()
    if case.get('nonStringKey'):data[0][1]='bad key'
    returns=case.get('returns',case['generated'])
    if case.get('extraReturn'):returns=dict(returns,extra_empty=[])
    family=module.SERVERPLAN_STATE_TRIPWIRE_EXTRACTOR.extractor_id
    ret_family=module.SERVERPLAN_RETURN_KIND_EXTRACTOR.extractor_id
    bundles={Path('first.py'):SimpleNamespace(facts={family:rows(data,case.get('container')),ret_family:returns})}
    if 'secondRows' in case:bundles[Path('second.py')]=SimpleNamespace(facts={family:rows(case['secondRows'],case.get('secondContainer'))})
    out,err=io.StringIO(),io.StringIO()
    try:
        with redirect_stdout(out),redirect_stderr(err):
            try:
                plugin=module.serverplan_state_tripwires_plugin(root)
                metadata=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description,
                    scope=asdict(plugin.file_scope),required_facts=[asdict(spec) for spec in plugin.required_facts],
                    collector_return_kinds=module.ServerPlanStateTripwireCollector.project_return_kinds)
                value=[] if case.get('metadataOnly') else plugin.reporter(SimpleNamespace(root=root,files=bundles))
                result=dict(metadata=canonical(metadata,scratch),value=canonical(value,scratch))
            except Exception as error:
                result=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    finally:
        for name,value in saved.items():setattr(module,name,value)
        production.ALLOWLIST_SIGNATURE=old_signature;production.ALLOWED_PRODUCTION_IR_TYPES=old_entries
    return dict(result,stdout=out.getvalue(),stderr=err.getvalue())


def install(backend,module,*,detectors):
    from state_tripwires_port import install as install_state
    install_state(backend,module)


if __name__=='__main__':
    main(CHECK,'rerail.state-plugin-contract/v1',cases,exercise,install,
         ['state_plugin_contract.py','state_plugin_port.py','state_tripwires_port.py'])
