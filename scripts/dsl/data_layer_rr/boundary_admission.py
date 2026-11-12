"""Admit boundary only after complete native parity, historical replay, edits and controls."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
import boundary_scanner_checkpoint
from boundary_controls import EDITS,SOURCES,mutate,library_path,invalid_result
from boundary_updates import SEQUENCES
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,compare,require_complete
from scanner_checkpoint import walk
from validate_supplied_facts_checkpoint import require,equal

CHECK='async_sync_boundary'
FORMAT='rerail.boundary-admission/v1'
MODES=('snapshot','workspace')
COUNTS=dict(display=31,cli=37,legacy=17)
PARTS=dict(display=8,cli=37,legacy=4)
SUPPLEMENTAL=dict(api=295,report=158)
CORPORA=dict(api='rr-fresh-async-sync-boundary-cases.json',report='rr-async-sync-boundary-reporter-cases.json')


def validate(value):
    require(value['format']==FORMAT and value['admitted'] is True and value['fullRuleMigrationAcceptance'] is True and value['fullMigrationAcceptance'] is False,'Wrong full boundary admission scope')
    base=dict(value,format=boundary_scanner_checkpoint.FORMAT,admitted=False,remaining=['Additional admission requirements are checked below.'])
    summary=boundary_scanner_checkpoint.validate(base);engine=value['engineSource']['cliArtifact']['sha256']
    events=[r for r in value['originalSource']['events'] if r['check']==CHECK]
    require(len(events)==5 and sum(bool(r.get('findings')) for r in events)==3,'Missing original boundary positives')
    for group,count in COUNTS.items():
        item=value['groups'][group];source=item['source'];oracle=source['value']
        require(sha(source['text'].encode())==source['sha256'] and equal(json.loads(source['text']),oracle),'Changed frontend source bytes')
        require(oracle['sourcePin']==PIN and oracle['mode']=='source' and oracle['passed'] and oracle['totalCases']==count and oracle['sourceSha256']==value['source']['sourceSha256'],'Invalid frontend source')
        wanted={r['input']['id']:r for r in oracle['cases']};require(len(wanted)==count,'Missing frontend source cases')
        for mode in MODES:
            reports=item['reports'][mode];rows=[r for report in reports for r in report['cases']]
            require(Counter(r['input']['id'] for r in rows)==Counter(wanted.keys()),'Missing or repeated frontend case')
            for report in reports:
                require(report['passed'] and report['executionMode']==mode and report['engineSha256']==engine and report['oracle'] is False and not report['semanticFallbacks'],'Failed frontend engine or API')
                require(report['sourceSha256']==oracle['sourceSha256'] and report['baselineSha256']==source['sha256'],'Mixed frontend source')
            for row in rows:
                expected=wanted[row['input']['id']]
                require(row['passed'] and row['native'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Changed complete frontend value')
    legacy=value['groups']['legacy']['source']['value']['cases']
    require(Counter(r['input']['corpus'] for r in legacy)=={'batch-02-recording.json':5,'batch-02-probes.json':12},'Missing earlier input family')
    for row in legacy:require(row['input']['corpusSha256']==value['legacyCorpora'][row['input']['corpus']]['sha256'],'Changed earlier corpus')
    for group,count in SUPPLEMENTAL.items():
        item=value['supplemental'][group];source=item['source'];oracle=source['value'];corpus=item['corpus']
        require(sha(source['text'].encode())==source['sha256'] and equal(json.loads(source['text']),oracle),'Changed supplemental source')
        require(oracle['sourcePin']==PIN and oracle['execution']=='offline-source' and oracle['totalCases']==count and len(oracle['results'])==count,'Incomplete supplemental source')
        require(oracle['sourceFiles']['scripts/lint_async_sync_boundary.py']==value['source']['sourceSha256'],'Mixed supplemental source module')
        require(sha(corpus['text'].encode())==corpus['sha256']==oracle['casesSha256'],'Changed supplemental corpus')
        wanted={r['id']:r for r in oracle['results']};inputs=json.loads(corpus['text'])['cases']
        require(len(wanted)==count and Counter(r['id'] for r in inputs)==Counter(wanted.keys()) and all(r['status']=='passed' for r in wanted.values()),'Missing supplemental source case')
        reports=item['reports'];rows=[r for report in reports for r in report['results']]
        require(Counter(r['id'] for r in rows)==Counter(wanted.keys()),'Missing or repeated supplemental case')
        for report in reports:
            require(report['engineSha256']==engine and report['oracleSha256']==source['sha256'] and report['casesSha256']==corpus['sha256'] and report['execution']=='native-snapshot','Mixed supplemental engine or corpus')
            require(not report.get('semanticFallbacks') and not report.get('sourceDetectorModules'),'Supplemental source callback')
            if group=='api':require(report['status']=='completed' and report['sourceFiles']==oracle['sourceFiles'] and report['calls'] and all(c['status']=='passed' for c in report['calls']),'Incomplete supplemental API execution')
            else:require('activeCaseId' not in report and report['executionIsolation']=='One owned cache per reporter run; reused across all cases','Incomplete or non-isolated reporter execution')
        for row in rows:require(row['status']=='passed' and equal(row['value'],wanted[row['id']]['value']),'Changed complete supplemental value')
    source=value['scanners']['production']['source'];production={r['input']['id']:r for r in source['value']['cases']}
    for mode in MODES:
        reports=value['updates'][mode];require(Counter(r['sequence'] for r in reports)==Counter(range(len(SEQUENCES))),'Missing retained update sequence')
        for report in reports:
            require(report['passed'] and report['complete'] and report['mode']==mode and report['sourcePin']==PIN and report['engineSha256']==engine and report['baselineSha256']==source['sha256'],'Incomplete retained update report')
            sequence=SEQUENCES[report['sequence']]
            require([r['step'] for r in report['cases']]==list(range(len(sequence))) and [r['id'] for r in report['cases']]==sequence,'Missing retained edit step')
            previous={}
            for row in report['cases']:
                expected=production[row['id']];current={f['path']:f for f in expected['input']['files']};changed=sorted(p for p in previous.keys()|current.keys() if previous.get(p)!=current.get(p))
                require(row['changedPaths']==changed,'Changed untouched update fixture')
                require(row['passed'] and row['native'] and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Changed complete retained update value')
                previous=current
    controls=[r for report in value['controls'] for r in report['cases']]
    require(Counter((r['control'],r['mode']) for r in controls)==Counter((n,m) for n in EDITS for m in MODES),'Missing removal control')
    items=dict(components=dict(text=value['sourceText'],sha256=value['sourceSha256'],value=value['source']),**{g:value['scanners'][g]['source'] for g in ['production','public']},**{g:value['groups'][g]['source'] for g in ['display','cli']})
    hashes={g:items[g]['sha256'] for g in SOURCES};oracles={g:{r['input']['id']:r for r in item['value']['cases']} for g,item in items.items()}
    for report in value['controls']:require(report['passed'] and report['complete'] and report['engineSha256']==engine and report['sourcePin']==PIN and not report['semanticFallbacks'] and report['baselineSha256']==hashes,'Incomplete removal report')
    original=value['programs'][value['rulePath']]['text']
    for row in controls:
        group,identity,area,kind,before,after=EDITS[row['control']];expected=oracles[group][identity]
        require(row['group']==group and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']),'Changed removal witness')
        require(row['rejected'] and row['native'] and not equal(row['actual'],expected['expected']) and not invalid_result(row['actual']),'Invalid or ineffective removal control')
        main=original;extra=[]
        if area=='main':main=mutate(main,kind,before,after)
        else:
            extra.append(mutate(value['programs'][library_path(area)]['text'],kind,before,after));main=main.replace(f'#include "data-layer/boundary-{area}.rr"','#include "control.rr"')
        require(row['programCopies']=={sha(t.encode()):t for t in [main,*extra]},'Unrelated removal mutation')
    extra=0;utilities=0
    for group in ['groups','supplemental','updates','controls']:
        for row in walk(value[group]):
            if row.get('utility')=='sign-ed25519':utilities+=1
            for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed admission harness')
            if 'programInputs' not in row:continue
            extra+=1
            if group!='controls':require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed main rule revision')
            for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound consumed admission program')
    rules=summary['nativeExecutions']+extra
    summary.update(admitted=True,fullRuleMigrationAcceptance=True,frontendCasesPerAPI=COUNTS,supplementalCasesSnapshot=SUPPLEMENTAL,persistentStepsPerAPI=sum(map(len,SEQUENCES)),controls=len(controls),nativeRuleExecutions=rules,nativeUtilityExecutions=utilities,nativeExecutions=rules+utilities)
    return summary


def pack(work):
    value=boundary_scanner_checkpoint.pack(work)
    value.update(format=FORMAT,admitted=True,fullRuleMigrationAcceptance=True,remaining=[],groups={},supplemental={},
        updates={m:[read_json(work/f'updates-{m}-{i}.json') for i in range(len(SEQUENCES))] for m in MODES},controls=[read_json(work/f'controls-{m}-{i}.json') for m in MODES for i in range(len(EDITS))],
        legacyCorpora={n:dict(sha256=sha((ROOT/'gate/data-layer/fixtures/host/data-layer-port'/n).read_bytes())) for n in ['batch-02-recording.json','batch-02-probes.json']})
    for group in COUNTS:
        text=(work/f'{group}-source.json').read_text();value['groups'][group]=dict(source=dict(text=text,sha256=sha(text.encode()),value=json.loads(text)),reports={m:[read_json(work/f'{group}-final-{m}-{i}.json') for i in range(PARTS[group])] for m in MODES})
    for group in SUPPLEMENTAL:
        text=(work/f'{group}-source.json').read_text();corpus=(ROOT/'gate/data-layer/fixtures/host/data-layer-port'/CORPORA[group]).read_text()
        value['supplemental'][group]=dict(source=dict(text=text,sha256=sha(text.encode()),value=json.loads(text)),corpus=dict(text=corpus,sha256=sha(corpus.encode())),reports=[read_json(work/f'{group}-final-{i}.json') for i in range(16)])
    mutations={d:t for report in value['controls'] for row in report['cases'] for d,t in row['programCopies'].items()}
    for group in ['groups','supplemental','updates','controls']:
        for row in walk(value[group]):
            for p in row.get('programInputs',[]):
                digest=p['sha256'];text=mutations.get(digest)
                if text is None and digest is not None:text=(ROOT/p['path']).read_text()
                require((sha(text.encode()) if text is not None else None)==digest,'Program changed after admission execution');value['programs'][p['path']]=dict(text=text,sha256=digest)
            for path,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Harness changed after admission execution');value['harness'][path]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


def admit(path,value):
    summary=validate(value);manifest=read_json(ROOT/MANIFEST);compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact'])
    entry=next(r for r in manifest['rules'] if r['check']==CHECK);compare(entry['ruleSha256'],value['ruleSha256'])
    for name,item in value['programs'].items():
        if not name.startswith('.rerail/') and item['sha256'] is not None:compare(sha((ROOT/name).read_bytes()),item['sha256'])
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port';reports=[]
    for mode in MODES:
        target=folder/f'rr-boundary-complete-native-{mode}.json';write_json(target,value['originals'][mode]);reports.append(dict(path=target.relative_to(ROOT).as_posix(),sha256=sha(target.read_bytes())))
    inventory=folder/'rr-original-test-inventory-v2.json';checkpoint=dict(path=path.absolute().relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes()))
    evidence=dict(assertions=5,positives=3,failures=0,skipped=0,semanticFallbacks=0,ruleSha256=entry['ruleSha256'],engineCommit=value['engineSource']['cliArtifact']['engineCommit'],engineSha256=value['engineSource']['cliArtifact']['sha256'],corpusSha256=value['originalSourceSha256'],sourceCommit=PIN,
        originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),reports=reports,acceptanceCheckpoints=[checkpoint],supplemental=summary,
        scope='All five unchanged original assertions and three finding-positive observations. Components, public scanners, production, rendering, CLI, historical inputs, retained edits and valid removal controls pass through both APIs. The complete supplemental API and reporter corpora also pass.')
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
