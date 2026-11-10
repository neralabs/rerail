"""Transport supplied dispatch bundles and construct passive native result DTOs."""
import json
from pathlib import Path
from dto import canonical
from migration import ROOT
from fact_transport import kind,members
from python_value_transport import encode
from replay import execute_native,execute_workspace


def encode_views(views):
    result=[]
    for view in views:
        bundles=[]
        for bundle in view.files.values():
            families={}
            for name,raw in bundle.facts.items():
                rows=[dict(key=str(key),kind=kind(row),fields={str(k):encode(v) for k,v in members(row)}) for key,row in members(raw)]
                families[name]=dict(kind=kind(raw),rows=rows)
            bundles.append(dict(path=str(bundle.path),facts=families))
        result.append(dict(root=str(view.root),bundles=bundles))
    return result


def encode_cases(module,cases):
    from dispatch_supplied_facts import bundle_map
    from types import SimpleNamespace
    result=encode_views([SimpleNamespace(root=Path(c['root']),files=bundle_map(module,c)) for c in cases])
    for item,case in zip(result,cases):
        item['inventory']=case.get('inventory') or ''
        if 'signature' in case:item['signature']=case['signature']
    return result


def finding_rows(module,rows,case_id=0):
    def values(code):return [r['parameters'] for r in rows if r['code']==code and r['parameters']['case_id']==case_id]
    meta=values('DS_META')
    if len(meta)!=1:raise ValueError('Missing or duplicate supplied Finding case')
    errors=values('DS_ERROR')
    if errors:
        if len(errors)!=1:raise ValueError('Multiple first errors')
        import builtins
        error=errors[0]
        if error['error_type'] not in {'TypeError','ValueError','OverflowError'}:raise ValueError('Unexpected native error type')
        raise getattr(builtins,error['error_type'])(error['detail'])
    ordered=sorted(values('DS_FACT'),key=lambda p:tuple(p[k] for k in ['phase','path','sign','length','digits','source_code','bundle','ordinal']))
    return [module.Finding(p['source_code'],Path(p['path']),json.loads(p['line_json']),p['scope'],p['expression'],p['message'],json.loads(p['occurrence_json'])) for p in ordered]


def findings_for_view(backend,module,view):
    payload=dict(cases=encode_views([view]))
    rows=backend.native('serverplan_dispatch_architecture','_dispatch_findings',
        [dict(path='dispatch-facts.json',source=json.dumps(payload,ensure_ascii=False,allow_nan=False))])
    if any(r['code'] not in {'DS_META','DS_FACT','DS_ERROR'} or r['parameters']['case_id']!=0 for r in rows):raise ValueError('Unexpected supplied Finding response')
    return finding_rows(module,rows)


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
            report=dict(value=canonical([module.StructuredViolation(p['linter_name'],p['source_code'],Path(p['file']),json.loads(p['line_json']),p['message'],p['severity']) for p in reports],physical_root),stdout='',stderr='')
            output=sorted(values('DS_OUTPUT'),key=lambda p:p['ordinal'])
            for channel in ['stdout','stderr']:report[channel]=''.join(p['text'] for p in output if p['channel']==channel)
        result.append(dict(findings=findings,report=report))
    if any(not 0<=r['parameters']['case_id']<count for r in rows):raise ValueError('Unexpected case identity')
    return result


def execute_batch(engine,module,cases,mode,policy=None):
    if mode not in {'snapshot','workspace'}:raise ValueError('Unknown execution mode')
    execute=execute_native if mode=='snapshot' else execute_workspace
    execution=execute(engine,policy or ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr',
        [dict(path='dispatch-facts.json',source=json.dumps(dict(cases=encode_cases(module,cases)),ensure_ascii=False,allow_nan=False))],dict(entryPoint='_dispatch_report_facts'))
    try:return decode(module,execution,len(cases)),execution
    except Exception as error:
        error.execution=execution
        raise


def execute_findings_batch(engine,module,cases,mode):
    from dispatch_supplied_facts import bundle_map
    from dispatch_port import install
    from types import SimpleNamespace
    from contextlib import redirect_stdout,redirect_stderr
    import io
    views=[SimpleNamespace(root=Path(c['root']),files=bundle_map(module,c)) for c in cases]
    packed=encode_views(views)
    execute=execute_native if mode=='snapshot' else execute_workspace
    execution=execute(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr',
        [dict(path='dispatch-facts.json',source=json.dumps(dict(cases=packed),ensure_ascii=False,allow_nan=False))],dict(entryPoint='_dispatch_findings'))
    groups={i:[] for i in range(len(cases))}
    for row in execution['diagnostics']:
        if row['code'] not in {'DS_META','DS_FACT','DS_ERROR'}:raise ValueError('Unexpected Finding batch row')
        i=row['parameters']['case_id']
        groups[i].append(dict(row,parameters=dict(row['parameters'],case_id=0)))
    cache={}
    for i,payload in enumerate(packed):
        key=json.dumps(payload,sort_keys=True,ensure_ascii=False)
        if key in cache and cache[key]!=groups[i]:raise ValueError('Equal supplied inputs returned different rows')
        cache[key]=groups[i]
    class Backend:
        def replace(self,owner,name,value):setattr(owner,name,value)
        def native(self,check,entry,files,options=None):
            if check!='serverplan_dispatch_architecture' or entry!='_dispatch_findings' or options:raise ValueError('Unexpected Finding request')
            payload=json.loads(files[0]['source'])['cases']
            if len(files)!=1 or len(payload)!=1:raise ValueError('Unexpected Finding transport')
            return cache[json.dumps(payload[0],sort_keys=True,ensure_ascii=False)]
    install(Backend(),module)
    results=[]
    for view in views:
        out,err=io.StringIO(),io.StringIO()
        with redirect_stdout(out),redirect_stderr(err):
            try:result=dict(value=canonical(module._findings(view)))
            except Exception as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
        results.append(dict(result,stdout=out.getvalue(),stderr=err.getvalue()))
    return results,execution
