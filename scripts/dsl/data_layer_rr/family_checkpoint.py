"""Audit complete recorded rule families, original assertions, updates and controls."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,require_complete
from validate_supplied_facts_checkpoint import require,equal

FORMAT='rerail.recorded-rule-family/v1'
SPECS={'classvar_check':dict(public=82,components=36,cli=40,production=17,original=8,positive=5,updates=19,controls=9)}
MODES={'snapshot','workspace'}
PRE_QUERY_ERRORS={'public':{'file-missing':'FileNotFoundError','file-invalid-utf8':'UnicodeDecodeError'},
                  'cli':{'parser---help':'SystemExit','parser---bad-flag':'SystemExit'}}


def validate(value):
    require(value['format']==FORMAT and value['check'] in SPECS,'Unsupported family receipt')
    check=value['check'];spec=SPECS[check];sources=value['sources'];engine=value['engineSource']['cliArtifact']['sha256']
    for item in sources.values():
        require(sha(item['text'].encode())==item['sha256'],'Source recording differs')
    source={k:json.loads(item['text']) for k,item in sources.items()}
    require(source['original']['commit']==PIN and source['original']['pytest_exit_code']==0 and not source['original']['source_mismatches'],'Original source suite failed')
    require(len(source['original']['test_outcomes'])==spec['original'],'Original source inventory count differs')
    positive=sum(bool(e.get('findings')) for e in source['original']['events'] if e['check']==check)
    require(positive==spec['positive'],'Original positives omitted')
    require(set(value['groups'])=={'public','components','cli','production'},'Contract family omitted')
    executions=0
    def collect(d):
        nonlocal executions
        if isinstance(d,dict):
            if 'programInputs' in d:
                executions+=1;require(bool(d['programInputs']),'Native program provenance missing')
                for p in d['programInputs']:
                    require(p['path'] in value['programs'] and value['programs'][p['path']]['sha256']==p['sha256'],'Native program differs')
            for child in d.values():collect(child)
        elif isinstance(d,list):
            for child in d:collect(child)
    def report(row):
        require(row['engineSha256']==engine,'Mixed engine cohort')
        require(not row.get('semanticFallbacks') and not row.get('hostProtocolCalls'),'Original detector or CLI executed')
        for n,digest in row.get('harnessFiles',{}).items():require(value['harness'][n]['sha256']==digest,'Mixed harness cohort')
        collect(row)
    for name,item in value['programs'].items():require(sha(item['text'].encode())==item['sha256'],'Stored program bytes differ')
    for name,item in value['harness'].items():require(sha(item['text'].encode())==item['sha256'],'Stored harness bytes differ')
    for family,reports in value['groups'].items():
        oracle=source[family];require(oracle['passed'] and oracle['sourcePin']==PIN,'Contract source failed')
        require(oracle['sourceSha256']==source['public']['sourceSha256'],'Mixed original linter sources')
        wanted={c['input']['id']:c for c in oracle['cases']};require(len(wanted)==spec[family],'Contract source cases omitted')
        seen={mode:[] for mode in MODES}
        for row in reports:
            report(row);require(row['oracle'] is False and row['passed'],'Native contract failed')
            require(row['baselineSha256']==sources[family]['sha256'] and row['sourceSha256']==oracle['sourceSha256'],'Wrong contract source recording')
            for c in row['cases']:
                identity=c['input']['id'];seen[row['executionMode']].append(identity);expected=wanted[identity]
                require(equal(c['input'],expected['input']) and equal(c['expected'],expected['expected']),'Contract input or oracle differs')
                require(c['passed'] and equal(c['actual'],expected['expected']),'Complete native result differs')
                if not c['native']:
                    error=PRE_QUERY_ERRORS.get(family,{}).get(identity)
                    framework=family=='components' and identity in {'extract-clean','extract-invalid'}
                    require((error is not None and c['expected'].get('error',{}).get('type')==error) or framework,
                            'Semantic contract never submitted native work')
                    if framework:require(c['expected']==dict(value=dict(kind='dict',items=[])),
                                         'Framework pre-query result differs')
        for identities in seen.values():require(Counter(identities)==Counter(wanted.keys()),'Contract cases omitted or repeated')
    inventory=json.loads(value['originalInventory']['text']);require(sha(value['originalInventory']['text'].encode())==value['originalInventory']['sha256'],'Inventory snapshot differs')
    require(inventory['sourcePin']==PIN,'Wrong independent source inventory')
    files={p for p,row in inventory['files'].items() if check in row['detectors']}
    owned={n for n in inventory['cases'] if n.split('::')[0] in files}
    require(owned==set(source['original']['test_outcomes']),'Original recording misses owned assertions')
    require(set(value['originalSuites'])==MODES,'Original API omitted')
    for mode,row in value['originalSuites'].items():
        report(row);require(row['oracle'] is False and row['exitCode']==0 and row['mode']==mode,'Original native suite failed')
        require(row['originalRecordingSha256']==sources['original']['sha256'],'Wrong lossless original recording')
        require(all(row['sourceFiles'].get(p)==inventory['files'][p]['sha256'] for p in files),'Original test source changed')
        require(row['sourceFiles'].get('scripts/lint_'+check+'.py')==source['public']['sourceSha256'],'Original linter source changed')
        require(not row['missingResults'] and not row.get('retainedCaseIds') and not row['casesWithoutNativeCalls'],'Original assertions delegated or omitted')
        require(set(row['cases'])==owned,'Owned original assertions omitted')
        for phases in row['cases'].values():require({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Original assertion failed or skipped')
        events=[e for e in source['original']['events'] if e['check']==check]
        require(len(events)==len(row['detectorObservations']),'Original detector observations omitted')
        for event,observation in zip(events,row['detectorObservations']):
            require((event['test'],event['entry'])==(observation['test'],observation['entry']),'Original observation order differs')
            require(equal(event['sourceResult'],observation['actualSourceResult']),'Original full DTO differs')
        require(row['policySha256'][check]==value['ruleSha256'],'Original suite ran a different rule')
    for mode in MODES:
        updates=value['updates'][mode];require(len(updates)==3,'Update sequence omitted')
        require(Counter(r['sequence'] for r in updates)==Counter(range(3)),'Update sequence repeated')
        steps=0
        for row in updates:
            report(row);require(row['complete'] and row['passed'] and row['mode']==mode,'Persistent update failed')
            require(row['baselineSha256']==sources['production']['sha256'],'Update oracle differs')
            for c in row['cases']:
                steps+=1;wanted=next(v for v in source['production']['cases'] if v['input']['id']==c['id'])
                require(c['passed'] and equal(c['expected'],wanted['expected']) and equal(c['actual'],wanted['expected']),'Persistent update result differs')
        require(steps==spec['updates'],'Update steps omitted')
        row=value['controls'][mode];report(row)
        require(row['baselineSha256']=={k:sources[k]['sha256'] for k in ['public','components','cli']},'Control source recording differs')
        require(row['complete'] and row['passed'] and row['mode']==mode,'Removal control failed')
        require(len(row['cases'])==spec['controls'] and len({c['id'] for c in row['cases']})==spec['controls'],'Removal controls omitted')
        for c in row['cases']:
            require(c['passed'] and not equal(c['expected'],c['actual']),'Ineffective removal control')
            require(sha(c['mutatedPolicy'].encode())==c['mutatedPolicySha256'],'Removal mutation differs')
            wanted=next(v for v in source[c['family']]['cases'] if v['input']['id']==c['input']['id'])
            require(equal(wanted['input'],c['input']) and equal(wanted['expected'],c['expected']),'Removal control oracle differs')
    return dict(casesPerAPI=spec,nativeExecutions=executions,check=check)


def pack(work,check):
    require(check in SPECS,'Unsupported rule family');m=read_json(ROOT/MANIFEST);entry=next(r for r in m['rules'] if r['check']==check)
    names=dict(public='classvar-source-expanded-fc6.json',components='classvar-components-source-fc6.json',cli='classvar-cli-source-aliases-fc6.json',production='classvar-production-source-fc6.json',original='classvar-original-source-fc6.json')
    def snapshot(path):return dict(text=path.read_text(),sha256=sha(path.read_bytes()))
    value=dict(format=FORMAT,check=check,fullMigrationAcceptance=False,ruleSha256=entry['ruleSha256'],engineSource=m['engineSource'],
        sources={k:snapshot(work/n) for k,n in names.items()},groups={},originalSuites={},updates={},controls={},programs={},harness={},
        originalInventory=snapshot(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json'))
    for family in ['public','components','cli','production']:value['groups'][family]=[read_json(p) for p in sorted(work.glob('classvar-final-'+family+'-*.json'))]
    for mode in MODES:
        value['originalSuites'][mode]=read_json(work/f'classvar-final-original-{mode}.json')
        value['updates'][mode]=[read_json(work/f'classvar-final-updates-{mode}-{i}.json') for i in range(3)]
        value['controls'][mode]=read_json(work/f'classvar-final-controls-{mode}.json')
    mutants={c['mutatedPolicySha256']:c['mutatedPolicy'] for r in value['controls'].values() for c in r['cases']}
    def walk(d):
        if isinstance(d,dict):
            for p in d.get('programInputs',[]):
                path=ROOT/p['path'];raw=path.read_bytes() if path.is_file() else mutants[p['sha256']].encode();require(sha(raw)==p['sha256'],'Consumed rule changed');value['programs'][p['path']]=dict(sha256=p['sha256'],text=raw.decode())
            for n,h in d.get('harnessFiles',{}).items():
                raw=(ROOT/n).read_bytes();require(sha(raw)==h,'Consumed harness changed');value['harness'][n]=dict(sha256=h,text=raw.decode())
            for child in d.values():walk(child)
        elif isinstance(d,list):
            for child in d:walk(child)
    for key in ['groups','originalSuites','updates','controls']:walk(value[key])
    value['harness'][Path(__file__).relative_to(ROOT).as_posix()]=snapshot(Path(__file__))
    value['summary']=validate(value);return value


def admit(value,checkpoint):
    summary=validate(value);m=read_json(ROOT/MANIFEST);check=value['check'];entry=next(r for r in m['rules'] if r['check']==check)
    require(m['engineSource']['cliArtifact']==value['engineSource']['cliArtifact'],'Engine changed before admission')
    require(entry['ruleSha256']==value['ruleSha256'],'Rule changed before admission')
    for n,p in value['programs'].items():
        if not n.startswith('.rerail/'):require(sha((ROOT/n).read_bytes())==p['sha256'],'Current program changed')
    ref=dict(path=checkpoint.relative_to(ROOT).as_posix(),sha256=sha(checkpoint.read_bytes()));reports=[]
    for mode,r in value['originalSuites'].items():
        path=ROOT/f'gate/data-layer/fixtures/host/data-layer-port/rr-{check.replace("_","-")}-family-native-{mode}.json'
        write_json(path,dict(r,checkpoint=ref,calls=[]));reports.append(dict(path=path.relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes())))
    inv=ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json';spec=SPECS[check]
    entry.update(status='full-contract-passed',evidence=dict(assertions=spec['original'],positives=spec['positive'],failures=0,skipped=0,semanticFallbacks=0,
        ruleSha256=entry['ruleSha256'],engineCommit=value['engineSource']['cliArtifact']['engineCommit'],corpusSha256=value['sources']['public']['sha256'],sourceCommit=PIN,
        engineSha256=value['engineSource']['cliArtifact']['sha256'],originalInventory=dict(path=inv.relative_to(ROOT).as_posix(),sha256=sha(inv.read_bytes())),reports=reports,
        acceptanceCheckpoints=[ref],supplemental=summary,scope='All owned original assertions, public APIs, plugin and AST facts, supplied facts, CLI, production, persistent updates and removal controls through both native interfaces.'))
    entry.setdefault('migrationProgress',{}).update(status='full-contract-passed',verifiedNativeExecutions=summary['nativeExecutions'])
    require_complete([entry]);write_json(ROOT/MANIFEST,m);return summary


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate','admit']);p.add_argument('path',type=Path);p.add_argument('--check',default='classvar_check');p.add_argument('--out',type=Path);a=p.parse_args()
    if a.command=='pack':
        v=pack(a.path.absolute(),a.check);a.out.write_bytes(gzip.compress(json.dumps(v,sort_keys=True,separators=(',',':')).encode(),mtime=0))
    else:v=json.loads(gzip.decompress(a.path.read_bytes()))
    print(json.dumps(admit(v,a.path.absolute()) if a.command=='admit' else validate(v),sort_keys=True))
