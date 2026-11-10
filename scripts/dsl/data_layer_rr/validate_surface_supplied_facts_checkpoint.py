"""Validate the full recorded Surface reporter and model-validation checkpoint."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from validate_supplied_facts_checkpoint import require,digest,equal,PIN


def validate(value):
    require(value['format']=='rerail.surface-supplied-facts-checkpoint/v1','Wrong checkpoint format')
    require(value['fullMigrationAcceptance'] is False and value['remaining'],'Missing migration limits')
    source=value['source'];require(digest(source['text'].encode())==source['sha256'],'Source hash differs')
    baseline=json.loads(source['text']);require(baseline['sourcePin']==PIN and baseline['passed'],'Invalid source recording')
    expected={r['input']['id']:r for r in baseline['cases']}
    require(len(expected)==197 and sum('error' in r['expected'] for r in expected.values())==124,'Wrong source coverage')
    seen=[];engine=value['engineSource']['cliArtifact']['sha256']
    def programs(execution,mutation=None):
        for item in execution['programInputs']:
            name=item['path']
            if name.startswith('.rerail/surface-fact-control-'):
                require(mutation is not None and digest(mutation.encode())==item['sha256'],'Control program differs')
            else:require(value['programs'][name]['sha256']==item['sha256'],'Consumed program differs')
    for report in [*value['reports'],value['controls']]:
        require(report['sourcePin']==PIN and report['passed'],'Incomplete native report')
        require(report['engineSha256']==engine and report['baselineSha256']==source['sha256'],'Native cohort differs')
        for name,sha in report['harnessFiles'].items():require(value['harness'][name]['sha256']==sha,'Consumed harness differs')
        for row in report['cases']:
            identity=row['input']['id'];require(identity in expected,'Unrecorded input')
            require(equal(row['input'],expected[identity]['input']) and equal(row['expected'],expected[identity]['expected']),'Source observation differs')
            require(row['passed'] and 'nativeFailure' not in row,'Native execution failed')
            if report is value['controls']:
                require(not equal(row['actual'],row['expected']),'Disabled behavior went undetected')
                if identity=='surface-model:4':
                    wanted=dict(findings=[dict(f,severity='disabled') for f in row['expected']['findings']])
                    require(equal(row['actual'],wanted),'Severity control failed for an unrelated reason')
                else:require(identity=='surface-model:2' and row['actual']==dict(findings=[]),'Validation control failed for an unrelated reason')
                programs(row['execution'],row['mutatedPolicy'])
            else:
                seen.append(identity);require(equal(row['actual'],row['expected']),'Complete native result differs')
                programs(row['execution'])
                diagnostics=row['execution']['diagnostics']
                errors=[r['parameters'] for r in diagnostics if r['code']=='SF_ERROR']
                require(len(errors)==int('error' in row['expected']),'Native error decision differs')
                if errors:require(equal(json.loads(errors[0]['error_json']),row['actual']['error']),'Native error DTO differs')
                else:require(sum(r['code']=='SF_REPORT' for r in diagnostics)==len(row['actual']['findings']),'Native finding multiplicity differs')
    require(Counter(seen)==Counter(expected.keys()),'Source cases omitted or repeated')
    require(Counter(r['input']['id'] for r in value['controls']['cases'])==Counter(['surface-model:2','surface-model:4']),'Controls omitted or repeated')
    original=0
    original_source=value['originalSource']
    require(digest(original_source['text'].encode())==original_source['sha256'],'Original recording hash differs')
    original_baseline=json.loads(original_source['text']);require(original_baseline['commit']==PIN,'Wrong original source pin')
    for mode,report in value['originalSuites'].items():
        require(mode in {'snapshot','workspace'} and report['mode']==mode,'Original API differs')
        require(report['exitCode']==0 and not report['semanticFallbacks'] and not report['missingResults'],'Original assertions failed or delegated')
        require(report['engineSha256']==engine,'Original engine differs')
        require(report['originalRecordingSha256']==original_source['sha256'],'Original source recording differs')
        require(len(report['collected'])==9,'Original module coverage differs')
        require(set(report['collected'])==set(report['cases']),'Original cases omitted')
        for phases in report['cases'].values():require({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Original assertion skipped')
        require(any(c['entry']=='_report_facts' and c['findings']>0 for c in report['calls']),'Original reporter positive did not run natively')
        for call in report['calls']:
            require(call['status']=='passed','Original native call failed')
            for item in call['programInputs']:require(value['programs'][item['path']]['sha256']==item['sha256'],'Original consumed program differs')
        wanted=Counter((r['test'],r['entry']) for r in original_baseline['events'] if r['check']=='serverplan_grammar_surface' and r['test'] in report['cases'])
        actual=Counter((r['test'],r['entry']) for r in report['detectorObservations'])
        require(wanted==actual,'Original observations omitted or repeated')
        for row in report['detectorObservations']:
            require(row['status']=='passed' and equal(row['expected'],row['actual']),'Original complete result differs')
            if 'expectedSourceResult' in row:require(equal(row['expectedSourceResult'],row['actualSourceResult']),'Original source result differs')
        original+=len(report['cases'])
    require(set(value['originalSuites'])=={'snapshot','workspace'},'Original APIs omitted')
    production_source=value['productionSource'];require(digest(production_source['text'].encode())==production_source['sha256'],'Production source hash differs')
    production_baseline=json.loads(production_source['text'])
    require(production_baseline['sourcePin']==PIN and production_baseline['passed'],'Invalid production source recording')
    production_expected={r['input']['id']:r for r in production_baseline['cases'] if r['input']['check']=='serverplan_grammar_surface'}
    production_seen=[]
    for report in value['productionReports']:
        require(report['passed'] and report['engineSha256']==engine,'Invalid production execution')
        require(report['baselineSha256']==production_source['sha256'],'Production recording differs')
        for row in report['cases']:
            identity=row['input']['id'];require(identity in production_expected,'Unexpected production input')
            production_seen.append(identity);wanted=production_expected[identity]
            require(row['passed'] and equal(row['input'],wanted['input']) and equal(row['expected'],wanted['expected']) and equal(row['actual'],wanted['expected']),'Complete production result differs')
            for execution in row['native'].values():programs(execution)
    require(len(production_expected)==20 and Counter(production_seen)==Counter(production_expected.keys()),'Production cases omitted or repeated')
    updates=value['updates'];require(updates['passed'] and updates['engineSha256']==engine and len(updates['cases'])==14,'Incomplete persistent updates')
    require(updates['baselineSha256']==production_source['sha256'],'Persistent source recording differs')
    require([r['id'] for r in updates['cases']]==['empty','roots','clean','roots','file-links','directory-links','ordering','duplicate-findings','syntax','clean','empty','roots','roots','empty'],'Persistent update ladder differs')
    for row in updates['cases']:
        require(row['check']=='serverplan_grammar_surface' and row['passed'] and equal(row['expected'],row['actual']),'Persistent result differs')
        require(equal(row['expected'],production_expected['serverplan_grammar_surface:'+row['id']]['expected']),'Persistent source result differs')
        require(len(row['native'])==2,'Persistent query omitted')
        for execution in row['native']:programs(execution)
    for section in ['programs','harness']:
        for item in value[section].values():require(digest(item['text'].encode())==item['sha256'],section+' snapshot differs')
    return dict(suppliedCases=len(seen),expectedValidationErrors=124,
        findingPositiveCases=sum(bool(r['expected'].get('findings')) for r in expected.values()),
        disabledBehaviorControls=2,originalAssertions=original,
        productionCases=20,persistentSteps=14,
        nativeExecutions=len(seen)+2+40+28+sum(len(r['calls']) for r in value['originalSuites'].values()),fullMigrationAcceptance=False)


def pack(work):
    from migration import ROOT,MANIFEST,read_json
    raw=(work/'surface-source-wire.json').read_text()
    original_raw=(ROOT/'.rerail/rr-batch1-final93/source.json').read_text()
    production_raw=(ROOT/'.rerail/rr-first-ten-production/source-locked.json').read_text()
    value=dict(format='rerail.surface-supplied-facts-checkpoint/v1',fullMigrationAcceptance=False,
        source=dict(text=raw,sha256=digest(raw.encode())),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        originalSource=dict(text=original_raw,sha256=digest(original_raw.encode())),
        productionSource=dict(text=production_raw,sha256=digest(production_raw.encode())),
        productionReports=[read_json(work/f'surface-production-{i}.json') for i in range(2)],
        updates=read_json(work/'surface-updates.json'),
        reports=[read_json(work/f'surface-wire-{i}.json') for i in range(20)],
        controls=read_json(work/'surface-controls.json'),
        originalSuites={mode:read_json(work/f'surface-original-{mode}.json') for mode in ['snapshot','workspace']},
        programs={},harness={},remaining=[
            'The complete first-ten production and persistent-update corpora need a refresh on the current cohort.',
            'First-ten admission and the remaining 90 rule migrations are unfinished.',
            'This receipt covers recorded contracts, not every possible Python runtime object.'])
    def program(item):
        name=item['path']
        if name.startswith('.rerail/surface-fact-control-'):return
        text=(ROOT/name).read_text();require(digest(text.encode())==item['sha256'],'Program changed during acceptance')
        value['programs'][name]=dict(text=text,sha256=item['sha256'])
    for report in [*value['reports'],value['controls'],*value['originalSuites'].values(),*value['productionReports'],value['updates']]:
        for name,sha in report['harnessFiles'].items():
            text=(ROOT/name).read_text();require(digest(text.encode())==sha,'Harness changed during acceptance')
            value['harness'][name]=dict(text=text,sha256=sha)
        for row in report.get('cases',[]) if isinstance(report.get('cases'),list) else []:
            executions=([row['execution']] if 'execution' in row else
                list(row['native'].values()) if isinstance(row.get('native'),dict) else row['native'])
            for execution in executions:
                for item in execution['programInputs']:program(item)
        for call in report.get('calls',[]):
            for item in call['programInputs']:program(item)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=digest(text.encode()))
    value['summary']=validate(value);return value


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','validate'])
    parser.add_argument('path',type=Path);parser.add_argument('--out',type=Path);args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.path);args.out.write_bytes(gzip.compress((json.dumps(value,ensure_ascii=False,indent=2)+'\n').encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))


if __name__=='__main__':main()
