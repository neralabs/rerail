"""Require scanner-family and native inventory-error removals to fail source parity."""
import argparse
from contextlib import redirect_stdout,redirect_stderr
import importlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from migration import ROOT,PIN,MANIFEST,compare,git,read_json,sha,write_json,verify_source_cohort,verify_engine_artifact
from dto import canonical
from replay import execute_native,execute_workspace,program_inputs
from dispatch_cli_contract import fixture
from dispatch_inventory_io_contract import exception
from dispatch_repository_contract import decode


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['source','engine','baseline','io-baseline','out']:p.add_argument('--'+n,type=Path,required=True)
    p.add_argument('--mode',choices=['snapshot','workspace'],required=True);a=p.parse_args();source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    baseline=read_json(a.baseline);io_source=read_json(a.io_baseline)
    for oracle in [baseline,io_source]:compare(oracle['passed'],True);compare(oracle['sourcePin'],PIN);compare(oracle['sourceSha256'],sha((source/'scripts/lint_serverplan_dispatch_architecture.py').read_bytes()))
    from data_layer_audit.native_guard import codes
    original=codes({'dispatch':module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in original and not original[frame.f_code][2]:fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    rows=[];files=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dispatch_inventory_io_contract.py','dispatch_cli_contract.py','dispatch_repository_contract.py','migration.py','replay.py','dto.py']]]
    report=dict(format='rerail.dispatch-scanner-controls/v1',sourcePin=PIN,admitted=False,complete=False,passed=False,mode=a.mode,cases=rows,semanticFallbacks=fallbacks,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),ioBaselineSha256=sha(a.io_baseline.read_bytes()),harnessFiles={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in files});write_json(a.out,report)
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr';library=ROOT/'docs/rule-syntax/lib/data-layer/dispatch-source.rr';text=library.read_text()
    families=[f'SPD00{i}' for i in range(1,6)];selected=[]
    for code in families:
        selected.append(next(r for r in baseline['cases'] if len(r['expected'].get('value',{}).get('items',[]))==1 and r['expected']['value']['items'][0]['fields']['code']==code))
    buffers=[dict(path=f'case-{i}.py',source=r['input']['source']) for i,r in enumerate(selected)]
    options=dict(entryPoint='scan_source',dispatchDocuments=[f['path'] for f in buffers],dispatchPaths=[r['input']['path'] for r in selected])
    execute=execute_native if a.mode=='snapshot' else execute_workspace
    for index,code in enumerate(families):
        changed=text.replace(':- sd_hit(n,layer,code),',f':- sd_hit(n,layer,code),code!="{code}",');compare(changed!=text,True)
        mutation=policy.read_text().replace('#include "data-layer/dispatch-source.rr"',changed)
        with tempfile.TemporaryDirectory(prefix='dispatch-scanner-control-',dir=ROOT/'.rerail') as temporary:
            path=Path(temporary)/'control.rr';path.write_text(mutation)
            sys.setprofile(guard)
            try:execution=execute(a.engine.absolute(),path,buffers,options)
            finally:sys.setprofile(None)
        groups={f['path']:[] for f in buffers}
        for r in execution['diagnostics']:
            if r['code'] not in families:raise AssertionError('Unexpected control output')
            groups[r['parameters']['document']].append(r)
        actual=[]
        for i,witness in enumerate(selected):
            values=sorted(groups[f'case-{i}.py'],key=lambda r:(r['line'],r['code'],r['parameters']['expression'],r['parameters']['occurrence']))
            actual.append(dict(value=canonical(tuple(module.Finding(r['code'],Path(witness['input']['path']),r['line'],r['parameters']['scope'],r['parameters']['expression'],r['message'],r['parameters']['occurrence']) for r in values))))
            compare(actual[-1],dict(value=canonical(tuple())) if i==index else witness['expected'])
        rows.append(dict(id=code,expected=selected[index]['expected'],actual=actual[index],otherFamiliesUnchanged=True,execution=execution,mutatedPolicy=mutation,mutatedPolicySha256=sha(mutation.encode()),passed=True));write_json(a.out,report)
    library=ROOT/'docs/rule-syntax/lib/data-layer/dispatch-repository.rr';text=library.read_text();needle='file_error(p,k,n,d,b,s,e,r).';compare(text.count(needle),1)
    mutation=policy.read_text().replace('#include "data-layer/dispatch-repository.rr"',text.replace(needle,'file_error(p,k,n,d,b,s,e,r),k="SuppressedError".'))
    cohort=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,cohort);verify_engine_artifact(ROOT,cohort,a.engine.absolute())
    for identity in ['directory','invalid-start']:
        oracle=next(r for r in io_source['cases'] if r['input']['id']==identity)
        with tempfile.TemporaryDirectory(prefix='dispatch-io-control-',dir=ROOT/'.rerail') as temporary:
            base=Path(temporary);root=base/'corpus';root.mkdir();fixture(root,oracle['input']['files']);path=base/'control.rr';path.write_text(mutation)
            options=dict(entryPoint='_production_contract')
            if a.mode=='snapshot':options['inputFiles']=['impl/common/l2065_plan_core/a.py']
            write_json(base/'options.json',options);run=subprocess.run([str(a.engine.absolute()),'run-rr',str(path),str(root),str(base/'out'),str(base/'options.json')],capture_output=True,text=True,timeout=100);compare(run.returncode,0)
            execution=json.loads(run.stdout);execution['programInputs']=program_inputs(execution['programInputs']);out,err=io.StringIO(),io.StringIO()
            sys.setprofile(guard)
            try:
                with redirect_stdout(out),redirect_stderr(err):
                    try:result=decode(module,execution);actual=dict(value=result['violations'],stdout=result['stdout'],stderr=result['stderr'])
                    except BaseException as error:actual=dict(error=exception(error,root),stdout=out.getvalue(),stderr=err.getvalue())
            finally:sys.setprofile(None)
        compare('error' in oracle['expected'],True);compare('error' in actual,False);compare(len(actual['value']),1)
        rows.append(dict(id='inventory-'+identity,expected=oracle['expected'],actual=actual,execution=execution,mutatedPolicy=mutation,mutatedPolicySha256=sha(mutation.encode()),passed=True));write_json(a.out,report)
    report.update(complete=True,passed=not fallbacks);write_json(a.out,report);print(dict(controls=len(rows),passed=report['passed']))


if __name__=='__main__':main()
