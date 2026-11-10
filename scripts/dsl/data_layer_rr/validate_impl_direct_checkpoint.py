"""Validate the complete direct-execution checkpoint without dropping any corpus family."""
import argparse
import gzip
import json
from pathlib import Path

from impl_direct_execution_controls import mutations
from impl_direct_execution_production_contract import CASES as PRODUCTION
from impl_direct_execution_signing_contract import CASES as SIGNING
from impl_direct_execution_source_contract import CASES as API, RULE
from impl_direct_execution_updates import STEPS
from impl_direct_execution_ratchet_boundary_contract import CASES as BOUNDARIES
from migration import ROOT, compare, read_json, sha
from validate_impl_direct_ratchet import validate, validate_original


def complete(work, base, engine):
    digest=sha(engine.read_bytes())
    compare(base['engineSha256'],digest)
    validate(base['source'],base['reports'],engine)
    validate(base['boundaries']['source'],base['boundaries']['reports'],engine,BOUNDARIES)
    original=base['originalSuite']
    validate_original(original['source'],original['reports'],engine)
    reports={};programs={};executions=[]

    def consume(value):
        for run in value if isinstance(value,list) else [value]:
            assert run['programInputs']
            for program in run['programInputs']:
                path=Path(program['path'])
                if not path.is_absolute():path=ROOT/path
                text=path.read_text();compare(program['sha256'],sha(text.encode()))
                programs[program['path']]=dict(text=text,sha256=program['sha256'])
            executions.append(run)

    def load(name):
        report=read_json(work/('bc83-'+name+'.json'))
        assert report['passed'];compare(report['engineSha256'],digest)
        if 'ruleSha256' in report:compare(report['ruleSha256'],sha(RULE.read_bytes()))
        for path,h in report.get('harnessFiles',{}).items():compare(h,sha((ROOT/path).read_bytes()))
        reports[name]=report;return report

    baselines={}
    for label,filename,cases in [('shapes','shapes-source-v5.json',API),('production','production-source-v4.json',PRODUCTION)]:
        baseline=baselines[label]=read_json(work/filename);assert baseline['passed']
        compare([c['input'] for c in baseline['cases']],cases)
        report=load(label);compare([c['input'] for c in report['cases']],cases)
        for expected,case in zip(baseline['cases'],report['cases']):
            compare(case['expected'],expected['expected']);compare(case['actual'],expected['expected']);assert case['passed'];consume(case['native'])
    edits=list(mutations(RULE.read_text()))
    report=load('controls');compare([c['id'] for c in report['cases']],[name for name,_,_ in edits])
    for case,(_,label,changed) in zip(report['cases'],edits):
        source_case=next(c for c in baselines['shapes']['cases'] if c['input']['id']==label)
        compare(case['input'],source_case['input']);compare(case['expected'],source_case['expected']);compare(case['mutant'],changed)
        assert case['actual']!=case['expected']
        assert any(p['sha256']==sha(case['mutant'].encode()) for p in case['native']['programInputs'])
        consume(case['native'])
    report=load('updates');compare([c['id'] for c in report['cases']],STEPS);assert report['cases'][13]['changedPaths']==[]
    for case in report['cases']:
        assert case['passed'];wanted=next(c for c in baselines['production']['cases'] if c['input']['id']==case['id'])
        compare(case['expected'],wanted['expected']);compare(case['actual'],case['expected']);consume(case['native'])
    baseline=baselines['signing']=read_json(work/'signing-source.json');assert baseline['passed'];compare([c['input'] for c in baseline['cases']],SIGNING)
    signing_batches=0
    for part in range(4):
        report=load('signing-'+str(part));compare(report['sourceFiles'],baseline['sourceFiles'])
        wanted=list(range(part,len(SIGNING),4));compare([c['input'] for c in report['cases']],[SIGNING[i] for i in wanted])
        for index,case in zip(wanted,report['cases']):
            compare(case['expected'],baseline['cases'][index]['expected']);compare(case['actual'],case['expected'])
            consume(case['nativeCalls']);signing_batches+=len(case['signingCalls'])
    source=baselines['sharedRatchet']=read_json(work/'ratchet-original-source-current.json');assert source['exitCode']==0 and len(source['collected'])==3
    for mode in ['snapshot','workspace']:
        report=read_json(work/('bc83-ratchet-original-green-'+mode+'.json'));reports['sharedRatchet-'+mode]=report
        compare(report['engineSha256'],digest);assert report['exitCode']==0 and not report['semanticFallbacks']
        compare(report['executionMode'],mode)
        for key in ['sourcePin','sourceFiles','collected','cases','calls']:compare(report[key],source[key])
        compare(set(report['cases']),set(source['collected']))
        for phases in report['cases'].values():compare(phases,[dict(phase=p,outcome='passed') for p in ['setup','call','teardown']])
        for name,h in report['harnessFiles'].items():compare(h,sha((ROOT/name).read_bytes()))
        consume(report['native'])
    summary=dict(originalTestsPerAPI=174,sharedRatchetTestsPerAPI=3,positiveOriginalScannerObservationsPerAPI=35,
        ratchetCLICasesPerAPI=50,apiCases=len(API),productionCases=len(PRODUCTION),controls=8,persistentSteps=len(STEPS),signingCases=len(SIGNING),
        verifiedNativeExecutions=base['summary']['verifiedNativeExecutions']+base['boundaries']['summary']['verifiedNativeExecutions']+original['summary']['verifiedNativeExecutions']+len(executions),
        supplementalSigningBatches=signing_batches,failures=0,skippedSelectedAssertions=0,semanticFallbacks=0)
    return dict(format='rerail.impl-direct-complete-corpus-checkpoint/v1',fullMigrationAcceptance=False,passed=True,summary=summary,
        primary=base,baselines=baselines,reports=reports,programs=programs,validator=Path(__file__).read_text(),
        remaining='The global 100-rule migration remains unfinished. This receipt certifies the recorded direct-execution corpora and original assertions; it does not certify arbitrary Python runtime inputs.')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['work','base','engine','out']:p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args()
    with gzip.open(a.base,'rt') as stream:base=json.load(stream)
    result=complete(a.work,base,a.engine.absolute())
    a.out.write_bytes(gzip.compress(json.dumps(result,sort_keys=True,separators=(',',':')).encode(),mtime=0))
    print(json.dumps(result['summary']))


if __name__=='__main__':main()
