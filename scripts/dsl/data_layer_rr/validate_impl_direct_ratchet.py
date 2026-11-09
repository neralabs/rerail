"""Validate complete recorded ratchet parity, with both APIs and immutable inputs."""
import argparse
import gzip
import json
from pathlib import Path

from impl_direct_execution_ratchet_contract import CASES
from migration import ROOT, PIN, MANIFEST, compare, read_json, sha, verify_engine_artifact, verify_source_cohort


def validate(source, reports, engine):
    compare(source['sourcePin'], PIN)
    assert source['mode']=='source' and source['passed']
    compare([c['input'] for c in source['cases']], CASES)
    cohort=read_json(ROOT/MANIFEST)['engineSource']
    verify_source_cohort(ROOT,cohort)
    verify_engine_artifact(ROOT,cohort,engine)
    digest=sha(engine.read_bytes())
    seen={mode:set() for mode in ['snapshot','workspace']}
    executions=0
    signing=0
    for report in reports:
        assert report['mode']=='native' and report['passed']
        compare(report['sourcePin'],PIN)
        compare(report['engineSha256'],digest)
        compare(report['sourceFiles'],source['sourceFiles'])
        for name,h in report['harnessFiles'].items():
            compare(h,sha((ROOT/name).read_bytes()))
        mode=report['executionMode']
        assert mode in seen
        selection=report['selection']
        wanted=list(range(selection['part'],len(CASES),selection['parts']))
        compare([c['input'] for c in report['cases']],[CASES[i] for i in wanted])
        for index,case in zip(wanted,report['cases']):
            assert index not in seen[mode],(mode,index)
            seen[mode].add(index)
            assert case['passed']
            compare(case['expected'],source['cases'][index]['expected'])
            compare(case['actual'],case['expected'])
            assert case['native']
            for execution in case['native']:
                assert execution['programInputs']
                for program in execution['programInputs']:
                    path=Path(program['path'])
                    if not path.is_absolute():path=ROOT/path
                    compare(program['sha256'],sha(path.read_bytes()))
                executions+=1
            signing+=len(case['signing'])
    for mode,indices in seen.items():compare(indices,set(range(len(CASES))))
    return dict(format='rerail.impl-direct-ratchet-checkpoint/v1',sourcePin=PIN,
        fullMigrationAcceptance=False,passed=True,engineSource=cohort,engineSha256=digest,
        summary=dict(casesPerAPI=len(CASES),APIs=list(seen),verifiedNativeExecutions=executions,
            signingBatches=signing,failures=0,skippedSelectedAssertions=0),
        source=source,reports=reports,validator=Path(__file__).read_text())


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,required=True)
    p.add_argument('--engine',type=Path,required=True)
    p.add_argument('--reports',type=Path,nargs='+',required=True)
    p.add_argument('--out',type=Path,required=True)
    a=p.parse_args()
    result=validate(read_json(a.source),[read_json(n) for n in a.reports],a.engine.absolute())
    a.out.write_bytes(gzip.compress(json.dumps(result,sort_keys=True,separators=(',',':')).encode(),mtime=0))
    print(json.dumps(result['summary']))


if __name__=='__main__':main()
