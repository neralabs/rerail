"""Retain async-pattern production and supplemental parity without granting admission."""
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

CHECK='async_patterns'
FORMAT='rerail.async-pattern-owner-progress/v1'
COUNTS=dict(api=158,report=136)
CORPORA=dict(api='rr-fresh-async-patterns-cases.json',report='rr-async-patterns-reporter-cases.json')


def validate(value):
    require(value['format']==FORMAT,'Wrong async-pattern progress format')
    summary=contract_checkpoint.validate(dict(value,format='rerail.recorded-contract-progress/v1'))
    require(summary['check']==CHECK and summary['originalCasesPerAPI']==11 and summary['originalPositiveObservationsPerAPI']==8 and summary['recordedCasesPerAPI']==19,'Incomplete original or production inventory')
    engine=value['engineSource']['cliArtifact']['sha256'];extra_executions=0
    for group,count in COUNTS.items():
        item=value['supplemental'][group];source=item['source'];oracle=source['value']
        require(sha(source['text'].encode())==source['sha256'] and equal(json.loads(source['text']),oracle),'Changed supplemental source snapshot')
        require(oracle['sourcePin']==PIN and oracle['execution']=='offline-source' and oracle['totalCases']==count and len(oracle['results'])==count,'Incomplete supplemental source')
        require(oracle['sourceFiles']['scripts/lint_async_patterns.py']==value['source']['sourceSha256'],'Mixed supplemental source modules')
        corpus=item['corpus'];require(sha(corpus['text'].encode())==corpus['sha256']==oracle['casesSha256'],'Changed supplemental inputs')
        expected={r['id']:r for r in oracle['results']};inputs=json.loads(corpus['text'])['cases']
        require(len(expected)==count and Counter(r['id'] for r in inputs)==Counter(expected.keys()) and all(r['status']=='passed' for r in expected.values()),'Omitted or repeated supplemental source case')
        reports=item['reports'];rows=[r for report in reports for r in report['results']]
        require(Counter(r['id'] for r in rows)==Counter(expected.keys()),'Omitted or repeated supplemental native case')
        extra_executions+=sum(len(report['calls']) if group=='api' else len(report['results']) for report in reports)
        for report in reports:
            require(report['engineSha256']==engine and report['oracleSha256']==source['sha256'] and report['casesSha256']==corpus['sha256'] and report['execution']=='native-snapshot','Mixed supplemental engine or oracle')
            require(not report.get('semanticFallbacks') and not report.get('sourceDetectorModules'),'Source detector execution in native supplemental run')
            if group=='api':
                require(report['status']=='completed' and report['sourceFiles']==oracle['sourceFiles'],'Incomplete supplemental API run')
                require(report['calls'] and all(c['status']=='passed' for c in report['calls']),'Failed supplemental API execution')
            else:
                require('activeCaseId' not in report,'Incomplete supplemental reporter run')
                require(report.get('executionIsolation')=='One owned cache per reporter run; reused across all cases','Shared reporter cache')
        for row in rows:require(row['status']=='passed' and equal(row['value'],expected[row['id']]['value']),'Changed complete supplemental value')
        for row in walk(reports):
            for name,digest in row.get('harnessFiles',{}).items():require(value['harness'][name]['sha256']==digest,'Mixed supplemental harness')
            if 'programInputs' not in row:continue
            require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed supplemental rule revision')
            for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound consumed supplemental program')
    summary['supplementalCasesSnapshot']=COUNTS
    summary['nativeExecutions']+=extra_executions
    return summary


def pack(work):
    shutil.copyfile(work/'production-source.json',work/'scanner-source.json')
    for mode in ['snapshot','workspace']:
        shutil.copyfile(work/f'original-owner-{mode}.json',work/f'original-accepted-{mode}.json')
        for part in range(4):shutil.copyfile(work/f'production-owner-{mode}-{part}.json',work/f'scanner-accepted-{mode}-{part}.json')
    value=contract_checkpoint.pack(work,CHECK)
    value.update(format=FORMAT,supplemental={},remaining=['Collector callbacks, registry and complete helper APIs remain required.','Complete raw supplied-fact and CLI/ratchet contracts remain required.','Historical inputs, retained updates, behavior-removal controls and strict full admission remain required.'])
    for group in COUNTS:
        path=work/(group+'-source.json');text=path.read_text();corpus=(ROOT/'gate/data-layer/fixtures/host/data-layer-port'/CORPORA[group]).read_text()
        prefix='api-final' if group=='api' else 'report-owned'
        reports=[read_json(work/f'{prefix}-{i}.json') for i in range(16)]
        value['supplemental'][group]=dict(source=dict(text=text,sha256=sha(text.encode()),value=json.loads(text)),corpus=dict(text=corpus,sha256=sha(corpus.encode())),reports=reports)
        for row in walk(reports):
            for p in row.get('programInputs',[]):
                text=(ROOT/p['path']).read_text() if p['sha256'] is not None else None
                require((sha(text.encode()) if text is not None else None)==p['sha256'],'Supplemental program changed after execution')
                value['programs'][p['path']]=dict(text=text,sha256=p['sha256'])
            for name,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/name).read_text();require(sha(text.encode())==digest,'Supplemental harness changed after execution')
                value['harness'][name]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate']);p.add_argument('path',type=Path);p.add_argument('--work',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.work);a.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
