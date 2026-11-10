"""Run pinned source observations and native production queries for explicit cases."""
import argparse
import importlib
from pathlib import Path
import subprocess
import sys
from migration import ROOT,PIN,read_json,write_json,compare,sha,git
from first_ten_production_contract import source_case,native_case
from validate_supplied_facts_checkpoint import equal


def main(check,format,cases,caller):
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=name=='out')
    args=parser.parse_args()
    paths=[Path(caller),Path(__file__),*[Path(__file__).with_name(n) for n in ['first_ten_production_contract.py','replay.py','migration.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}
    if args.command=='source':
        source=args.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN);path=source/f'scripts/lint_{check}.py'
        compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+path.relative_to(source).as_posix()]))
        sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+check)
        report=dict(format=format+'-source/v1',sourcePin=PIN,passed=True,sourceSha256=sha(path.read_bytes()),harnessFiles=hashes,
            cases=[dict(input=case,expected=source_case(module,case)) for case in cases])
    else:
        baseline=read_json(args.baseline);compare(baseline['passed'],True);compare([r['input'] for r in baseline['cases']],cases)
        assert not any(name.startswith('scripts.lint_') for name in sys.modules)
        report=dict(format=format+'-native/v1',sourcePin=PIN,oracle=False,passed=False,engineSha256=sha(args.engine.read_bytes()),
            baselineSha256=sha(args.baseline.read_bytes()),harnessFiles=hashes,cases=[])
        for row in baseline['cases']:
            actual,native=native_case(args.engine.absolute(),row['input'])
            report['cases'].append(dict(input=row['input'],expected=row['expected'],actual=actual,native=native,passed=equal(actual,row['expected'])))
            write_json(args.out,report)
        report['passed']=all(r['passed'] for r in report['cases'])
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    write_json(args.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)
