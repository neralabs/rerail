"""Admit admin only after exact original assertions and all native contract groups pass."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from admin_controls import EDITS,SOURCES,mutate,library_path,invalid_result
from admin_updates import SEQUENCES
from assertion_shards import merge
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,compare,require_complete
from validate_supplied_facts_checkpoint import require,equal

CHECK='admin_async_purity'
FORMAT='rerail.admin-admission/v1'
MODES=('snapshot','workspace')
COUNTS=dict(components=128,public=38,cli=31,production=22,legacy=59)
PARTS=dict(components=16,public=8,cli=31,production=8,legacy=16)
CONFIG_NAMES={'LINTER_NAME','SCAN_DIRS','ALLOWLISTED_FILES','ALLOWLIST_SIGNATURE','FORBIDDEN_PATTERNS','SKIP_PATTERNS','ADMIN_ASYNC_PURITY_EXTRACTOR'}


def snapshot(path):
    text=path.read_text();return dict(text=text,sha256=sha(text.encode()),value=json.loads(text))


def source_value(item):
    require(sha(item['text'].encode())==item['sha256'] and equal(json.loads(item['text']),item['value']),'Changed source recording bytes')
    return item['value']


def reports(value):
    for rows in value['originals'].values():yield from rows
    for group in value['groups'].values():
        for rows in group['reports'].values():yield from rows
    for rows in value['updates'].values():yield from rows
    yield from value['controls']


def calls(report):
    if 'calls' in report:yield from report['calls']
    else:
        for row in report['cases']:yield from row['native']


def validate(value):
    require(value['format']==FORMAT and value['check']==CHECK and value['admitted'] is True and value['fullRuleMigrationAcceptance'] is True and value['fullMigrationAcceptance'] is False,'Wrong admin admission scope')
    engine=value['engineSource']['cliArtifact']['sha256'];main=value['rulePath'];digest=value['ruleSha256']
    for section in ['programs','harness']:
        for item in value[section].values():require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Changed '+section+' snapshot')
    require(value['programs'][main]['sha256']==digest,'Changed main rule')
    original=source_value(value['originalSource']);inventory=source_value(value['inventory'])
    module_path='scripts/lint_admin_async_purity.py';source_sha=value['originals']['snapshot'][0]['sourceFiles'][module_path]
    require(original['commit']==PIN and type(original['pytest_exit_code']) is int and original['pytest_exit_code']==0 and not original['source_mismatches'] and inventory['sourcePin']==PIN,'Invalid pinned original source')
    files={p for p,r in inventory['files'].items() if CHECK in r['detectors']};wanted={p for p in inventory['cases'] if p.split('::')[0] in files}
    require(len(wanted)==125 and set(original['test_outcomes'])==wanted,'Missing unchanged source assertion')
    owned=[r for r in original['events'] if r['check']==CHECK];require(len(owned)==51 and sum(bool(r.get('findings')) for r in owned)==26,'Missing original positive observations')
    source_observations={};seen=Counter()
    for row in owned:
        key=(row['test'],row['entry']);source_observations[(*key,seen[key])]=row;seen[key]+=1
    for mode in MODES:
        shards=value['originals'][mode];require(Counter(r['part'] for r in shards)==Counter(range(32)),'Missing original shard')
        cases=[];observed=[]
        for report in shards:
            require(report['mode']==mode and report['sourcePin']==PIN and report['engineSha256']==engine and report['oracle'] is False and type(report['exitCode']) is int and report['exitCode']==0,'Invalid original native execution')
            require(not report['semanticFallbacks'] and not report['missingResults'] and not report.get('retainedCaseIds') and not report['hostProtocolCalls'],'Original assertion omitted or delegated')
            require(report['originalRecordingSha256']==value['originalSource']['sha256'] and report['policySha256'][CHECK]==digest,'Mixed original source or rule')
            require(set(report['collected'])==wanted and report['parts']==32 and set(report['cases'])==set(report['collected'][report['part']::32]),'Incomplete original assertion shard')
            for identity,phases in report['cases'].items():
                require(len(phases)==3 and {p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Original assertion failed or skipped');cases.append(identity)
            require(all(report['sourceFiles'][p]==inventory['files'][p]['sha256'] for p in files),'Changed original assertions')
            require(report['sourceFiles'][module_path]==source_sha,'Mixed original source module')
            binding=report['configurationBindings'].get(CHECK,{})
            require(set(binding.get('names',[]))==CONFIG_NAMES and type(binding.get('nativeCall')) is int,'Original constants not natively bound')
            index=binding['nativeCall'];require(0<=index<len(report['calls']),'Missing native configuration call')
            call=report['calls'][index];require(call['entry']=='_admin_configuration' and call['status']=='passed' and call['diagnostics'] and all(r['code']=='ADMIN_CONFIG' for r in call['diagnostics']),'Invalid native configuration binding')
            for row in report['detectorObservations']:
                key=(row['test'],row['entry'],row['ordinal']);require(key in source_observations,'Unexpected original observation');source=source_observations[key];observed.append(key)
                require(equal(row['expectedSourceResult'],source['sourceResult']) and equal(row['actualSourceResult'],source['sourceResult']),'Changed complete original DTO')
                if row['status']!='passed':require(row['status']=='source-result-passed; legacy normalized DTO unavailable' and bool(source.get('retained_reason')),'Unverified original observation')
            require(all(c['status']=='passed' for c in report['calls']),'Failed original native call')
        require(Counter(cases)==Counter(wanted) and Counter(observed)==Counter(source_observations.keys()),'Missing or repeated original case or complete value')
    for group,count in COUNTS.items():
        item=value['groups'][group];oracle=source_value(item['source'])
        require(oracle['sourcePin']==PIN and oracle['mode']=='source' and oracle['passed'] is True and oracle['totalCases']==count,'Invalid component source')
        require(oracle['sourceSha256']==source_sha,'Mixed source module across contracts')
        expected={r['input']['id']:r for r in oracle['cases']};require(len(expected)==count,'Missing source contract identity')
        for mode in MODES:
            shards=item['reports'][mode];require(Counter(r['part'] for r in shards)==Counter(range(PARTS[group])),'Missing native contract shard')
            rows=[r for report in shards for r in report['cases']];require(Counter(r['input']['id'] for r in rows)==Counter(expected.keys()),'Missing or repeated native contract case')
            for report in shards:
                require(report['passed'] is True and report['executionMode']==mode and report['engineSha256']==engine and report['oracle'] is False and not report['semanticFallbacks'],'Failed or delegated native contract')
                require(report['sourceSha256']==oracle['sourceSha256'] and report['baselineSha256']==item['source']['sha256'],'Mixed contract source cohort')
            for row in rows:
                source=expected[row['input']['id']]
                require(row['passed'] is True and row['native'] and equal(row['input'],source['input']) and equal(row['expected'],source['expected']) and equal(row['actual'],source['expected']),'Changed complete contract value')
    legacy=value['groups']['legacy']['source']['value']['cases']
    require(Counter(r['input']['corpus'] for r in legacy)=={'batch-02-recording.json':51,'batch-02-probes.json':8},'Missing historical input family')
    for row in legacy:require(row['input']['corpusSha256']==value['legacyCorpora'][row['input']['corpus']]['sha256'],'Changed historical input bytes')
    production=value['groups']['production']['source'];oracle={r['input']['id']:r for r in production['value']['cases']}
    for mode in MODES:
        shards=value['updates'][mode];require(Counter(r['sequence'] for r in shards)==Counter(range(len(SEQUENCES))),'Missing retained update sequence')
        for report in shards:
            require(report['passed'] is True and report['complete'] is True and report['mode']==mode and report['sourcePin']==PIN and report['engineSha256']==engine and report['baselineSha256']==production['sha256'],'Invalid retained update evidence')
            sequence=SEQUENCES[report['sequence']];require([r['id'] for r in report['cases']]==sequence and [r['step'] for r in report['cases']]==list(range(len(sequence))),'Missing retained update step')
            previous={}
            for row in report['cases']:
                source=oracle[row['id']];current={f['path']:f for f in source['input']['files']};changed=sorted(p for p in previous.keys()|current.keys() if previous.get(p)!=current.get(p))
                require(row['changedPaths']==changed and row['passed'] is True and row['native'] and equal(row['expected'],source['expected']) and equal(row['actual'],source['expected']),'Changed complete retained update');previous=current
    controls=[row for report in value['controls'] for row in report['cases']]
    require(Counter((r['control'],r['mode']) for r in controls)==Counter((n,m) for n in EDITS for m in MODES),'Missing removal control')
    source_hashes={g:value['groups'][g]['source']['sha256'] for g in SOURCES}
    for report in value['controls']:require(report['passed'] is True and report['complete'] is True and report['engineSha256']==engine and report['sourcePin']==PIN and not report['semanticFallbacks'] and report['baselineSha256']==source_hashes,'Failed or mixed removal control')
    for row in controls:
        group,identity,area,kind,before,after=EDITS[row['control']];source=next(r for r in value['groups'][group]['source']['value']['cases'] if r['input']['id']==identity)
        require(row['group']==group and equal(row['input'],source['input']) and equal(row['expected'],source['expected']),'Changed removal witness')
        require(row['rejected'] is True and row['native'] and not equal(row['actual'],source['expected']) and not invalid_result(row['actual']),'Invalid or ineffective native mutation')
        text=value['programs'][main]['text'];extra=[]
        if area=='main':text=mutate(text,kind,before,after)
        else:
            extra.append(mutate(value['programs'][library_path(area)]['text'],kind,before,after));text=text.replace(f'#include "data-layer/admin-{area}.rr"','#include "control.rr"')
        require(row['programCopies']=={sha(t.encode()):t for t in [text,*extra]},'Unrelated native mutation')
    executions=0;utilities=0;control_ids={id(r) for r in value['controls']}
    for report in reports(value):
        for path,d in report['harnessFiles'].items():require(value['harness'][path]['sha256']==d,'Mixed consumed harness')
        for call in calls(report):
            if call.get('utility')=='sign-ed25519':
                require(call['requests'] and all(set(r)=={'id','key_env','message'} for r in call['requests']) and len(call['requests'])==len(call['replies']),'Invalid native signing utility evidence');utilities+=1;continue
            inputs=call.get('programInputs');require(isinstance(inputs,list) and inputs and isinstance(call.get('phases'),list),'Missing native consumed programs')
            if id(report) not in control_ids:require(any(p['path']==main and p['sha256']==digest for p in inputs),'Mixed main rule revision')
            for p in inputs:require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound native program')
            executions+=1
    return dict(check=CHECK,admitted=True,fullRuleMigrationAcceptance=True,fullMigrationAcceptance=False,originalCasesPerAPI=125,originalObservationsPerAPI=51,originalPositiveObservationsPerAPI=26,casesPerAPI=COUNTS,persistentStepsPerAPI=sum(map(len,SEQUENCES)),controls=len(controls),nativeRuleExecutions=executions,nativeUtilityExecutions=utilities,nativeExecutions=executions+utilities)


def pack(work):
    manifest=read_json(ROOT/MANIFEST);entry=next(r for r in manifest['rules'] if r['check']==CHECK)
    value=dict(format=FORMAT,check=CHECK,admitted=True,fullRuleMigrationAcceptance=True,fullMigrationAcceptance=False,rulePath=entry['rulePath'],ruleSha256=entry['ruleSha256'],engineSource=manifest['engineSource'],originalSource=snapshot(work/'source.json'),inventory=snapshot(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json'),originals={m:[read_json(work/f'original-final-{m}-{i}.json') for i in range(32)] for m in MODES},groups={},updates={m:[read_json(work/f'updates-first-{m}-{i}.json') for i in range(len(SEQUENCES))] for m in MODES},controls=[read_json(work/f'controls-first-{m}-{i}.json') for m in MODES for i in range(len(EDITS))],programs={},harness={},legacyCorpora={n:dict(sha256=sha((ROOT/'gate/data-layer/fixtures/host/data-layer-port'/n).read_bytes())) for n in ['batch-02-recording.json','batch-02-probes.json']})
    for group in COUNTS:
        phase='first' if group in ['production','legacy'] else 'final'
        value['groups'][group]=dict(source=snapshot(work/f'{group}-source.json'),reports={m:[read_json(work/f'{group}-{phase}-{m}-{i}.json') for i in range(PARTS[group])] for m in MODES})
    mutations={d:t for report in value['controls'] for row in report['cases'] for d,t in row['programCopies'].items()}
    for report in reports(value):
        for call in calls(report):
            for p in call.get('programInputs',[]):
                d=p['sha256'];text=mutations.get(d)
                if text is None and d is not None:text=(ROOT/p['path']).read_text()
                require((sha(text.encode()) if text is not None else None)==d,'Program changed after native execution');value['programs'][p['path']]=dict(text=text,sha256=d)
        for path,d in report['harnessFiles'].items():
            text=(ROOT/path).read_text();require(sha(text.encode())==d,'Harness changed after native execution');value['harness'][path]=dict(text=text,sha256=d)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


def admit(path,value):
    summary=validate(value);manifest=read_json(ROOT/MANIFEST);compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact']);entry=next(r for r in manifest['rules'] if r['check']==CHECK);compare(entry['ruleSha256'],value['ruleSha256'])
    for name,item in value['programs'].items():
        if not name.startswith('.rerail/') and item['sha256'] is not None:compare(sha((ROOT/name).read_bytes()),item['sha256'])
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port';refs=[]
    for mode in MODES:
        target=folder/f'rr-admin-complete-native-{mode}.json';write_json(target,merge(value['originals'][mode]));refs.append(dict(path=target.relative_to(ROOT).as_posix(),sha256=sha(target.read_bytes())))
    inventory=folder/'rr-original-test-inventory-v2.json';checkpoint=dict(path=path.absolute().relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes()))
    evidence=dict(assertions=125,positives=26,failures=0,skipped=0,semanticFallbacks=0,ruleSha256=entry['ruleSha256'],engineCommit=value['engineSource']['cliArtifact']['engineCommit'],engineSha256=value['engineSource']['cliArtifact']['sha256'],corpusSha256=value['originalSource']['sha256'],sourceCommit=PIN,originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),reports=refs,acceptanceCheckpoints=[checkpoint],supplemental=summary,scope='All 125 unchanged original assertions, all 51 complete API observations, full native components, public scanning, CLI/signing, production, historical inputs, retained edits and valid removal controls through both APIs.')
    if entry.get('evidence') and entry['evidence']!=evidence:entry.setdefault('evidenceHistory',[]).append(entry['evidence'])
    previous=entry.get('migrationProgress')
    if previous and not previous.get('admitted'):entry.setdefault('migrationProgressHistory',[]).append(previous)
    entry.update(status='full-contract-passed',evidence=evidence,migrationProgress=dict(status='full-contract-passed',admitted=True,verifiedNativeExecutions=summary['nativeExecutions'],checkpoint=checkpoint,summary=summary,remaining=[]))
    require_complete([entry]);write_json(ROOT/MANIFEST,manifest)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate','admit']);p.add_argument('path',type=Path);p.add_argument('--work',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.work);a.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as stream:value=json.load(stream)
    if a.command=='admit':admit(a.path,value)
    print(json.dumps(validate(value),sort_keys=True))
