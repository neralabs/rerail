"""Audit complete projection observations, controls and native original-suite calls."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from validate_supplied_facts_checkpoint import require,equal,digest,PIN


def validate(value):
    require(value['format']=='rerail.model-projection-checkpoint/v1' and not value['fullMigrationAcceptance'],'Wrong checkpoint scope')
    source=value['source'];require(digest(source['text'].encode())==source['sha256'],'Source hash differs')
    baseline=json.loads(source['text']);require(baseline['passed'] and baseline['sourcePin']==PIN,'Invalid source recording')
    expected={r['input']['id']:r for r in baseline['cases']};require(len(expected)==206,'Wrong source case count')
    engine=value['engineSource']['cliArtifact']['sha256'];seen=[];native=0
    def programs(execution,mutation=None):
        for item in execution['programInputs']:
            if item['path'].startswith('.rerail/projection-control-'):
                require(mutation is not None and digest(mutation.encode())==item['sha256'],'Control policy differs')
            else:require(value['programs'][item['path']]['sha256']==item['sha256'],'Consumed program differs')
    for report in [*value['reports'],value['controls']]:
        require(report['passed'] and report['sourcePin']==PIN and report['engineSha256']==engine,'Invalid native cohort')
        require(report['baselineSha256']==source['sha256'],'Native baseline differs')
        for name,sha in report['harnessFiles'].items():require(value['harness'][name]['sha256']==sha,'Native harness differs')
        for row in report['cases']:
            identity=row['input']['id'];require(identity in expected,'Unrecorded source case')
            require(equal(row['input'],expected[identity]['input']) and equal(row['expected'],expected[identity]['expected']),'Source observation differs')
            require(row['passed'],'Native case failed')
            if report is value['controls']:
                if row['name'] in {'integer','boolean'}:
                    require(row['actual']==dict(protocolTypeError=dict(expected='int' if row['name']=='integer' else 'bool',actual='str')),'Coercion control failed for an unrelated reason')
                elif row['name']=='validation':
                    require('error' in row['expected'] and row['actual'].get('messages')==[] and row['actual']['model']['values']['files']=={'tuple':[]},'Validation control failed for an unrelated reason')
                else:
                    require(not equal(row['actual'],row['expected']) and row['actual'].get('model',{}).get('type')=='ProjectServerPlanFacts','File-order control failed for an unrelated reason')
            else:
                seen.append(identity);require(equal(row['actual'],row['expected']),'Complete model or message result differs')
                require(len(row['executions'])==(1 if 'error' in row['expected'] else 2),'Projection or formatting execution omitted')
            for execution in row['executions']:
                programs(execution,row.get('mutatedPolicy'));native+=1
            if report is not value['controls']:
                errors=[r for r in row['executions'][0]['diagnostics'] if r['code']=='SF_ERROR']
                require(len(errors)==int('error' in row['expected']),'Native validation decision differs')
                if not errors:
                    markers=[r['parameters'] for r in row['executions'][0]['diagnostics'] if r['code']=='SF_MODEL']
                    require(len(markers)==len(row['actual']['model']['values']['files']['tuple']),'Native model multiplicity differs')
                    messages=[r for r in row['executions'][1]['diagnostics'] if r['code']=='SPG_MESSAGE']
                    require(len(messages)==len(row['actual']['messages']),'Native message multiplicity differs')
    require(Counter(seen)==Counter(expected.keys()),'Projection cases omitted or repeated')
    require(Counter(r['name'] for r in value['controls']['cases'])==Counter(['integer','boolean','file-order','validation']),'Controls omitted or repeated')
    for mode,report in value['originalSuites'].items():
        require(report['mode']==mode and report['exitCode']==0 and report['engineSha256']==engine,'Invalid original suite')
        require(not report['semanticFallbacks'] and not report['missingResults'],'Original semantic fallback or missing result')
        require(len(report['collected'])==9 and set(report['collected'])==set(report['cases']),'Original cases omitted')
        for phases in report['cases'].values():require({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Original assertion skipped or failed')
        require(any(c['entry']=='_project_facts' for c in report['calls']),'Original CLI did not execute native projection')
        require(any(c['entry']=='_project_messages' for c in report['calls']),'Original CLI did not execute native path joining')
        for call in report['calls']:
            require(call['status']=='passed','Original native call failed');native+=1
            for item in call['programInputs']:require(value['programs'][item['path']]['sha256']==item['sha256'],'Original program differs')
        for observation in report['detectorObservations']:
            require(observation['status']=='passed' and equal(observation['expected'],observation['actual']),'Original complete observation differs')
            if 'expectedSourceResult' in observation:require(equal(observation['expectedSourceResult'],observation['actualSourceResult']),'Original API result differs')
    require(set(value['originalSuites'])=={'snapshot','workspace'},'Original APIs omitted')
    cli=value['cli'];require(cli['passed'] and cli['engineSha256']==engine and len(cli['cases'])==33,'Incomplete CLI corpus')
    for row in cli['cases']:
        require(row['passed'] and equal(row['expected'],row['actual']),'CLI streams, status or callbacks differ')
        for execution in row['native']:programs(execution);native+=1
    for section in ['programs','harness']:
        for item in value[section].values():require(digest(item['text'].encode())==item['sha256'],section+' snapshot differs')
    require(value['remaining'],'Missing global migration limits')
    return dict(projectionCases=len(seen),expectedErrors=sum('error' in r['expected'] for r in expected.values()),
        nonemptyProjections=sum(bool(r['expected'].get('model',{}).get('values',{}).get('files',{}).get('tuple')) for r in expected.values()),
        originalAssertions=18,cliCases=33,controls=4,nativeExecutions=native,fullMigrationAcceptance=False)


def pack(work):
    from migration import ROOT,MANIFEST,read_json
    raw=(work/'source-final.json').read_text()
    value=dict(format='rerail.model-projection-checkpoint/v1',fullMigrationAcceptance=False,
        source=dict(text=raw,sha256=digest(raw.encode())),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        reports=[read_json(work/f'accepted-{i}.json') for i in range(20)],controls=read_json(work/'controls-accepted.json'),
        cli=read_json(work/'cli-workspace.json'),
        originalSuites={mode:read_json(work/f'original-{mode}.json') for mode in ['snapshot','workspace']},
        programs={},harness={},remaining=['Complete first-ten acceptance refresh and per-rule admission.',
            'The remaining rule migrations and five absent translations are unfinished.'])
    def program(item):
        name=item['path']
        if name.startswith('.rerail/projection-control-'):return
        text=(ROOT/name).read_text();require(digest(text.encode())==item['sha256'],'Program changed during execution')
        value['programs'][name]=dict(text=text,sha256=item['sha256'])
    for report in [*value['reports'],value['controls'],*value['originalSuites'].values(),value['cli']]:
        for name,sha in report['harnessFiles'].items():
            text=(ROOT/name).read_text();require(digest(text.encode())==sha,'Harness changed during execution')
            value['harness'][name]=dict(text=text,sha256=sha)
        if isinstance(report.get('cases'),list):
            for row in report['cases']:
                for execution in row.get('executions',row.get('native',[])):
                    for item in execution['programInputs']:program(item)
        for call in report.get('calls',[]):
            for item in call['programInputs']:program(item)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=digest(text.encode()))
    value['summary']=validate(value);return value


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate']);p.add_argument('path',type=Path);p.add_argument('--out',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.path);a.out.write_bytes(gzip.compress((json.dumps(value,ensure_ascii=False,indent=2)+'\n').encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))


if __name__=='__main__':main()
