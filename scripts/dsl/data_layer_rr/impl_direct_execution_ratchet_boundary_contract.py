"""Record remaining ratchet value boundaries without weakening the original CLI corpus."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import impl_direct_execution_ratchet_contract as contract
from migration import ROOT, PIN, compare, git, read_json, sha, write_json

CASES=[
    *[dict(id='root-'+name,count=1,baseline=value) for name,value in
        [('null',None),('boolean',True),('integer',1),('float',1.5),('string','text')]],
    *[dict(id='signature-'+name,count=1,baseline=contract.baseline(1,value)) for name,value in
        [('null',None),('boolean',True),('integer',1),('object',{'value':'bad'})]],
    dict(id='large-positive-ceiling',count=1,baseline=contract.baseline(2**80),skip=True),
    dict(id='large-negative-ceiling',count=1,baseline=contract.baseline(-(2**80)),skip=True),
    dict(id='large-float-ceiling',count=1,baseline=contract.baseline(1e100),skip=True),
    *[dict(id='conversion-'+name,count=1,baseline=contract.baseline(value)) for name,value in
        [('string','wrong'),('null',None),('list',[]),('object',{})]],
    dict(id='conversion-first-member',count=1,baseline=dict(counts={'ignored':None,'violations':'wrong'},_metadata={})),
    dict(id='non-object-metadata-skips-string-conversion',count=1,baseline=dict(counts={'violations':'wrong'},_metadata=[])),
    dict(id='unicode-integer-string-is-not-a-ceiling',count=1,baseline=contract.baseline(' +٢_٠ '),skip=True),
    dict(id='infinite-exponent-conversion',count=1,raw='{"counts":{"violations":1e999},"_metadata":{}}'),
]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('command',choices=['source','native'])
    for name in ['source','out','baseline','engine']:p.add_argument('--'+name,type=Path,required=name in ['source','out'])
    p.add_argument('--part',type=int,default=0)
    p.add_argument('--parts',type=int,default=1)
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot')
    a=p.parse_args();a.source=a.source.absolute()
    assert git(a.source,'rev-parse','HEAD')==PIN and 0<=a.part<a.parts
    files=['scripts/lint_impl_direct_execution.py','scripts/lint_lib/ratchet.py','scripts/lint_lib/signing.py']
    for name in files:compare((a.source/name).read_bytes(),subprocess.check_output(['git','-C',str(a.source),'show',PIN+':'+name]))
    dependencies=[Path(__file__),Path(contract.__file__),*[Path(contract.__file__).with_name(n) for n in ['impl_direct_execution_port.py','policy_routes.py','replay.py','migration.py']]]
    report=dict(format='rerail.impl-direct-ratchet-boundaries/v1',sourcePin=PIN,mode=a.command,executionMode=a.mode,
        fullMigrationAcceptance=False,passed=False,cases=[],sourceFiles={n:sha((a.source/n).read_bytes()) for n in files},
        harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in dependencies})
    if a.command=='native':
        assert not any(n.endswith('lint_impl_direct_execution') for n in sys.modules)
        baseline=read_json(a.baseline);assert baseline['passed'];compare(CASES,[c['input'] for c in baseline['cases']]);report['engineSha256']=sha(a.engine.read_bytes())
    for index in range(a.part,len(CASES),a.parts):
        case=CASES[index]
        if a.command=='source':
            with tempfile.TemporaryDirectory(prefix='ratchet-boundary-oracle-') as temporary:
                root=Path(temporary);write_json(root/'case.json',case)
                command=[sys.executable,str(Path(contract.__file__).absolute()),'source-one','--source',str(a.source),'--case',str(root/'case.json'),'--out',str(root/'result.json')]
                run=subprocess.run(command,env=contract.environment(case),capture_output=True,text=True,timeout=30)
                if run.returncode:raise RuntimeError(run.stderr)
                report['cases'].append(dict(input=case,expected=read_json(root/'result.json')))
        else:
            actual,calls,signatures=contract.native_one(a.engine.absolute(),case,a.source,a.mode)
            expected=baseline['cases'][index]['expected']
            report['cases'].append(dict(input=case,expected=expected,actual=actual,native=calls,signing=signatures,passed=actual==expected))
        write_json(a.out,report)
    for name,h in report['harnessFiles'].items():compare(h,sha((ROOT/name).read_bytes()))
    report['passed']=all(c.get('passed',True) for c in report['cases']);write_json(a.out,report)
    print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
