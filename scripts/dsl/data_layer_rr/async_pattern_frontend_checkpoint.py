"""Retain exact async-pattern filesystem and CLI parity without granting admission."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha
from scanner_checkpoint import walk
from validate_supplied_facts_checkpoint import require,equal

FORMAT='rerail.async-pattern-frontend-progress/v1'
COUNTS=dict(files=28,cli=40)
PARTS=dict(files=8,cli=40)
MODES=['snapshot','workspace']


def validate(value):
    require(value['format']==FORMAT and value['admitted'] is False and value['fullMigrationAcceptance'] is False and value['remaining'],'Invalid frontend progress scope')
    for section in ['programs','harness']:
        for item in value[section].values():require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Changed stored frontend bytes')
    executions=0;engine=value['engineSource']['cliArtifact']['sha256']
    for group,count in COUNTS.items():
        item=value['groups'][group];source=item['source'];oracle=source['value']
        require(sha(source['text'].encode())==source['sha256'] and equal(json.loads(source['text']),oracle),'Changed frontend source snapshot')
        require(oracle['sourcePin']==PIN and oracle['passed'] and oracle['mode']=='source' and oracle['totalCases']==count,'Invalid frontend source')
        wanted={r['input']['id']:r for r in oracle['cases']};require(len(wanted)==count,'Missing frontend source case')
        for mode in MODES:
            reports=item['reports'][mode];rows=[r for report in reports for r in report['cases']]
            require(Counter(r['input']['id'] for r in rows)==Counter(wanted.keys()),'Missing or repeated frontend case')
            for report in reports:
                require(report['passed'] and report['engineSha256']==engine and report['executionMode']==mode and report['oracle'] is False and not report['semanticFallbacks'],'Failed frontend native execution')
                require(report['sourceSha256']==oracle['sourceSha256'] and report['baselineSha256']==source['sha256'],'Mixed frontend source recording')
            for row in rows:
                expected=wanted[row['input']['id']]
                require(row['passed'] and row['native'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Changed complete frontend value')
                executions+=len(row['native'])
            for row in walk(reports):
                for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed frontend harness')
                if 'programInputs' in row:
                    require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed frontend main rule')
                    for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Mixed frontend program')
    return dict(admitted=False,fullMigrationAcceptance=False,casesPerAPI=COUNTS,nativeExecutions=executions)


def pack(work):
    m=read_json(ROOT/MANIFEST);rule=next(r for r in m['rules'] if r['check']=='async_patterns')
    value=dict(format=FORMAT,admitted=False,fullMigrationAcceptance=False,engineSource=m['engineSource'],rulePath=rule['rulePath'],ruleSha256=rule['ruleSha256'],groups={},programs={},harness={},remaining=['Recheck all rule families on one final cohort.','Historical inputs, retained updates, behavior-removal controls and strict admission remain required.'])
    for group in COUNTS:
        path=work/(group+'-source.json');text=path.read_text()
        reports={mode:[read_json(work/(f'{group}-'+('second' if mode=='snapshot' else 'workspace')+f'-{i}.json')) for i in range(PARTS[group])] for mode in MODES}
        value['groups'][group]=dict(source=dict(text=text,sha256=sha(text.encode()),value=json.loads(text)),reports=reports)
        for row in walk(reports):
            for p in row.get('programInputs',[]):
                text=(ROOT/p['path']).read_text() if p['sha256'] is not None else None;require((sha(text.encode()) if text is not None else None)==p['sha256'],'Frontend program changed after execution');value['programs'][p['path']]=dict(text=text,sha256=p['sha256'])
            for path,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Frontend harness changed after execution');value['harness'][path]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate']);p.add_argument('path',type=Path);p.add_argument('--work',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.work);a.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
