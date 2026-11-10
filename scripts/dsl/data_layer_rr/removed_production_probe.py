"""Verify the production trigger boundary separately from the direct scanner API."""
import argparse
import importlib
from pathlib import Path
import subprocess
import sys
from migration import ROOT,PIN,read_json,write_json,compare,sha,git
from first_ten_production_contract import source_case,native_case

CASES=[dict(id='removed_compat_surfaces:unicode-'+label,check='removed_compat_surfaces',
            files=[dict(path='impl/a.py',source=text)]) for label,text in
       [('off','ＴimerClaim\n'),('on','# TimerClaim\nＴimerClaim\n')]]


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=name=='out')
    a=p.parse_args()
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in
            ['first_ten_production_contract.py','replay.py','migration.py']]]}
    if a.command=='source':
        source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
        path=source/'scripts/lint_removed_compat_surfaces.py'
        compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':scripts/lint_removed_compat_surfaces.py']))
        sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_removed_compat_surfaces')
        report=dict(format='rerail.removed-production-trigger-source/v1',sourcePin=PIN,sourceSha256=sha(path.read_bytes()),
                    passed=True,harnessFiles=hashes,cases=[dict(input=c,expected=source_case(module,c)) for c in CASES])
    else:
        baseline=read_json(a.baseline);compare(baseline['sourcePin'],PIN);compare(baseline['passed'],True)
        compare([r['input'] for r in baseline['cases']],CASES)
        assert not any(n.startswith('scripts.lint_') for n in sys.modules)
        report=dict(format='rerail.removed-production-trigger-native/v1',sourcePin=PIN,oracle=False,passed=False,
                    engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),harnessFiles=hashes,cases=[])
        for row in baseline['cases']:
            actual,native=native_case(a.engine.absolute(),row['input'])
            compare(actual,row['expected'])
            report['cases'].append(dict(input=row['input'],expected=row['expected'],actual=actual,native=native,passed=True))
        report['passed']=True
    for name,digest in hashes.items():compare(sha((ROOT/name).read_bytes()),digest)
    write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']))


if __name__=='__main__': main()
