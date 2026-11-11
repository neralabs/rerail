"""Compare persistent generated-index production updates with recorded source results."""
import argparse
import os
from pathlib import Path
import tempfile
from unittest.mock import patch
from dto import canonical
from fixture_delta import apply
from generated_indexes_port import CHECK,production
from migration import ROOT,PIN,read_json,write_json,compare,sha
from replay import execute_repository
from validate_supplied_facts_checkpoint import equal

SEQUENCES=[['empty','positive','positive','order','clean','positive','empty'],
           ['empty','file-link','directory-link','positive','empty'],
           ['empty','trigger-only','invalid','helper','duplicates','empty']]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--mode',choices=['snapshot','workspace'],required=True)
    parser.add_argument('--sequence',type=int,choices=range(len(SEQUENCES)),required=True)
    args=parser.parse_args();source=read_json(args.baseline);compare(source['sourcePin'],PIN);compare(source['passed'],True)
    oracle={r['input']['id']:r for r in source['cases']}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/generated-index-sources.rr'
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['generated_indexes_port.py','fixture_delta.py','first_ten_production_contract.py','replay.py','migration.py','dto.py']]]
    report=dict(format='rerail.generated-indexes-updates/v1',sourcePin=PIN,passed=False,complete=False,mode=args.mode,
        sequence=args.sequence,engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),
        harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths},cases=[])
    with tempfile.TemporaryDirectory(prefix='generated-index-updates-') as temporary:
        root=Path(temporary)/'repo';root.mkdir();previous={};previous_mtimes={}
        class Backend:
            def __init__(self):self.calls=[]
            def native(self,check,entry,files,options=None,*,root=None):
                compare(check,CHECK);compare(files,[])
                result=execute_repository(args.engine.absolute(),policy,root,dict(({} if entry is None else dict(entryPoint=entry)),**(options or {})),args.mode)
                self.calls.append(result);return result['diagnostics']
        backend=Backend()
        with patch.dict(os.environ,{'RERAIL_TAPE_CACHE':str(Path(temporary)/'cache.sqlite')}):
            for step,label in enumerate(SEQUENCES[args.sequence]):
                row=oracle['production-'+label];current={f['path']:f for f in row['input']['files']}
                changed=apply(root,previous,current);mtimes={p:(root/p).lstat().st_mtime_ns for p in current}
                for name in current.keys()&previous.keys()-set(changed):compare(mtimes[name],previous_mtimes[name])
                try:
                    actual=dict(value=canonical(production(backend,root),Path(temporary)))
                except Exception as error:
                    report['cases'].append(dict(id=row['input']['id'],step=step,changedPaths=changed,expected=row['expected'],error=str(error),passed=False,native=list(backend.calls)))
                    write_json(args.out,report)
                    raise
                report['cases'].append(dict(id=row['input']['id'],step=step,changedPaths=changed,expected=row['expected'],actual=actual,passed=equal(actual,row['expected']),native=list(backend.calls)))
                backend.calls.clear();previous=current;previous_mtimes=mtimes;write_json(args.out,report)
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    report.update(complete=True,passed=all(r['passed'] for r in report['cases']));write_json(args.out,report)
    print(dict(steps=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
