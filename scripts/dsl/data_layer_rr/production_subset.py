"""Run recorded production cases only for the kernels that changed."""
import argparse
import json
from pathlib import Path
import first_ten_production_contract as contract
from migration import ROOT,PIN,compare,read_json,write_json,sha


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--checks',required=True)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args();assert 0<=args.part<args.parts
    baseline=read_json(args.baseline);compare(baseline['sourcePin'],PIN);compare(baseline['passed'],True)
    checks=args.checks.split(',');unknown=set(checks)-set(contract.CHECKS)
    if unknown:parser.error('Unknown production contracts: '+', '.join(sorted(unknown)))
    selected=[row for row in baseline['cases'] if row['input']['check'] in checks]
    if not selected:parser.error('No source cases for the requested checks')
    dependencies=[Path(__file__),*[Path(__file__).with_name(p) for p in [
        'first_ten_production_contract.py','replay.py','migration.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in dependencies}
    report=dict(format='rerail.production-kernel-subset/v1',sourcePin=PIN,passed=False,fullMigrationAcceptance=False,
        checks=checks,part=args.part,parts=args.parts,totalCases=len(selected),harnessFiles=hashes,
        baselineSha256=sha(args.baseline.read_bytes()),engineSha256=sha(args.engine.read_bytes()),cases=[])
    for row in selected[args.part::args.parts]:
        actual,native=contract.native_case(args.engine.absolute(),row['input'])
        passed=json.dumps(actual,sort_keys=True,ensure_ascii=False)==json.dumps(row['expected'],sort_keys=True,ensure_ascii=False)
        report['cases'].append(dict(input=row['input'],expected=row['expected'],actual=actual,native=native,passed=passed))
        write_json(args.out,report);compare(passed,True)
    for p,digest in hashes.items():compare(digest,sha((ROOT/p).read_bytes()))
    report['passed']=True;write_json(args.out,report);print(dict(cases=len(report['cases']),nativeExecutions=2*len(report['cases']),passed=True))


if __name__=='__main__':main()
