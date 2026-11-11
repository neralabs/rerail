"""Retain scanner and production parity without claiming complete ForwardRef migration."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha
from validate_supplied_facts_checkpoint import require,equal

CHECK='forwardref'
MODES=('snapshot','workspace')
LINTER='scripts/tests/test_lint_forwardref.py'
WORKFLOW='scripts/tests/test_monitor_pr_status.py'


def walk(value):
    if isinstance(value,dict):
        yield value
        for child in value.values():yield from walk(child)
    elif isinstance(value,list):
        for child in value:yield from walk(child)


def validate(value):
    require(value['format']=='rerail.forwardref-progress/v1' and not value['admitted'] and not value['fullMigrationAcceptance'] and value['remaining'],'Invalid progress scope')
    engine=value['engineSource']['cliArtifact']['sha256'];executions=0
    for name,item in value['sources'].items():require(sha(item['text'].encode())==item['sha256'] and equal(json.loads(item['text']),item['value']),'Changed '+name+' oracle')
    original=value['sources']['original']['value'];require(original['commit']==PIN and original['pytest_exit_code']==0 and not original['source_mismatches'],'Invalid unchanged source suite')
    inventory=value['inventory'];files={name for name,row in inventory['files'].items() if CHECK in row['detectors']}
    require(files=={LINTER,WORKFLOW},'Changed independent original scope')
    wanted={name for name in inventory['cases'] if name.split('::')[0] in files};native_cases={name for name in wanted if name.split('::')[0]==LINTER};host_cases=wanted-native_cases
    require(len(wanted)==53 and len(native_cases)==5 and len(host_cases)==48 and set(original['test_outcomes'])==wanted,'Missing original identities')
    expected=[e for e in original['events'] if e['check']==CHECK];require(len(expected)==5 and sum(bool(e['findings']) for e in expected)==2,'Missing original observations or positives')
    for mode in MODES:
        report=value['originals'][mode]
        require(report['exitCode']==0 and report['mode']==mode and report['engineSha256']==engine and report['oracle'] is False,'Failed original API or engine')
        require(not report['semanticFallbacks'] and not report['missingResults'] and set(report['cases'])==wanted and set(report['collected'])==wanted,'Missing original assertions')
        require(set(report['casesWithoutNativeCalls'])==host_cases,'Unexpected original host-only coverage')
        require(report['originalRecordingSha256']==value['sources']['original']['sha256'] and all(report['sourceFiles'][f]==inventory['files'][f]['sha256'] for f in files),'Wrong original recording or changed tests')
        for phases in report['cases'].values():require(len(phases)==3 and {p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Failed original test phase')
        rows=report['detectorObservations'];require(len(rows)==len(expected),'Omitted native observation')
        oracle={(r['test'],r['entry']):r for r in expected}
        require(Counter((r['test'],r['entry']) for r in rows)==Counter(oracle.keys()),'Changed native observation identities')
        for row in rows:require(row['status']=='passed' and equal(row['actualSourceResult'],oracle[row['test'],row['entry']]['sourceResult']),'Different original full return')
    no_native={}
    for group,count in [('public',80),('production',18)]:
        item=value['sources'][group];source=item['value'];oracle={r['input']['id']:r for r in source['cases']}
        require(source['passed'] and source['mode']=='source' and source['sourcePin']==PIN and len(oracle)==count and source['totalCases']==count,'Missing '+group+' source cases')
        for mode in MODES:
            reports=value['reports'][group][mode];rows=[r for report in reports for r in report['cases']]
            require(Counter(r['input']['id'] for r in rows)==Counter(oracle.keys()),'Missing or repeated '+group+' case')
            for report in reports:
                require(report['passed'] and report['engineSha256']==engine and report['executionMode']==mode and report['oracle'] is False and not report['semanticFallbacks'],'Failed '+group+' replay')
                require(report['baselineSha256']==item['sha256'] and report['sourceSha256']==source['sourceSha256'],'Wrong '+group+' recording')
            no_native[group,mode]=[]
            for row in rows:
                expected=oracle[row['input']['id']];require(row['passed'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Different '+group+' complete return')
                if not row['native']:no_native[group,mode].append(row['input']['id'])
    for group in ['originals','reports']:
        for row in walk(value[group]):
            for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed native harness')
            if 'programInputs' not in row:continue
            executions+=1
            require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed native main rules')
            for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Changed consumed program')
    for group in ['programs','harness']:
        for item in value[group].values():require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Changed '+group+' snapshot')
    return dict(admitted=False,fullMigrationAcceptance=False,originalCasesPerAPI=53,nativeLinterOriginalCasesPerAPI=5,originalPositiveObservationsPerAPI=2,
        hostWorkflowCasesPerAPI=48,publicCasesPerAPI=80,productionCasesPerAPI=18,nativeExecutions=executions,
        casesWithoutNativeCalls={group:{mode:no_native[group,mode] for mode in MODES} for group in ['public','production']})


def pack(work):
    manifest=read_json(ROOT/MANIFEST);rule=next(r for r in manifest['rules'] if r['check']==CHECK)
    value=dict(format='rerail.forwardref-progress/v1',admitted=False,fullMigrationAcceptance=False,engineSource=manifest['engineSource'],rulePath=rule['rulePath'],ruleSha256=rule['ruleSha256'],
        inventory=read_json(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json'),sources={},programs={},harness={},
        originals={m:read_json(work/f'green-original-{m}.json') for m in MODES},reports={
            'public':{m:[read_json(work/f'public-green-{m}-{i}.json') for i in range(4)] for m in MODES},
            'production':{m:[read_json(work/f'green-production-{m}.json')] for m in MODES}},
        remaining=['Native explicit-file preparation for the three identified host-only I/O cases','Native collector, registry, helpers, configuration and supplied-fact contracts','Native CLI and its source-recorded assertions','Persistent updates, behavior-removal controls and complete admission'],
        hostWorkflowScope=dict(file=WORKFLOW,reason='The PR-monitor tests mock check_forwardref_handling. They produce no linter observations and remain host workflow assertions.'))
    for name,file in [('original','source.json'),('public','public-source-expanded.json'),('production','production-source.json')]:
        text=(work/file).read_text();value['sources'][name]=dict(text=text,sha256=sha(text.encode()),value=json.loads(text))
    for group in ['originals','reports']:
        for row in walk(value[group]):
            for p in row.get('programInputs',[]):
                text=(ROOT/p['path']).read_text() if p['sha256'] else None;require((sha(text.encode()) if text is not None else None)==p['sha256'],'Program changed after execution')
                value['programs'][p['path']]=dict(text=text,sha256=p['sha256'])
            for p,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/p).read_text();require(sha(text.encode())==digest,'Harness changed after execution');value['harness'][p]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','validate']);parser.add_argument('path',type=Path);parser.add_argument('--work',type=Path);args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.work);args.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
