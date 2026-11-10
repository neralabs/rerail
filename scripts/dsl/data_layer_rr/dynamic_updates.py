"""Check production edits, file identities and no-op runs against source recordings."""
import argparse
import json
import os
from pathlib import Path
import tempfile
from unittest.mock import patch
from migration import ROOT,PIN,read_json,write_json,sha,compare
from validate_supplied_facts_checkpoint import equal
from replay import execute_repository
from dynamic_production_port import production
from fixture_delta import apply

SEQUENCES=[['empty','clean','production-roots','clean','production-roots','production-roots','empty'],
           ['empty','file-links','directory-links','linked-scope-root','empty'],
           ['empty','no-fast-substring','fast-substring-elsewhere','type-checking','quoted','duplicate-lines','empty']]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['engine','baseline','out']:p.add_argument('--'+n,type=Path,required=True)
    p.add_argument('--mode',choices=['snapshot','workspace'],required=True);p.add_argument('--sequence',type=int,choices=range(len(SEQUENCES)),required=True);a=p.parse_args()
    source=read_json(a.baseline);compare(source['sourcePin'],PIN);compare(source['passed'],True)
    oracle={c['input']['id']:c for c in source['cases']}
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dynamic_production_port.py','first_ten_production_contract.py','fixture_delta.py','replay.py','migration.py']]]
    report=dict(format='rerail.dynamic-updates/v1',fullMigrationAcceptance=False,sourcePin=PIN,complete=False,passed=False,mode=a.mode,sequence=a.sequence,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),cases=[],harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths})
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/dynamic-module-loading.rr';write_json(a.out,report)
    with tempfile.TemporaryDirectory(prefix='dynamic-updates-') as temporary:
        base=Path(temporary);workspace=base/'corpus';workspace.mkdir();previous={};previous_mtimes={};programs={}
        class Backend:
            def __init__(self):self.calls=[]
            def native(self,check,entry,files,options=None,*,root=None):
                if check!='dynamic_module_loading' or entry not in {None,'_production_contract'} or files or root!=workspace:raise ValueError('Unexpected production update request')
                native=execute_repository(a.engine.absolute(),policy,workspace,dict(({} if entry is None else dict(entryPoint=entry)),**(options or {})),a.mode)
                for item in native['programInputs']:
                    compare(programs.setdefault(item['path'],item['sha256']),item['sha256'])
                self.calls.append(native);return native['diagnostics']
        backend=Backend()
        with patch.dict(os.environ,{'RERAIL_TAPE_CACHE':str(base/'cache.sqlite')}):
            for step,label in enumerate(SEQUENCES[a.sequence]):
                wanted=oracle[label];current={f['path']:f for f in wanted['input']['files']};changed=apply(workspace,previous,current)
                mtimes={p:(workspace/p).lstat().st_mtime_ns for p in current}
                for name in current.keys()&previous.keys()-set(changed):compare(mtimes[name],previous_mtimes[name])
                if step and label==SEQUENCES[a.sequence][step-1]:compare(changed,[]);compare(mtimes,previous_mtimes)
                actual=production(backend,workspace)
                row=dict(step=step,id=label,changedPaths=changed,expected=wanted['expected'],actual=actual,passed=equal(actual,wanted['expected']),native=list(backend.calls));backend.calls.clear()
                report['cases'].append(row);write_json(a.out,report);print(dict(step=step,id=label,passed=row['passed']),flush=True)
                previous=current;previous_mtimes=mtimes
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    report.update(complete=True,passed=all(c['passed'] for c in report['cases']));write_json(a.out,report)
    raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
