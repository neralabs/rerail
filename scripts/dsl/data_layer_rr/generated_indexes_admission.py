"""Require complete generated-index contracts before admitting the native migration."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from generated_indexes_checkpoint import CHECK,MODES,coverage
from generated_indexes_updates import SEQUENCES
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,compare,require_complete
from validate_supplied_facts_checkpoint import require,equal

FORMAT='rerail.generated-indexes-admission/v1'
SCANNER_CONTROLS={'direct-call','import-alias','first-keyword','unicode-name','path-fallback','required-parameter','finding-order'}
INTEGRATION_CONTROLS={'metadata-version','supplied-report','cli-status','production-selection','production-facts','extraction-facts','parse-helper','format-helper','unavailable-error'}


def walk(value):
    if isinstance(value,dict):
        yield value
        for child in value.values():yield from walk(child)
    elif isinstance(value,list):
        for child in value:yield from walk(child)


def validate(value):
    require(value['format']==FORMAT and value['fullMigrationAcceptance'] is False,'Wrong admission scope')
    for section in ['programs','harness']:
        for item in value[section].values():
            require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Changed '+section+' snapshot')
    for name in ['original','public','integration','legacy']:
        require(sha(value[name+'SourceText'].encode())==value[name+'SourceSha256'] and equal(json.loads(value[name+'SourceText']),value[name+'Source']),'Changed '+name+' oracle bytes')
    summary=coverage(value)
    for report in value['originals'].values():
        require(not report['hostProtocolCalls'] and not report['casesWithoutNativeCalls'],'Original assertions retained host semantics')
    engine=value['engineSource']['cliArtifact']['sha256']
    for name in ['integration','legacy']:
        source=value[name+'Source'];oracle={r['input']['id']:r for r in source['cases']}
        require(source['passed'] and source['mode']=='source' and source['sourcePin']==PIN,'Invalid '+name+' oracle')
        require(source['sourceSha256']==value['publicSource']['sourceSha256'],'Mixed source modules')
        require(sha(value[name+'SourceText'].encode())==value[name+'SourceSha256'],'Changed '+name+' oracle bytes')
        require(equal(json.loads(value[name+'SourceText']),source),'Changed '+name+' oracle')
        require(len(oracle)==(84 if name=='integration' else 47),'Missing '+name+' source cases')
        if name=='legacy':
            require(Counter(r['input']['corpus'] for r in oracle.values())=={
                'batch-10-shape-seeds.json':2,'batch-10-file-contract-regressions.json':16,
                'batch-10-complete-api.json.gz':24,'batch-10-lifecycle.json':5},'Legacy corpus omitted')
            for r in oracle.values():
                item=value['legacyCorpora'][r['input']['corpus']]
                require(item['sha256']==r['input']['corpusSha256'],'Changed legacy corpus')
        for mode in MODES:
            reports=value[name][mode];rows=[c for r in reports for c in r['cases']]
            require(Counter(c['input']['id'] for c in rows)==Counter(oracle.keys()),'Missing or duplicate '+name+' cases')
            for report in reports:
                require(report['passed'] and report['oracle'] is False and not report['semanticFallbacks'],'Failed '+name+' native report')
                require(report['engineSha256']==engine and report['executionMode']==mode,'Mixed '+name+' engine or API')
                require(report['baselineSha256']==value[name+'SourceSha256'],'Wrong '+name+' baseline')
            for row in rows:
                expected=oracle[row['input']['id']]
                require(row['passed'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Different '+name+' full return')
                require(bool(row['native']) or name=='integration' and row['input']['id'] in {'cli-help','cli-bad-option'},'Missing '+name+' native execution')
    production={r['input']['id']:r for r in value['integrationSource']['cases'] if r['input']['kind']=='production'}
    for mode in MODES:
        reports=value['updates'][mode]
        require(Counter(r['sequence'] for r in reports)==Counter(range(len(SEQUENCES))),'Missing update sequence')
        for report in reports:
            require(report['passed'] and report['complete'] and report['mode']==mode and report['engineSha256']==engine,'Incomplete update report')
            require(report['baselineSha256']==value['integrationSourceSha256'],'Wrong update oracle')
            sequence=SEQUENCES[report['sequence']]
            require([r['step'] for r in report['cases']]==list(range(len(sequence))) and [r['id'] for r in report['cases']]==['production-'+x for x in sequence],'Missing update step')
            for row in report['cases']:
                expected=production[row['id']]['expected']
                require(row['passed'] and equal(row['expected'],expected) and equal(row['actual'],expected) and row['native'],'Update result differs')
    controls=value['integrationControls']
    require(controls['passed'] and not controls['semanticFallbacks'] and controls['engineSha256']==engine,'Failed integration controls')
    require(controls['baselineSha256']==value['integrationSourceSha256'],'Wrong integration-control oracle')
    require(Counter((r['control'],r['mode']) for r in controls['cases'])==Counter((n,m) for n in INTEGRATION_CONTROLS for m in MODES),'Missing integration control')
    oracle={r['input']['id']:r for r in value['integrationSource']['cases']}
    for row in controls['cases']:
        expected=oracle[row['input']['id']]
        require(equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']),'Changed integration-control witness')
        require(row['rejected'] and not equal(row['actual'],row['expected']) and row['native'],'Ineffective integration control')
        require('error' not in row['actual'] or row['control']=='unavailable-error','Invalid integration mutant')
        for digest,text in row['programCopies'].items():require(sha(text.encode())==digest,'Changed integration mutant')
    executions=0
    for name in ['originals','public','integration','legacy','updates']:
        for row in walk(value[name]):
            if 'programInputs' in row:
                require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed main policy versions')
    for name in ['integration','legacy','updates','integrationControls','integerProbe','worldProbe','retainedWorldProbe']:
        for row in walk(value[name]):
            for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed acceptance harness')
            if 'programInputs' not in row:continue
            executions+=1
            for program in row['programInputs']:
                item=value['programs'][program['sha256'] or 'missing']
                require(program['path'] in item['paths'] and item['sha256']==program['sha256'],'Unbound consumed program')
    for name in ['integerProbe','worldProbe','retainedWorldProbe']:
        probe=value[name]
        require(probe['engineSha256']==engine and probe['passed'] and probe['cases'] and all(r['passed'] for r in probe['cases']),'Failed generic native probe')
    summary.update(admitted=True,integrationCasesPerAPI=84,legacyCasesPerAPI=47,persistentStepsPerAPI=sum(map(len,SEQUENCES)),integrationControls=18,nativeExecutions=summary['nativeExecutions']+executions,fullMigrationAcceptance=False,
        hostProtocols={'cli-help':'Unchanged argparse help exits before rule evaluation.','cli-bad-option':'Unchanged argparse rejects an unknown option before rule evaluation.'})
    return summary


def pack(work):
    manifest=read_json(ROOT/MANIFEST);rule=next(r for r in manifest['rules'] if r['check']==CHECK)
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
    value=dict(format=FORMAT,fullMigrationAcceptance=False,engineSource=manifest['engineSource'],ruleSha256=rule['ruleSha256'],rulePath=rule['rulePath'],
        inventory=read_json(folder/'rr-original-test-inventory-v2.json'),programs={},harness={},
        originals={m:read_json(work/f'original-{m}.json') for m in MODES},
        public={m:[read_json(work/f'public-{m}-{i}.json') for i in range(8)] for m in MODES},
        integration={m:[read_json(work/f'integration-{m}-{i}.json') for i in range(6)] for m in MODES},
        legacy={m:[read_json(work/f'legacy-{m}-{i}.json') for i in range(4)] for m in MODES},
        updates={m:[read_json(work/f'updates-{m}-{i}.json') for i in range(3)] for m in MODES},
        controls=read_json(work/'scanner-controls.json'),integrationControls=read_json(work/'integration-controls.json'),
        legacyCorpora={p.name:dict(sha256=sha(p.read_bytes())) for p in [folder/n for n in ['batch-10-shape-seeds.json','batch-10-file-contract-regressions.json','batch-10-complete-api.json.gz','batch-10-lifecycle.json']]})
    for name,file in [('original','source.json'),('public','public-source-expanded.json'),('integration','source-final.json'),('legacy','legacy-source.json')]:
        text=(work/file).read_text();value[name+'Source']=json.loads(text);value[name+'SourceSha256']=sha(text.encode());value[name+'SourceText']=text
    for name,file in [('integerProbe','integer-probe.json'),('worldProbe','world-probe.json'),('retainedWorldProbe','retained-world-probe.json')]:value[name]=read_json(work/file)
    mutations={}
    for row in value['controls']['cases']:
        mutations[row['policySha256']]=row['mutatedPolicy']
        text=row['companionTable'];mutations[sha(text.encode())]=text
    for row in value['integrationControls']['cases']:mutations.update(row['programCopies'])
    for name in ['originals','public','integration','legacy','updates','controls','integrationControls','integerProbe','worldProbe','retainedWorldProbe']:
        for row in walk(value[name]):
            for program in row.get('programInputs',[]):
                key=program['sha256'] or 'missing';text=mutations.get(key)
                if text is None and program['sha256'] is not None:text=(ROOT/program['path']).read_text()
                require((sha(text.encode()) if text is not None else None)==program['sha256'],'Program changed after execution')
                item=value['programs'].setdefault(key,dict(sha256=program['sha256'],text=text,paths=[]))
                if program['path'] not in item['paths']:item['paths'].append(program['path'])
            for path,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Harness changed after execution')
                value['harness'][path]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


def admit(path,value):
    summary=validate(value);manifest=read_json(ROOT/MANIFEST)
    compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact'])
    entry=next(r for r in manifest['rules'] if r['check']==CHECK)
    compare(entry['ruleSha256'],value['ruleSha256']);compare(sha((ROOT/entry['rulePath']).read_bytes()),entry['ruleSha256'])
    for item in value['programs'].values():
        for name in item['paths']:
            if not name.startswith('.rerail/') and item['sha256'] is not None:compare(sha((ROOT/name).read_bytes()),item['sha256'])
    checkpoint=dict(path=path.absolute().relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes()))
    reports=[];folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
    for mode in MODES:
        target=folder/f'rr-generated-indexes-complete-native-{mode}.json';write_json(target,value['originals'][mode])
        reports.append(dict(path=target.relative_to(ROOT).as_posix(),sha256=sha(target.read_bytes())))
    inventory=folder/'rr-original-test-inventory-v2.json'
    evidence=dict(assertions=5,positives=4,failures=0,skipped=0,semanticFallbacks=0,
        ruleSha256=entry['ruleSha256'],engineCommit=value['engineSource']['cliArtifact']['engineCommit'],
        engineSha256=value['engineSource']['cliArtifact']['sha256'],corpusSha256=value['originalSourceSha256'],sourceCommit=PIN,
        originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),
        reports=reports,acceptanceCheckpoints=[checkpoint],supplemental=summary,
        scope='All original assertions, all earlier batch-ten inputs, scanner, registered collector, supplied facts, helpers, CLI, production, persistent updates and removal controls through both native APIs.')
    if entry.get('evidence') and entry['evidence']!=evidence:entry.setdefault('evidenceHistory',[]).append(entry['evidence'])
    entry.update(status='full-contract-passed',evidence=evidence)
    entry.setdefault('migrationProgress',{}).update(status='full-contract-passed',verifiedNativeExecutions=summary['nativeExecutions'])
    require_complete([entry]);write_json(ROOT/MANIFEST,manifest)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','validate','admit']);parser.add_argument('path',type=Path);parser.add_argument('--work',type=Path)
    args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.work);args.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    if args.command=='admit':admit(args.path,value)
    print(json.dumps(validate(value),sort_keys=True))
