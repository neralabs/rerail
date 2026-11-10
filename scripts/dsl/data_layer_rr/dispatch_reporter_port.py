"""Transport supplied dispatch bundles and construct passive native result DTOs."""
import json
from pathlib import Path
from dto import canonical
from migration import ROOT
from fact_transport import kind,members
from python_value_transport import encode
from replay import execute_native,execute_workspace


def encode_cases(module,cases):
    from dispatch_supplied_facts import bundle_map
    result=[]
    for case in cases:
        bundles=[]
        for bundle in bundle_map(module,case).values():
            raw=bundle.facts[module.DISPATCH_EXTRACTOR.extractor_id]
            rows=[dict(key=str(key),kind=kind(row),fields=[dict(key=str(k),value=encode(v)) for k,v in members(row)]) for key,row in members(raw)]
            bundles.append(dict(path=str(bundle.path),kind=kind(raw),rows=rows))
        item=dict(root=str(Path(case['root'])),bundles=bundles,inventory=case.get('inventory') or '')
        if 'signature' in case:item['signature']=case['signature']
        result.append(item)
    return result


def decode(module,execution,count):
    rows=execution['diagnostics'];known={'DS_META','DS_FACT','DS_REPORT','DS_ERROR','DS_OUTPUT'}
    if any(r['code'] not in known for r in rows):raise ValueError('Unexpected supplied dispatch output')
    result=[]
    for c in range(count):
        def values(code):return [r['parameters'] for r in rows if r['code']==code and r['parameters']['case_id']==c]
        meta=values('DS_META')
        if len(meta)!=1:raise ValueError('Missing or duplicate supplied dispatch case: '+str(c))
        physical_root=Path(meta[0]['root']);errors=values('DS_ERROR')
        if len(errors)>1:raise ValueError('Multiple first errors')
        if errors:
            value=dict(error=dict(type=errors[0]['error_type'],args=canonical((errors[0]['detail'],))))
            findings=dict(value,stdout='',stderr='');report=dict(findings)
        else:
            def order(p):return tuple(p[k] for k in ['phase','path','sign','length','digits','source_code','bundle','ordinal'])
            facts=sorted(values('DS_FACT'),key=order)
            findings=dict(value=canonical([module.Finding(p['source_code'],Path(p['path']),json.loads(p['line_json']),p['scope'],p['expression'],p['message'],json.loads(p['occurrence_json'])) for p in facts]),stdout='',stderr='')
            reports=sorted(values('DS_REPORT'),key=order)
            report=dict(value=canonical([module.StructuredViolation('serverplan_dispatch_architecture',p['source_code'],Path(p['file']),json.loads(p['line_json']),p['message'],'hard-fail') for p in reports],physical_root),stdout='',stderr='')
            output=sorted(values('DS_OUTPUT'),key=lambda p:p['ordinal'])
            for channel in ['stdout','stderr']:report[channel]=''.join(p['text'] for p in output if p['channel']==channel)
        result.append(dict(findings=findings,report=report))
    if any(not 0<=r['parameters']['case_id']<count for r in rows):raise ValueError('Unexpected case identity')
    return result


def execute_batch(engine,module,cases,mode):
    if mode not in {'snapshot','workspace'}:raise ValueError('Unknown execution mode')
    execute=execute_native if mode=='snapshot' else execute_workspace
    execution=execute(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr',
        [dict(path='dispatch-facts.json',source=json.dumps(dict(cases=encode_cases(module,cases)),ensure_ascii=False,allow_nan=False))],dict(entryPoint='_dispatch_report_facts'))
    return decode(module,execution,len(cases)),execution
