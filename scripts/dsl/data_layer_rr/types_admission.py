"""Admit type safety only after complete parity, retained updates, and valid controls."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from assertion_shards import merge
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,compare,require_complete
from scanner_checkpoint import walk
import types_progress
from types_controls import EDITS,mutate
from types_updates import SEQUENCES
from validate_supplied_facts_checkpoint import require,equal

CHECK=types_progress.CHECK
MODES=types_progress.MODES
FORMAT='rerail.type-safety-admission/v1'
CONTROL_PARTS=8


def validate(value):
    require(value['format']==FORMAT and value['admitted'] is True and value['fullRuleMigrationAcceptance'] is True and value['fullMigrationAcceptance'] is False,'Wrong full rule admission scope')
    base=dict(value,format=types_progress.FORMAT,admitted=False,remaining=['Additional admission requirements are checked below.'])
    summary=types_progress.validate(base);engine=value['engineSource']['cliArtifact']['sha256']
    source=value['sources']['production'];oracle={r['input']['id']:r for r in source['value']['cases']}
    for mode in MODES:
        reports=value['updates'][mode]
        require(Counter(r['sequence'] for r in reports)==Counter(range(len(SEQUENCES))),'Missing update sequence')
        for report in reports:
            require(report['passed'] and report['complete'] and report['mode']==mode and report['sourcePin']==PIN and report['engineSha256']==engine,'Incomplete update report')
            require(report['baselineSha256']==source['sha256'],'Wrong update oracle')
            sequence=SEQUENCES[report['sequence']]
            require([r['step'] for r in report['cases']]==list(range(len(sequence))) and [r['id'] for r in report['cases']]==sequence,'Missing update step')
            previous={}
            for row in report['cases']:
                expected=oracle[row['id']];current={f['path']:f for f in expected['input']['files']}
                changed=sorted(p for p in previous.keys()|current.keys() if previous.get(p)!=current.get(p))
                require(row['changedPaths']==changed,'Changed untouched fixture in update')
                require(row['passed'] and row['native'] and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Different update result')
                previous=current
    controls=[r for report in value['controls'] for r in report['cases']]
    require(Counter((r['control'],r['mode']) for r in controls)==Counter((n,m) for n in EDITS for m in MODES),'Missing behavior-removal control')
    for report in value['controls']:
        require(report['passed'] and report['complete'] and report['engineSha256']==engine and report['sourcePin']==PIN and not report['semanticFallbacks'],'Incomplete behavior-removal report')
        require(report['baselineSha256']=={g:value['sources'][g]['sha256'] for g in ['public','components','production','cli']},'Wrong control oracles')
    oracles={g:{r['input']['id']:r for r in value['sources'][g]['value']['cases']} for g in ['public','components','production','cli']}
    original_main=value['programs'][value['rulePath']]['text']
    for row in controls:
        group,identity,area,kind,before,after=EDITS[row['control']];expected=oracles[group][identity]
        require(row['group']==group and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']),'Changed control witness')
        require(row['rejected'] and row['native'] and not equal(row['actual'],expected['expected']),'Ineffective behavior-removal control')
        require(not isinstance(row['actual'],dict) or 'error' not in row['actual'],'Invalid native mutant')
        main=original_main;extra=[]
        if area=='main':main=mutate(main,kind,before,after)
        else:
            library=value['programs'][f'docs/rule-syntax/lib/data-layer/type-safety-{area}.rr']['text']
            extra.append(mutate(library,kind,before,after));main=main.replace(f'#include "data-layer/type-safety-{area}.rr"','#include "control.rr"')
        require(row['programCopies']=={sha(t.encode()):t for t in [main,*extra]},'Control differs from the specified mutation')
    extra_executions=0
    for group in ['updates','controls']:
        for row in walk(value[group]):
            for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed admission harness')
            if 'programInputs' not in row:continue
            extra_executions+=1
            if group=='updates':require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed update policy cohort')
            for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound consumed admission program')
    summary.update(admitted=True,fullRuleMigrationAcceptance=True,persistentStepsPerAPI=sum(map(len,SEQUENCES)),controls=len(controls),nativeExecutions=summary['nativeExecutions']+extra_executions)
    return summary


def pack(work):
    value=types_progress.pack(work)
    value.update(format=FORMAT,admitted=True,fullRuleMigrationAcceptance=True,remaining=[],
        updates={m:[read_json(work/f'updates-{m}-{i}.json') for i in range(len(SEQUENCES))] for m in MODES},
        controls=[read_json(work/f'controls-{i}.json') for i in range(CONTROL_PARTS)])
    mutations={d:t for report in value['controls'] for row in report['cases'] for d,t in row['programCopies'].items()}
    for group in ['updates','controls']:
        for row in walk(value[group]):
            for p in row.get('programInputs',[]):
                digest=p['sha256'];text=mutations.get(digest)
                if text is None and digest is not None:text=(ROOT/p['path']).read_text()
                require((sha(text.encode()) if text is not None else None)==digest,'Program changed after admission execution');value['programs'][p['path']]=dict(text=text,sha256=digest)
            for path,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Harness changed after admission execution');value['harness'][path]=dict(text=text,sha256=digest)
    for path in [Path(__file__),Path(__file__).with_name('assertion_shards.py')]:
        text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


def admit(path,value):
    summary=validate(value);manifest=read_json(ROOT/MANIFEST);compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact'])
    entry=next(r for r in manifest['rules'] if r['check']==CHECK);compare(entry['ruleSha256'],value['ruleSha256'])
    for name,item in value['programs'].items():
        if not name.startswith('.rerail/') and item['sha256'] is not None:compare(sha((ROOT/name).read_bytes()),item['sha256'])
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port';reports=[]
    for mode in MODES:
        target=folder/f'rr-type-safety-complete-native-{mode}.json';write_json(target,merge(value['originals'][mode]));reports.append(dict(path=target.relative_to(ROOT).as_posix(),sha256=sha(target.read_bytes())))
    inventory=folder/'rr-original-test-inventory-v2.json';checkpoint=dict(path=path.absolute().relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes()))
    evidence=dict(assertions=193,positives=31,failures=0,skipped=0,semanticFallbacks=0,ruleSha256=entry['ruleSha256'],
        engineCommit=value['engineSource']['cliArtifact']['engineCommit'],engineSha256=value['engineSource']['cliArtifact']['sha256'],corpusSha256=value['sources']['original']['sha256'],sourceCommit=PIN,
        originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),reports=reports,acceptanceCheckpoints=[checkpoint],supplemental=summary,
        scope='All 193 original assertions, including 31 finding-positive observations. Native configuration, passive DTO and parser protocols are identified. Public APIs, registered collectors, supplied facts, CLI, production, all earlier inputs, persistent updates and removal controls pass through both APIs.')
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
        with gzip.open(a.path,'rt') as stream:value=json.load(stream)
    if a.command=='admit':admit(a.path,value)
    print(json.dumps(validate(value),sort_keys=True))
