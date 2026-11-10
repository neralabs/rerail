"""Require disabled native projection work to fail for its intended reason."""
import argparse
import json
from pathlib import Path
import re
import tempfile
from migration import ROOT,PIN,read_json,write_json,sha,compare
from projection_port import native_case,ProjectionTypeError


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','out']:p.add_argument('--'+name,type=Path,required=True)
    args=p.parse_args();baseline=read_json(args.baseline);compare(baseline['passed'],True)
    source={r['input']['id']:r for r in baseline['cases']}
    policy=(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-grammar-surface-project.rr').read_text()
    library=(ROOT/'docs/rule-syntax/lib/data-layer/model-projection.rr').read_text()
    specs=[('integer','projection-models:7.0','int'),('boolean','projection-models:7.0','bool'),
           ('file-order','surface-model:file-order',None),('validation','surface-model:2',None)]
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['projection_port.py','projection_contract.py','model_dtos.py','model_transport.py','surface_reporter_port.py','replay.py','migration.py']]]
    hashes={v.relative_to(ROOT).as_posix():sha(v.read_bytes()) for v in paths}
    report=dict(format='rerail.projection-controls/v1',sourcePin=PIN,passed=False,fullMigrationAcceptance=False,
        baselineSha256=sha(args.baseline.read_bytes()),engineSha256=sha(args.engine.read_bytes()),harnessFiles=hashes,cases=[])
    for name,identity,kind in specs:
        row=source[identity];changed=policy
        if kind:
            modified,count=re.subn(r'(?m)^(mp_scalar\([^\n]+"'+kind+r'"[^\n]+)\.$',r'\1, 0 = 1.',library)
            compare(count,1);changed=policy.replace('#include "data-layer/model-projection.rr"',modified)
        elif name=='file-order':changed=policy.replace('ProjectedModel(@place("",0),b,p,v)','ProjectedModel(@place("",0),b,"",v)')
        else:changed=re.sub(r'(?m)^(sv_schema\([^\n]+)\.$',r'\1 :- 0 = 1.',policy)
        assert changed!=policy
        with tempfile.TemporaryDirectory(prefix='projection-control-',dir=ROOT/'.rerail') as folder:
            target=Path(folder)/'control.rr';target.write_text(changed)
            try:actual,executions=native_case(args.engine,row['input'],policy=target)
            except ProjectionTypeError as error:
                compare(error.expected,kind);compare(error.actual,'str')
                actual=dict(protocolTypeError=dict(expected=error.expected,actual=error.actual));executions=error.executions
            else:
                assert not kind,'Disabled scalar coercion was silently repaired by the frontend'
                compare(actual==row['expected'],False)
        report['cases'].append(dict(name=name,input=row['input'],expected=row['expected'],actual=actual,
            executions=executions,mutatedPolicy=changed,passed=True))
        write_json(args.out,report)
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;write_json(args.out,report);print(dict(controls=4,passed=True))


if __name__=='__main__':main()
