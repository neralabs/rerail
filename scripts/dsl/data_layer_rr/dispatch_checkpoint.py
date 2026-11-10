"""Validate complete dispatch progress receipts without treating them as admission."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from migration import ROOT,MANIFEST,PIN,read_json,sha
from validate_supplied_facts_checkpoint import equal,require

COUNTS={'original':49,'scanner':298,'finding':47,'supplied':222,'repository':43,'updates':17,'cli':48,'signing':31}


def validate(value):
    require(value['format']=='rerail.dispatch-progress/v4','Wrong dispatch checkpoint format')
    require(value['admitted'] is False and value['pending'],'Progress must retain its admission gaps')
    for kind,source in value['sourceRecordings'].items():
        require(sha(value['sourceTexts'][kind].encode())==value['sourceHashes'][kind],'Source recording bytes differ')
        require(equal(json.loads(value['sourceTexts'][kind]),source),'Source recording contents differ')
        require(source['sourcePin']==PIN and source.get('passed',source.get('exitCode')==0),'Invalid source oracle')
    engine=value['build']['sha256'];seen={mode:{k:[] for k in COUNTS} for mode in ['snapshot','workspace']};executions=0
    for section in ['programs','harness']:
        for entry in value[section].values():require(sha(entry['text'].encode())==entry['sha256'],'Snapshot bytes differ')
    def walk(v):
        nonlocal executions
        if isinstance(v,dict):
            if 'programInputs' in v:
                executions+=1;require(bool(v['programInputs']),'Missing native program provenance')
                for entry in v['programInputs']:
                    key=entry['path']+'@'+entry['sha256']
                    require(key in value['programs'] and value['programs'][key]['sha256']==entry['sha256'],'Consumed program differs')
            for name,digest in v.get('harnessFiles',{}).items():require(value['harness'].get(name,{}).get('sha256')==digest,'Consumed harness differs')
            for child in v.values():walk(child)
        elif isinstance(v,list):
            for child in v:walk(child)
    for name,report in value['reports'].items():
        kind,mode=value['reportKinds'][name];require(kind in COUNTS and mode in seen,'Unexpected report group')
        require(report['engineSha256']==engine,'Mixed engine artifacts')
        require(report.get('sourcePin',PIN)==PIN and not report.get('semanticFallbacks'),'Source pin differs or Python semantics executed')
        require(report.get('passed',report.get('exitCode')==0),'Failed progress report')
        if kind in ['cli','signing','updates']:require(report['complete'],'Interrupted contract report')
        walk(report)
        if kind=='original':
            require(report['observationsMatch'] and report['exitCode']==0,'Original observations differ')
            oracle=value['sourceRecordings']['original']
            require(equal(report['observations'],oracle['observations']) and equal(report['sourceFiles'],oracle['sourceFiles']),'Original source result differs')
            require(Counter(report['cases'].keys())==Counter(oracle['cases'].keys()),'Original test coverage differs')
            for identity,phases in report['cases'].items():
                require({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Skipped or failed original assertion')
                seen[mode][kind].append(identity)
            continue
        oracle_kind='repository' if kind=='updates' else kind
        require(report['baselineSha256']==value['sourceHashes'][oracle_kind],'Source recording hash differs')
        oracle={r['input']['id']:r for r in value['sourceRecordings'][oracle_kind]['cases']}
        for row in report['cases']:
            require(row['passed'] and equal(row['expected'],row['actual']),'Complete typed result differs')
            source_row=oracle[row['id'] if kind=='updates' else row['input']['id']]
            require(equal(row['expected'],source_row['expected']),'Expected result differs from its oracle')
            if kind!='updates':require(equal(row['input'],source_row['input']),'Case input differs from its oracle')
            identity=(report['sequence'],row['step']) if kind=='updates' else row['input']['id']
            seen[mode][kind].append(identity)
    for mode,groups in seen.items():
        for kind,ids in groups.items():
            require(len(ids)==COUNTS[kind] and len(set(ids))==COUNTS[kind],f'{mode} {kind}: omitted or repeated cases')
            if kind not in ['original','updates']:
                require(Counter(ids)==Counter(r['input']['id'] for r in value['sourceRecordings'][kind]['cases']),f'{mode} {kind}: source coverage differs')
    return dict(casesPerAPI=COUNTS,nativeExecutions=executions,admitted=False)


def pack(work):
    reports={};kinds={}
    def add(path,kind,mode):
        reports[path.name]=read_json(path);kinds[path.name]=[kind,mode]
    for mode in ['snapshot','workspace']:
        for kind in ['original','finding','supplied']:add(work/f'demand-green-{kind}-{mode}.json',kind,mode)
        for i in range(3 if mode=='snapshot' else 2):add(work/f'demand-green-scanner-{mode}-{i}.json','scanner',mode)
        for i in range(8):
            name=f'repository-demand-v2-workspace-{i}.json' if mode=='workspace' and i in [0,4] else f'demand-green-repository-{mode}-{i}.json'
            add(work/name,'repository',mode)
        for i in range(3):
            name='updates-demand-v2-workspace-0.json' if mode=='workspace' and i==0 else f'demand-green-updates-{mode}-{i}.json'
            add(work/name,'updates',mode)
        for i in range(24):add(work/f'demand-green-cli-{mode}-{i}.json','cli',mode)
        for i in range(12):add(work/f'demand-green-signing-{mode}-{i}.json','signing',mode)
    names=dict(original='original-source.json',scanner='scope-scanner-source.json',finding='finding-source.json',supplied='supplied-rollup-source.json',repository='repository-full-source.json',cli='cli-source-finald03.json',signing='signing-source-finald03.json')
    sources={kind:read_json(work/name) for kind,name in names.items()}
    value=dict(format='rerail.dispatch-progress/v4',admitted=False,reports=reports,reportKinds=kinds,sourceRecordings=sources,sourceTexts={kind:(work/name).read_text() for kind,name in names.items()},sourceHashes={kind:sha((work/name).read_bytes()) for kind,name in names.items()},programs={},harness={},
        engineSource=read_json(ROOT/MANIFEST)['engineSource'],build=read_json(ROOT/'.rerail/rr-migration-20261106/grammar-render-methods/cli-dispatch-finald03-repair-build.json'),
        pending=['Production inventory I/O error contracts','Scanner behavior-removal controls','Full original-inventory admission'])
    def collect(v):
        if isinstance(v,dict):
            for item in v.get('programInputs',[]):
                raw=(ROOT/item['path']).read_bytes();require(sha(raw)==item['sha256'],'Program changed during acceptance')
                value['programs'][item['path']+'@'+item['sha256']]=dict(path=item['path'],sha256=item['sha256'],text=raw.decode())
            for name,digest in v.get('harnessFiles',{}).items():
                raw=(ROOT/name).read_bytes();require(sha(raw)==digest,'Harness changed during acceptance')
                value['harness'][name]=dict(sha256=digest,text=raw.decode())
            for child in v.values():collect(child)
        elif isinstance(v,list):
            for child in v:collect(child)
    collect(reports)
    path=Path(__file__);value['harness'][path.relative_to(ROOT).as_posix()]=dict(sha256=sha(path.read_bytes()),text=path.read_text())
    path=ROOT/'test/scripts/data_layer_rr/test_dispatch_checkpoint.py';value['harness'][path.relative_to(ROOT).as_posix()]=dict(sha256=sha(path.read_bytes()),text=path.read_text())
    value['summary']=validate(value);return value


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate']);p.add_argument('path',type=Path);p.add_argument('--out',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.path);a.out.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,sort_keys=True).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))


if __name__=='__main__':main()
