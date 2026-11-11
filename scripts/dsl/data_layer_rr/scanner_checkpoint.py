"""Retain exact scanner evidence without claiming full rule migration acceptance."""
import argparse
from collections import Counter,defaultdict
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha
from validate_supplied_facts_checkpoint import require,equal

MODES=('snapshot','workspace')


def walk(value):
    if isinstance(value,dict):
        yield value
        for child in value.values():yield from walk(child)
    elif isinstance(value,list):
        for child in value:yield from walk(child)


def validate(value):
    require(value['format']=='rerail.scanner-checkpoint/v1' and value['admitted'] is False and value['fullMigrationAcceptance'] is False and value['remaining'],'Invalid scanner progress scope')
    source=value['source'];original=value['originalSource'];check=value['check']
    require(source['passed'] and source['mode']=='source' and source['sourcePin']==PIN,'Invalid scanner oracle')
    require(original['commit']==PIN and original['pytest_exit_code']==0 and not original['source_mismatches'],'Invalid original oracle')
    oracle={r['input']['id']:r for r in source['cases']};require(len(oracle)==source['totalCases'] and oracle,'Missing source scanner identities')
    inventory=value['inventory'];require(inventory['sourcePin']==PIN,'Wrong independent inventory')
    files={p for p,r in inventory['files'].items() if check in r['detectors']}
    wanted={p for p in inventory['cases'] if p.split('::',1)[0] in files}
    require(set(original['test_outcomes'])==wanted and wanted,'Missing original source cases')
    original_calls=defaultdict(list)
    for event in original['events']:
        if event['check']==check:original_calls[event['test'],event['entry']].append(event)
    engine=value['engineSource']['cliArtifact']['sha256'];executions=0
    for mode in MODES:
        report=value['originals'][mode]
        require(report['mode']==mode and report['engineSha256']==engine and report['exitCode']==0 and report['oracle'] is False,'Original engine or API failed')
        require(not report['semanticFallbacks'] and not report['missingResults'] and not report.get('retainedCaseIds') and not report['casesWithoutNativeCalls'] and not report['hostProtocolCalls'],'Original scanner assertions omitted or delegated')
        require(set(report['cases'])==wanted and set(report['collected'])==wanted,'Original scanner cases omitted')
        for phases in report['cases'].values():require(len(phases)==3 and {p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Original scanner assertion failed')
        require(all(report['sourceFiles'].get(p)==inventory['files'][p]['sha256'] for p in files),'Changed original tests')
        require(report['policySha256'][check]==value['ruleSha256'],'Changed original policy')
        require(report['originalRecordingSha256']==value['originalSourceSha256'],'Wrong original recording')
        seen=Counter()
        for row in report['detectorObservations']:
            key=(row['test'],row['entry']);index=seen[key];seen[key]+=1
            require(index<len(original_calls[key]),'Extra original observation')
            expected=original_calls[key][index]
            require(row['status']=='passed' and equal(row['actualSourceResult'],expected['sourceResult']),'Original complete result differs')
        require(seen==Counter({key:len(events) for key,events in original_calls.items()}),'Original observations omitted or repeated')
        reports=value['scanners'][mode];rows=[c for r in reports for c in r['cases']]
        require(Counter(c['input']['id'] for c in rows)==Counter(oracle.keys()),'Scanner cases omitted or repeated')
        for report in reports:
            require(report['executionMode']==mode and report['engineSha256']==engine and report['oracle'] is False and report['passed'] and not report['semanticFallbacks'],'Scanner engine or API failed')
            require(report['sourceSha256']==source['sourceSha256'] and report['baselineSha256']==value['sourceSha256'],'Wrong scanner oracle')
        for row in rows:
            expected=oracle[row['input']['id']]
            require(row['passed'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']) and row['native'],'Scanner complete result differs')
    for name in ['source','originalSource']:
        require(sha(value[name+'Text'].encode())==value[name+'Sha256'] and equal(json.loads(value[name+'Text']),value[name]),'Changed oracle snapshot')
    for section in ['programs','harness']:
        for item in value[section].values():require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Changed '+section+' snapshot')
    for group in ['originals','scanners']:
        for row in walk(value[group]):
            for name,digest in row.get('harnessFiles',{}).items():require(value['harness'][name]['sha256']==digest,'Mixed scanner harness')
            if 'programInputs' not in row:continue
            executions+=1
            require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed scanner policies')
            for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Changed consumed program')
    positives=sum(bool(e['findings']) for events in original_calls.values() for e in events);require(positives>0,'Missing original positives')
    return dict(check=check,originalCasesPerAPI=len(wanted),originalPositiveObservationsPerAPI=positives,scannerCasesPerAPI=len(oracle),nativeExecutions=executions,admitted=False,fullMigrationAcceptance=False)


def pack(work,check):
    manifest=read_json(ROOT/MANIFEST);rule=next(r for r in manifest['rules'] if r['check']==check)
    value=dict(format='rerail.scanner-checkpoint/v1',check=check,admitted=False,fullMigrationAcceptance=False,
        rulePath=rule['rulePath'],ruleSha256=rule['ruleSha256'],engineSource=manifest['engineSource'],
        inventory=read_json(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json'),
        originals={m:read_json(work/f'original-accepted-{m}.json') for m in MODES},
        scanners={m:[read_json(p) for p in sorted(work.glob(f'scanner-accepted-{m}-*.json'))] for m in MODES},
        programs={},harness={},remaining=['Native collector helpers and registration','Supplied-fact reporting and signature contracts','Production discovery and persistent updates','CLI and signing administration','Behavior-removal controls and full admission'])
    for name,file in [('source','scanner-source.json'),('originalSource','source.json')]:
        text=(work/file).read_text();value[name]=json.loads(text);value[name+'Text']=text;value[name+'Sha256']=sha(text.encode())
    for group in ['originals','scanners']:
        for row in walk(value[group]):
            for p in row.get('programInputs',[]):
                text=(ROOT/p['path']).read_text() if p['sha256'] else None
                require((sha(text.encode()) if text is not None else None)==p['sha256'],'Program changed after execution')
                value['programs'][p['path']]=dict(text=text,sha256=p['sha256'])
            for p,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/p).read_text();require(sha(text.encode())==digest,'Harness changed after execution')
                value['harness'][p]=dict(text=text,sha256=digest)
    text=Path(__file__).read_text();value['harness'][Path(__file__).relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','validate']);parser.add_argument('path',type=Path);parser.add_argument('--work',type=Path);parser.add_argument('--check')
    args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.work,args.check);args.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
