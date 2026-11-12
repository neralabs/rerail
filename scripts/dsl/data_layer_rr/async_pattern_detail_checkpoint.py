"""Retain complete async helper and component parity without granting rule admission."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
import shutil
import contract_checkpoint
from migration import ROOT,PIN,read_json,sha
from scanner_checkpoint import walk
from validate_supplied_facts_checkpoint import require,equal

FORMAT='rerail.async-pattern-detail-progress/v1'
MODES=['snapshot','workspace']


def validate(value):
    require(value['format']==FORMAT,'Wrong detailed async-pattern checkpoint')
    summary=contract_checkpoint.validate(dict(value,format='rerail.recorded-contract-progress/v1'))
    require(summary['check']=='async_patterns' and summary['recordedCasesPerAPI']==137 and summary['originalCasesPerAPI']==11 and summary['originalPositiveObservationsPerAPI']==8,'Incomplete detailed inventory')
    item=value['components']['source'];source=item['value'];engine=value['engineSource']['cliArtifact']['sha256']
    require(sha(item['text'].encode())==item['sha256'] and equal(json.loads(item['text']),source),'Changed component source snapshot')
    require(source['sourcePin']==PIN and source['mode']=='source' and source['passed'] and source['totalCases']==162 and source['sourceSha256']==value['source']['sourceSha256'],'Invalid component source')
    oracle={r['input']['id']:r for r in source['cases']};require(len(oracle)==162,'Omitted component source case')
    extra=0
    for mode in MODES:
        reports=value['components']['reports'][mode];rows=[r for report in reports for r in report['cases']]
        require(Counter(r['input']['id'] for r in rows)==Counter(oracle.keys()),'Omitted or repeated component case')
        for report in reports:
            require(report['passed'] and report['executionMode']==mode and report['engineSha256']==engine and report['oracle'] is False and not report['semanticFallbacks'],'Failed component engine or API')
            require(report['baselineSha256']==item['sha256'] and report['sourceSha256']==source['sourceSha256'],'Mixed component oracle')
        for row in rows:
            expected=oracle[row['input']['id']]
            require(row['passed'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Different complete component value')
            if not row['native']:
                require(row['input']['id']=='extract-invalid' and row['input']['source']=='async def broken(:\n','Component inference did not execute natively')
                parts=dict(row['expected']['value']['items']);require(parts['facts']==dict(kind='dict',items=[]) and parts['errors']['items'] and parts['errors']['items'][0][1].startswith('SyntaxError:'),'Invalid framework parse boundary')
            extra+=len(row['native'])
        for row in walk(reports):
            for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed detailed harness')
            for p in row.get('programInputs',[]):
                require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound consumed detailed program')
    summary.update(componentCasesPerAPI=162,nativeExecutions=summary['nativeExecutions']+extra)
    return summary


def pack(work):
    stage=work/'details-work';stage.mkdir(exist_ok=True)
    for src,dst in [('source.json','source.json'),('helpers-source.json','scanner-source.json')]:shutil.copyfile(work/src,stage/dst)
    for mode in MODES:
        shutil.copyfile(work/f'original-components-{mode}.json',stage/f'original-accepted-{mode}.json')
        for part in range(16):shutil.copyfile(work/f'helpers-coherent-{mode}-{part}.json',stage/f'scanner-accepted-{mode}-{part}.json')
    value=contract_checkpoint.pack(stage,'async_patterns');path=work/'components/source.json';text=path.read_text()
    value.update(format=FORMAT,components=dict(source=dict(text=text,sha256=sha(text.encode()),value=json.loads(text)),reports={m:[read_json(work/f'components/coherent-{m}-{i}.json') for i in range(16)] for m in MODES}),
        remaining=['File/directory helpers and CLI/ratchet integration require native acceptance.','Historical inputs, retained updates, behavior-removal controls and strict full admission remain required.'])
    for row in walk(value['components']['reports']):
        for p in row.get('programInputs',[]):
            text=(ROOT/p['path']).read_text() if p['sha256'] else None;require((sha(text.encode()) if text is not None else None)==p['sha256'],'Program changed after detailed execution');value['programs'][p['path']]=dict(text=text,sha256=p['sha256'])
        for path,digest in row.get('harnessFiles',{}).items():
            text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Detailed harness changed after execution');value['harness'][path]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate']);p.add_argument('path',type=Path);p.add_argument('--work',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.work);a.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
