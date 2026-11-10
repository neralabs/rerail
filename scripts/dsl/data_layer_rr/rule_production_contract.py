"""Replay one rule's recorded production corpus without running unrelated kernels."""
import argparse
from pathlib import Path
from first_ten_production_contract import CHECKS,native_case
from migration import ROOT,PIN,read_json,write_json,sha,compare


def selected(source,check):
    if check not in CHECKS:raise ValueError('Unknown production contract')
    if source['sourcePin']!=PIN or not source['passed']:raise ValueError('A complete source recording is required')
    rows=[row for row in source['cases'] if row['input']['check']==check]
    if not rows:raise ValueError('Production cases are missing')
    return rows


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','out']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--check',choices=CHECKS,required=True);p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1)
    a=p.parse_args()
    if not 0<=a.part<a.parts:p.error('Invalid shard')
    source=read_json(a.baseline);cases=selected(source,a.check)
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['first_ten_production_contract.py','replay.py','migration.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}
    report=dict(format='rerail.rule-production-contract/v1',sourcePin=PIN,checks=[a.check],oracle=False,passed=False,
        fullMigrationAcceptance=False,baselineSha256=sha(a.baseline.read_bytes()),engineSha256=sha(a.engine.read_bytes()),
        part=a.part,parts=a.parts,totalCases=len(cases),harnessFiles=hashes,cases=[])
    for i in range(a.part,len(cases),a.parts):
        case=cases[i];actual,result=native_case(a.engine.absolute(),case['input'])
        row=dict(index=i,input=case['input'],expected=case['expected'],actual=actual,native=result,passed=actual==case['expected'])
        report['cases'].append(row);write_json(a.out,report)
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=all(row['passed'] for row in report['cases']);write_json(a.out,report)
    print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
