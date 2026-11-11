"""Bind generated-index progress to complete originals, supplemental cases, and native controls."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha
from validate_supplied_facts_checkpoint import require,equal

CHECK='generated_index_sources'
FORMAT='rerail.generated-indexes-progress/v1'
MODES=('snapshot','workspace')


def coverage(value):
    inventory=value['inventory'];require(inventory['sourcePin']==PIN,'Wrong independent source pin')
    files={p for p,r in inventory['files'].items() if CHECK in r['detectors']}
    wanted={p for p in inventory['cases'] if p.split('::',1)[0] in files}
    require(len(wanted)==5,'Wrong independent original inventory')
    source=value['originalSource']
    require(source['commit']==PIN and source['pytest_exit_code']==0 and not source['source_mismatches'],'Original source recording failed')
    expected={r['test']:r for r in source['events'] if r['check']==CHECK}
    require(set(expected)==wanted and sum(bool(r['findings']) for r in expected.values())==4,'Original positives omitted')
    public=value['publicSource'];require(public['sourcePin']==PIN and public['passed'],'Supplemental source recording failed')
    oracle={r['input']['id']:r for r in public['cases']}
    require(len(oracle)==117,'Wrong supplemental corpus')
    engine=value['engineSource']['cliArtifact']['sha256']
    native_executions=0
    def consumed(row):
        nonlocal native_executions
        if isinstance(row,dict):
            if 'programInputs' in row:
                native_executions+=1
                for p in row['programInputs']:
                    item=value['programs'][p['sha256'] or 'missing']
                    require(p['path'] in item['paths'] and item['sha256']==p['sha256'],'Native program provenance differs')
            for p,digest in row.get('harnessFiles',{}).items():require(value['harness'][p]['sha256']==digest,'Mixed harness versions')
            for child in row.values():consumed(child)
        elif isinstance(row,list):
            for child in row:consumed(child)
    for mode in MODES:
        row=value['originals'][mode]
        require(row['oracle'] is False and row['exitCode']==0 and row['mode']==mode and row['engineSha256']==engine,'Original native suite failed')
        require(not row['semanticFallbacks'] and not row['missingResults'] and not row.get('retainedCaseIds'),'Original assertions delegated or omitted')
        require(set(row['cases'])==wanted and set(row['collected'])==wanted,'Original assertions omitted')
        require(all(len(p)==3 and {r['phase']:r['outcome'] for r in p}==dict(setup='passed',call='passed',teardown='passed') for p in row['cases'].values()),'Original assertion failed or skipped')
        require(all(row['sourceFiles'].get(p)==inventory['files'][p]['sha256'] for p in files),'Original source test changed')
        require(row['policySha256'][CHECK]==value['ruleSha256'] and row['originalRecordingSha256']==value['originalSourceSha256'],'Original policy or source recording differs')
        require(Counter(r['test'] for r in row['calls'] if r['status']=='passed')==Counter(wanted),'Original native executions omitted or failed')
        observations=row['detectorObservations'];require(Counter(r['test'] for r in observations)==Counter(wanted),'Original observations omitted or repeated')
        for r in observations:
            e=expected[r['test']]
            require(r['status']=='passed' and equal(r['expected'],e['findings']) and equal(r['actual'],e['findings']) and equal(r['actualSourceResult'],e['sourceResult']),'Original complete return differs')
        parts=value['public'][mode];require(len(parts)==8 and Counter(r['part'] for r in parts)==Counter(range(8)),'Supplemental shards omitted or repeated')
        rows=[c for r in parts for c in r['cases']]
        require(Counter(c['input']['id'] for c in rows)==Counter(oracle.keys()),'Supplemental cases omitted or repeated')
        for r in parts:
            require(r['oracle'] is False and r['passed'] and not r['semanticFallbacks'] and r['engineSha256']==engine and r['executionMode']==mode,'Native supplemental suite failed')
            require(r['baselineSha256']==value['publicSourceSha256'],'Wrong supplemental source recording')
        for r in rows:
            e=oracle[r['input']['id']]
            require(r['passed'] and equal(r['input'],e['input']) and equal(r['expected'],e['expected']) and equal(r['actual'],e['expected']),'Supplemental complete return differs')
            require(bool(r['native']) or r['input']['id']=='invalid-utf8','Supplemental rule did not execute')
        consumed(row);consumed(parts)
    controls=value['controls'];require(controls['passed'] and controls['engineSha256']==engine and not controls['semanticFallbacks'],'Native removal controls failed')
    require(controls['baselineSha256']==value['publicSourceSha256'],'Wrong control source recording')
    wanted_controls={(name,mode) for mode in MODES for name in ['direct-call','import-alias','first-keyword','unicode-name','path-fallback','required-parameter','finding-order']}
    require(Counter((r['control'],r['mode']) for r in controls['cases'])==Counter(wanted_controls),'Removal controls omitted or repeated')
    for r in controls['cases']:
        e=oracle[r['input']['id']]
        require(equal(r['input'],e['input']) and equal(r['expected'],e['expected']) and bool(e['expected']) and isinstance(r['actual'],list) and not equal(r['actual'],e['expected']) and r['rejected'] and r['native'],'Ineffective removal control')
        require(sha(r['mutatedPolicy'].encode())==r['policySha256'],'Removal policy differs')
    consumed(controls)
    return dict(originalCasesPerAPI=5,originalPositiveObservationsPerAPI=4,supplementalCasesPerAPI=117,
        supplementalFindingPositiveCases=sum(isinstance(r['expected'],list) and bool(r['expected']) for r in oracle.values()),
        controls=14,nativeExecutions=native_executions,admitted=False)


def validate(value):
    require(value['format']==FORMAT and value['fullMigrationAcceptance'] is False,'Wrong progress scope')
    for item in value['programs'].values():require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Program snapshot differs')
    for item in value['harness'].values():require(sha(item['text'].encode())==item['sha256'],'Harness snapshot differs')
    history=value['identifierRegression'];policy=history['policy'];report=history['report']
    require(sha(policy['text'].encode())==policy['sha256'],'Identifier regression policy differs')
    history_engine=history.get('engineSource',value['engineSource'])
    require(report['engineSha256']==history_engine['cliArtifact']['sha256'] and not report['semanticFallbacks'],'Identifier regression engine differs')
    require(len(report['cases'])==9 and all(not c['passed'] and not equal(c['actual'],c['expected']) for c in report['cases']),'Identifier regression failures missing')
    for c in report['cases']:
        require(any(p['sha256']==policy['sha256'] for n in c['native'] for p in n['programInputs']),'Identifier regression did not consume its recorded policy')
    return coverage(value)


def pack(work):
    manifest=read_json(ROOT/MANIFEST);rule=next(r for r in manifest['rules'] if r['check']==CHECK)
    original=work/'source.json';public=work/'public-source-expanded.json'
    value=dict(format=FORMAT,fullMigrationAcceptance=False,engineSource=manifest['engineSource'],ruleSha256=rule['ruleSha256'],
        inventory=read_json(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json'),
        originalSource=read_json(original),originalSourceSha256=sha(original.read_bytes()),publicSource=read_json(public),publicSourceSha256=sha(public.read_bytes()),
        originals={m:read_json(work/f'accepted-original-{m}.json') for m in MODES},
        public={m:[read_json(work/f'accepted-public-{m}-{i}.json') for i in range(8)] for m in MODES},
        controls=read_json(work/'controls.json'),programs={},harness={},
        identifierRegression=dict(report=read_json(work/'identifier-red.json'),policy=dict(
            text=(work/'pre-identifier-policy.rr').read_text(),sha256=sha((work/'pre-identifier-policy.rr').read_bytes()))),
        remaining=['Native plugin and supplied-fact reporting','CLI contracts','Persistent workspace updates','Full production and shared registration acceptance'])
    mutations={r['policySha256']:r['mutatedPolicy'] for r in value['controls']['cases']}
    history=value['identifierRegression']
    history['engineSource']=next(s for s in [manifest['engineSource'],*manifest.get('engineSourceSnapshots',[])]
        if s.get('cliArtifact',{}).get('sha256')==history['report']['engineSha256'])
    def collect(row):
        if isinstance(row,dict):
            for p in row.get('programInputs',[]):
                key=p['sha256'] or 'missing';text=mutations.get(key)
                if text is None and p['sha256'] is not None:text=(ROOT/p['path']).read_text()
                require((sha(text.encode()) if text is not None else None)==p['sha256'],'Program changed after execution')
                item=value['programs'].setdefault(key,dict(text=text,sha256=p['sha256'],paths=[]))
                if p['path'] not in item['paths']:item['paths'].append(p['path'])
            for p,digest in row.get('harnessFiles',{}).items():
                raw=(ROOT/p).read_bytes();require(sha(raw)==digest,'Harness changed after execution')
                value['harness'][p]=dict(text=raw.decode(),sha256=digest)
            for child in row.values():collect(child)
        elif isinstance(row,list):
            for child in row:collect(child)
    for k in ['originals','public','controls']:collect(value[k])
    value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('command',choices=['pack','validate']);parser.add_argument('path',type=Path);parser.add_argument('--work',type=Path)
    args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.work);args.path.write_bytes(gzip.compress(json.dumps(value,sort_keys=True,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
