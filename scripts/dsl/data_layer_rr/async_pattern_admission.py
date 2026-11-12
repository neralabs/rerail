"""Admit async patterns only with one complete, coherent native acceptance cohort."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
import shutil
import async_pattern_checkpoint
from async_pattern_controls import EDITS,SOURCES,mutate,library_path,invalid_result
from async_pattern_updates import SEQUENCES
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,compare,require_complete
from scanner_checkpoint import walk
from validate_supplied_facts_checkpoint import require,equal

CHECK='async_patterns'
FORMAT='rerail.async-pattern-admission/v1'
MODES=('snapshot','workspace')
COUNTS=dict(helpers=137,components=162,files=41,cli=40,legacy=38,io=14)
PARTS=dict(helpers=16,components=16,files=8,cli=40,legacy=8,io=4)
SOURCE_FILES=dict(SOURCES,legacy='legacy-source.json')
CONFIG_NAMES={'VIOLATIONS','SYNC_WRITE_METHODS','SYNC_READ_METHODS','SYNC_INTERNAL_IO_METHODS','SYNC_CONNECTION_METHODS','SYNC_POLYMORPHIC_METADATA_METHODS','ALL_SYNC_TRANSACTION_METHODS','SCAN_PATHS','EXCLUDE_PATTERNS','TRANSACTION_PARAM_NAMES','TRANSACTION_TYPE_NAMES','QUERY_SOURCE_METHODS','SOURCE_TRIGGERS','ASYNC_PATTERNS_EXTRACTOR'}


def validate(value):
    require(value['format']==FORMAT and value['admitted'] is True and value['fullRuleMigrationAcceptance'] is True and value['fullMigrationAcceptance'] is False,'Wrong full-rule admission scope')
    base=dict(value,format=async_pattern_checkpoint.FORMAT,admitted=False,remaining=['Additional admission requirements are checked below.'])
    summary=async_pattern_checkpoint.validate(base)
    engine=value['engineSource']['cliArtifact']['sha256'];source_hash=value['source']['sourceSha256']
    events=[r for r in value['originalSource']['events'] if r['check']==CHECK]
    require(len(events)==12 and sum(bool(r.get('findings')) for r in events)==8,'Missing original observations or positives')
    for mode in MODES:
        original=value['originals'][mode];binding=original['configurationBindings'].get(CHECK,{})
        require(set(binding.get('names',[]))==CONFIG_NAMES,'Missing original native configuration binding')
        call=original['calls'][binding['nativeCall']]
        require(call['check']==CHECK and call['entry']=='_ap_configuration' and call['status']=='passed' and call['diagnostics'] and all(r['code']=='AP_CONFIG' for r in call['diagnostics']),'Invalid original native configuration')
        require(original['sourceFiles']['scripts/lint_async_patterns.py']==source_hash,'Mixed original source module')
    executions=0
    for group,count in COUNTS.items():
        item=value['groups'][group];source=item['source'];oracle=source['value']
        require(sha(source['text'].encode())==source['sha256'] and equal(json.loads(source['text']),oracle),'Changed '+group+' source bytes')
        require(oracle['sourcePin']==PIN and oracle['mode']=='source' and oracle['passed'] and oracle['totalCases']==count and oracle['sourceSha256']==source_hash,'Invalid '+group+' source recording')
        wanted={r['input']['id']:r for r in oracle['cases']};require(len(wanted)==count,'Missing '+group+' source case')
        for mode in MODES:
            reports=item['reports'][mode];rows=[r for report in reports for r in report['cases']]
            require(Counter(r['input']['id'] for r in rows)==Counter(wanted.keys()),'Missing or repeated '+group+' native case')
            for report in reports:
                require(report['passed'] and report['executionMode']==mode and report['engineSha256']==engine and report['oracle'] is False and not report['semanticFallbacks'],'Failed '+group+' engine or API')
                require(report['sourceSha256']==source_hash and report['baselineSha256']==source['sha256'],'Mixed '+group+' source recording')
            for row in rows:
                expected=wanted[row['input']['id']]
                require(row['passed'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Changed '+group+' complete value')
                if not row['native']:
                    require(group=='components' and row['input']['id']=='extract-invalid' and row['input']['source']=='async def broken(:\n','Missing native execution')
                    parts=dict(row['expected']['value']['items'])
                    require(parts['facts']==dict(kind='dict',items=[]) and parts['errors']['items'] and parts['errors']['items'][0][1].startswith('SyntaxError:'),'Invalid framework parsing boundary')
    legacy=value['groups']['legacy']['source']['value']['cases']
    require(Counter(r['input']['corpus'] for r in legacy)=={'batch-02-recording.json':12,'batch-02-probes.json':26},'Missing historical input family')
    for row in legacy:require(row['input']['corpusSha256']==value['legacyCorpora'][row['input']['corpus']]['sha256'],'Changed historical corpus')
    production={r['input']['id']:r for r in value['source']['cases']}
    for mode in MODES:
        reports=value['updates'][mode]
        require(Counter(r['sequence'] for r in reports)==Counter(range(len(SEQUENCES))),'Missing retained update sequence')
        for report in reports:
            require(report['passed'] and report['complete'] and report['mode']==mode and report['sourcePin']==PIN and report['engineSha256']==engine and report['baselineSha256']==value['sourceSha256'],'Incomplete retained update report')
            sequence=SEQUENCES[report['sequence']]
            require([r['step'] for r in report['cases']]==list(range(len(sequence))) and [r['id'] for r in report['cases']]==sequence,'Missing retained update step')
            previous={}
            for row in report['cases']:
                expected=production[row['id']];current={f['path']:f for f in expected['input']['files']}
                changed=sorted(p for p in previous.keys()|current.keys() if previous.get(p)!=current.get(p))
                require(row['changedPaths']==changed,'Changed untouched update input')
                require(row['passed'] and row['native'] and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Changed complete retained update value')
                previous=current
    controls=[r for report in value['controls'] for r in report['cases']]
    require(Counter((r['control'],r['mode']) for r in controls)==Counter((n,m) for n in EDITS for m in MODES),'Missing behavior-removal control')
    oracles={g:{r['input']['id']:r for r in value['groups'][g]['source']['value']['cases']} for g in COUNTS}
    oracles['production']=production
    baseline_hashes={g:(value['sourceSha256'] if g=='production' else value['groups'][g]['source']['sha256']) for g in SOURCES}
    for report in value['controls']:
        require(report['passed'] and report['complete'] and report['engineSha256']==engine and report['sourcePin']==PIN and not report['semanticFallbacks'] and report['baselineSha256']==baseline_hashes,'Incomplete behavior-removal report')
    original_main=value['programs'][value['rulePath']]['text']
    for row in controls:
        group,identity,area,kind,before,after=EDITS[row['control']];expected=oracles[group][identity]
        require(row['group']==group and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']),'Changed control witness')
        require(row['rejected'] and row['native'] and not equal(row['actual'],expected['expected']),'Ineffective behavior-removal control')
        require(not invalid_result(row['actual']),'Invalid native mutant')
        main=original_main;extra=[]
        if area=='main':main=mutate(main,kind,before,after)
        else:
            extra.append(mutate(value['programs'][library_path(area)]['text'],kind,before,after))
            main=main.replace(f'#include "data-layer/async-pattern-{area}.rr"','#include "control.rr"')
        require(row['programCopies']=={sha(t.encode()):t for t in [main,*extra]},'Unrelated native mutation')
    for group in ['groups','updates','controls']:
        for row in walk(value[group]):
            for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed admission harness')
            if 'programInputs' not in row:continue
            executions+=1
            if group!='controls':require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed main rule cohort')
            for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound consumed program')
    rules=summary['nativeExecutions']+executions
    utilities=sum(1 for row in walk(value['groups']) if row.get('utility')=='sign-ed25519')
    summary.update(admitted=True,fullRuleMigrationAcceptance=True,casesPerAPI=dict(COUNTS,production=19),persistentStepsPerAPI=sum(map(len,SEQUENCES)),controls=len(controls),nativeRuleExecutions=rules,nativeUtilityExecutions=utilities,nativeExecutions=rules+utilities)
    return summary


def pack(work):
    stage=work/'admission-stage';stage.mkdir(exist_ok=True)
    for name in ['source.json','production-source.json','api-source.json','report-source.json']:shutil.copyfile(work/name,stage/name)
    for mode in MODES:
        shutil.copyfile(work/f'final-original-{mode}.json',stage/f'original-owner-{mode}.json')
        for i in range(4):shutil.copyfile(work/f'final-production-{mode}-{i}.json',stage/f'production-owner-{mode}-{i}.json')
    for group,prefix in [('api','api-final'),('report','report-owned')]:
        for i in range(16):shutil.copyfile(work/f'final-{group}-{i}.json',stage/f'{prefix}-{i}.json')
    value=async_pattern_checkpoint.pack(stage)
    value.update(format=FORMAT,admitted=True,fullRuleMigrationAcceptance=True,remaining=[],groups={},
        updates={m:[read_json(work/f'final-updates-{m}-{i}.json') for i in range(len(SEQUENCES))] for m in MODES},
        controls=[read_json(work/f'final-controls-{m}-{i}.json') for m in MODES for i in range(len(EDITS))],
        legacyCorpora={n:dict(sha256=sha((ROOT/'gate/data-layer/fixtures/host/data-layer-port'/n).read_bytes())) for n in ['batch-02-recording.json','batch-02-probes.json']})
    for group in COUNTS:
        text=(work/SOURCE_FILES[group]).read_text()
        value['groups'][group]=dict(source=dict(text=text,sha256=sha(text.encode()),value=json.loads(text)),reports={m:[read_json(work/f'final-{group}-{m}-{i}.json') for i in range(PARTS[group])] for m in MODES})
    mutations={d:t for report in value['controls'] for row in report['cases'] for d,t in row['programCopies'].items()}
    for group in ['groups','updates','controls']:
        for row in walk(value[group]):
            for p in row.get('programInputs',[]):
                digest=p['sha256'];text=mutations.get(digest)
                if text is None and digest is not None:text=(ROOT/p['path']).read_text()
                require((sha(text.encode()) if text is not None else None)==digest,'Program changed after native execution');value['programs'][p['path']]=dict(text=text,sha256=digest)
            for path,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Harness changed after native execution');value['harness'][path]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


def admit(path,value):
    summary=validate(value);manifest=read_json(ROOT/MANIFEST);compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact'])
    entry=next(r for r in manifest['rules'] if r['check']==CHECK);compare(entry['ruleSha256'],value['ruleSha256'])
    for name,item in value['programs'].items():
        if not name.startswith('.rerail/') and item['sha256'] is not None:compare(sha((ROOT/name).read_bytes()),item['sha256'])
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port';reports=[]
    for mode in MODES:
        target=folder/f'rr-async-pattern-complete-native-{mode}.json';write_json(target,value['originals'][mode]);reports.append(dict(path=target.relative_to(ROOT).as_posix(),sha256=sha(target.read_bytes())))
    inventory=folder/'rr-original-test-inventory-v2.json';checkpoint=dict(path=path.absolute().relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes()))
    evidence=dict(assertions=11,positives=8,failures=0,skipped=0,semanticFallbacks=0,ruleSha256=entry['ruleSha256'],
        engineCommit=value['engineSource']['cliArtifact']['engineCommit'],engineSha256=value['engineSource']['cliArtifact']['sha256'],corpusSha256=value['originalSourceSha256'],sourceCommit=PIN,
        originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),reports=reports,acceptanceCheckpoints=[checkpoint],supplemental=summary,
        scope='All eleven unchanged original assertions, twelve observations and eight finding-positive observations. Native configuration, collectors, helpers, raw facts, file APIs, CLI, production, earlier inputs, retained edits and valid removal controls pass through both APIs. Supplemental APIs and reporters also pass their full recorded corpora.')
    if entry.get('evidence') and entry['evidence']!=evidence:entry.setdefault('evidenceHistory',[]).append(entry['evidence'])
    entry.update(status='full-contract-passed',evidence=evidence)
    previous=entry.get('migrationProgress')
    if previous and not previous.get('admitted'):entry.setdefault('migrationProgressHistory',[]).append(previous)
    entry['migrationProgress']=dict(status='full-contract-passed',admitted=True,verifiedNativeExecutions=summary['nativeExecutions'],checkpoint=checkpoint,summary=summary,remaining=[])
    require_complete([entry]);write_json(ROOT/MANIFEST,manifest)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate','admit']);p.add_argument('path',type=Path);p.add_argument('--work',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.work);a.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as f:value=json.load(f)
    if a.command=='admit':admit(a.path,value)
    print(json.dumps(validate(value),sort_keys=True))
