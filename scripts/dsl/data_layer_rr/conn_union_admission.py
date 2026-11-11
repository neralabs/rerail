"""Require complete connection-union parity before admitting the native migration."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from conn_union_controls import EDITS
from conn_union_updates import SEQUENCES
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,compare,require_complete
import scanner_checkpoint
from validate_supplied_facts_checkpoint import require,equal

CHECK='conn_or_txn_union'
MODES=('snapshot','workspace')
COUNTS=dict(scanner=64,components=118,production=18,cli=36,legacy=15)
PARTS=dict(scanner=4,components=4,production=2,cli=6,legacy=2)
SOURCE_FILES=dict(scanner='scanner-source.json',components='components-source-expanded.json',production='production-source-current.json',cli='cli-source-current.json',legacy='legacy-source.json',original='source.json')
FORMAT='rerail.conn-union-admission/v1'
HOST_PROTOCOLS={'parser-help','parser-bad-option'}


def walk(value):
    if isinstance(value,dict):
        yield value
        for child in value.values():yield from walk(child)
    elif isinstance(value,list):
        for child in value:yield from walk(child)


def validate(value):
    require(value['format']==FORMAT and value['fullMigrationAcceptance'] is False,'Wrong rule admission scope')
    for section in ['programs','harness']:
        for item in value[section].values():require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Changed '+section+' snapshot')
    for name,item in value['sources'].items():
        require(sha(item['text'].encode())==item['sha256'] and equal(json.loads(item['text']),item['value']),'Changed '+name+' source snapshot')
    original=value['sources']['original'];source=value['sources']['scanner']
    base=dict(format='rerail.scanner-checkpoint/v1',check=CHECK,admitted=False,fullMigrationAcceptance=False,remaining=['Other contracts require separate validation below.'],
        source=source['value'],sourceText=source['text'],sourceSha256=source['sha256'],originalSource=original['value'],originalSourceText=original['text'],originalSourceSha256=original['sha256'],
        rulePath=value['rulePath'],ruleSha256=value['ruleSha256'],engineSource=value['engineSource'],inventory=value['inventory'],originals=value['originals'],scanners=value['reports']['scanner'],programs=value['programs'],harness=value['harness'])
    summary=scanner_checkpoint.validate(base)
    require(summary['originalCasesPerAPI']==5 and summary['originalPositiveObservationsPerAPI']==2,'Changed original inventory or positive witnesses')
    engine=value['engineSource']['cliArtifact']['sha256'];oracles={}
    for group,count in COUNTS.items():
        item=value['sources'][group];source=item['value'];oracle={r['input']['id']:r for r in source['cases']};oracles[group]=oracle
        require(source['passed'] and source['mode']=='source' and source['sourcePin']==PIN and len(oracle)==count and source['totalCases']==count,'Missing '+group+' source cases')
        require(source['sourceSha256']==value['sources']['scanner']['value']['sourceSha256'],'Mixed original modules')
        for mode in MODES:
            reports=value['reports'][group][mode];rows=[row for report in reports for row in report['cases']]
            require(Counter(r['input']['id'] for r in rows)==Counter(oracle.keys()),'Missing or repeated '+group+' case')
            for report in reports:
                require(report['passed'] and report['oracle'] is False and not report['semanticFallbacks'],'Failed '+group+' native report')
                require(report['executionMode']==mode and report['engineSha256']==engine,'Mixed '+group+' engine or API')
                require(report['sourceSha256']==source['sourceSha256'] and report['baselineSha256']==item['sha256'],'Wrong '+group+' oracle')
            for row in rows:
                expected=oracle[row['input']['id']]
                require(row['passed'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Different '+group+' complete result')
                require(bool(row['native']) or group=='cli' and row['input']['id'] in HOST_PROTOCOLS,'Missing '+group+' native execution')
    require(Counter(r['input']['corpus'] for r in oracles['legacy'].values())=={'batch-02-recording.json':5,'batch-02-probes.json':10},'Missing historical input family')
    for row in oracles['legacy'].values():require(value['legacyCorpora'][row['input']['corpus']]['sha256']==row['input']['corpusSha256'],'Changed historical corpus')
    for mode in MODES:
        reports=value['updates'][mode];require(Counter(r['sequence'] for r in reports)==Counter(range(len(SEQUENCES))),'Missing update sequence')
        for report in reports:
            require(report['passed'] and report['complete'] and report['mode']==mode and report['engineSha256']==engine,'Incomplete update report')
            require(report['baselineSha256']==value['sources']['production']['sha256'],'Wrong update oracle')
            sequence=SEQUENCES[report['sequence']]
            require([r['step'] for r in report['cases']]==list(range(len(sequence))) and [r['id'] for r in report['cases']]==sequence,'Missing update step')
            previous={}
            for row in report['cases']:
                expected=oracles['production'][row['id']];current={f['path']:f for f in expected['input']['files']}
                changed=sorted(p for p in previous.keys()|current.keys() if previous.get(p)!=current.get(p))
                require(row['changedPaths']==changed,'Changed untouched fixture in persistent replay')
                require(row['passed'] and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']) and row['native'],'Different persistent result')
                previous=current
    controls=[row for report in value['controls'] for row in report['cases']]
    require(Counter((r['control'],r['mode']) for r in controls)==Counter((n,m) for n in EDITS for m in MODES),'Missing behavior-removal control')
    for report in value['controls']:
        require(report['passed'] and report['complete'] and report['engineSha256']==engine and not report['semanticFallbacks'],'Failed native controls')
        require(report['baselineSha256']=={g:value['sources'][g]['sha256'] for g in ['scanner','components','production','cli']},'Wrong control oracles')
    for row in controls:
        group,identity,*_=EDITS[row['control']];expected=oracles[group][identity]
        require(row['group']==group and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']),'Changed control witness')
        require(row['rejected'] and not equal(row['actual'],row['expected']) and row['native'],'Ineffective native control')
        require(not isinstance(row['actual'],dict) or 'error' not in row['actual'],'Invalid native mutant')
        for digest,text in row['programCopies'].items():require(sha(text.encode())==digest,'Changed native mutant bytes')
    executions=0
    for group in ['originals','reports','updates','controls']:
        for row in walk(value[group]):
            for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed acceptance harness')
            if 'programInputs' not in row:continue
            executions+=1
            if group!='controls':require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed main policy cohort')
            for program in row['programInputs']:require(value['programs'][program['path']]['sha256']==program['sha256'],'Unbound consumed program')
    summary.update(admitted=True,casesPerAPI=COUNTS,persistentStepsPerAPI=sum(map(len,SEQUENCES)),controls=len(controls),nativeExecutions=executions,fullMigrationAcceptance=False,
        hostProtocols={'parser-help':'Unchanged argparse help exits before rule evaluation.','parser-bad-option':'Unchanged argparse rejects the unknown option before rule evaluation.'})
    return summary


def pack(work):
    manifest=read_json(ROOT/MANIFEST);rule=next(r for r in manifest['rules'] if r['check']==CHECK)
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
    value=dict(format=FORMAT,fullMigrationAcceptance=False,rulePath=rule['rulePath'],ruleSha256=rule['ruleSha256'],engineSource=manifest['engineSource'],
        inventory=read_json(folder/'rr-original-test-inventory-v2.json'),sources={},programs={},harness={},
        originals={m:read_json(work/f'final-original-{m}.json') for m in MODES},
        reports={g:{m:[read_json(work/(f'cli-final-{m}-{i}.json' if g=='cli' else f'final-{g}-{m}-{i}.json')) for i in range(PARTS[g])] for m in MODES} for g in COUNTS},
        updates={m:[read_json(work/f'updates-{m}-{i}.json') for i in range(len(SEQUENCES))] for m in MODES},controls=[read_json(work/f'controls-{i}.json') for i in range(4)],
        legacyCorpora={n:dict(sha256=sha((folder/n).read_bytes())) for n in ['batch-02-recording.json','batch-02-probes.json']})
    for name,file in SOURCE_FILES.items():
        text=(work/file).read_text();value['sources'][name]=dict(text=text,sha256=sha(text.encode()),value=json.loads(text))
    mutations={d:t for report in value['controls'] for row in report['cases'] for d,t in row['programCopies'].items()}
    for group in ['originals','reports','updates','controls']:
        for row in walk(value[group]):
            for program in row.get('programInputs',[]):
                digest=program['sha256'];text=mutations.get(digest)
                if text is None and digest is not None:text=(ROOT/program['path']).read_text()
                require((sha(text.encode()) if text is not None else None)==digest,'Program changed after execution')
                value['programs'][program['path']]=dict(text=text,sha256=digest)
            for path,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Harness changed after execution')
                value['harness'][path]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


def admit(path,value):
    summary=validate(value);manifest=read_json(ROOT/MANIFEST)
    compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact'])
    entry=next(r for r in manifest['rules'] if r['check']==CHECK)
    compare(entry['ruleSha256'],value['ruleSha256'])
    for name,item in value['programs'].items():
        if not name.startswith('.rerail/') and item['sha256'] is not None:compare(sha((ROOT/name).read_bytes()),item['sha256'])
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port';reports=[]
    for mode in MODES:
        target=folder/f'rr-conn-union-complete-native-{mode}.json';write_json(target,value['originals'][mode]);reports.append(dict(path=target.relative_to(ROOT).as_posix(),sha256=sha(target.read_bytes())))
    inventory=folder/'rr-original-test-inventory-v2.json'
    evidence=dict(assertions=5,positives=2,failures=0,skipped=0,semanticFallbacks=0,ruleSha256=entry['ruleSha256'],
        engineCommit=value['engineSource']['cliArtifact']['engineCommit'],engineSha256=value['engineSource']['cliArtifact']['sha256'],corpusSha256=value['sources']['original']['sha256'],sourceCommit=PIN,
        originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),reports=reports,
        acceptanceCheckpoints=[dict(path=path.absolute().relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes()))],supplemental=summary,
        scope='All original assertions, all retained batch-two inputs, scanners, registered extraction, type helpers, supplied facts, CLI, production, persistent updates and removal controls through both native APIs.')
    if entry.get('evidence') and entry['evidence']!=evidence:entry.setdefault('evidenceHistory',[]).append(entry['evidence'])
    entry.update(status='full-contract-passed',evidence=evidence)
    entry.setdefault('migrationProgress',{}).update(status='full-contract-passed',verifiedNativeExecutions=summary['nativeExecutions'],remaining=[])
    require_complete([entry]);write_json(ROOT/MANIFEST,manifest)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','validate','admit']);parser.add_argument('path',type=Path);parser.add_argument('--work',type=Path);args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.work);args.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    if args.command=='admit':admit(args.path,value)
    print(json.dumps(validate(value),sort_keys=True))
