"""Retain coherent boundary components, owner-aware production and public scanner parity."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
import shutil
import boundary_component_checkpoint
from migration import ROOT,PIN,read_json,sha
from scanner_checkpoint import walk
from validate_supplied_facts_checkpoint import require,equal

FORMAT='rerail.boundary-scanner-progress/v1'
COUNTS=dict(public=32,production=22)
PARTS=dict(public=8,production=4)
MODES=('snapshot','workspace')


def validate(value):
    require(value['format']==FORMAT,'Wrong boundary scanner checkpoint')
    summary=boundary_component_checkpoint.validate(dict(value,format=boundary_component_checkpoint.FORMAT))
    engine=value['engineSource']['cliArtifact']['sha256'];extra=0
    for group,count in COUNTS.items():
        item=value['scanners'][group];source=item['source'];oracle=source['value']
        require(sha(source['text'].encode())==source['sha256'] and equal(json.loads(source['text']),oracle),'Changed scanner source bytes')
        require(oracle['sourcePin']==PIN and oracle['passed'] and oracle['mode']=='source' and oracle['totalCases']==count and oracle['sourceSha256']==value['source']['sourceSha256'],'Invalid scanner source')
        wanted={r['input']['id']:r for r in oracle['cases']};require(len(wanted)==count,'Missing scanner source case')
        for mode in MODES:
            reports=item['reports'][mode];rows=[r for report in reports for r in report['cases']]
            require(Counter(r['input']['id'] for r in rows)==Counter(wanted.keys()),'Missing or repeated scanner case')
            for report in reports:
                require(report['passed'] and report['executionMode']==mode and report['engineSha256']==engine and report['oracle'] is False and not report['semanticFallbacks'],'Failed scanner engine or API')
                require(report['sourceSha256']==oracle['sourceSha256'] and report['baselineSha256']==source['sha256'],'Mixed scanner source recording')
            for row in rows:
                expected=wanted[row['input']['id']]
                require(row['passed'] and row['native'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Changed complete scanner result')
            for row in walk(reports):
                for path,digest in row.get('harnessFiles',{}).items():require(value['harness'][path]['sha256']==digest,'Mixed scanner harness')
                if 'programInputs' not in row:continue
                extra+=1
                require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed main scanner rule')
                for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound consumed scanner program')
    summary.update(scannerCasesPerAPI=COUNTS,nativeExecutions=summary['nativeExecutions']+extra)
    return summary


def pack(work):
    stage=work/'scanner-checkpoint-stage';stage.mkdir(exist_ok=True)
    for name in ['source.json','components-source.json']:shutil.copyfile(work/name,stage/name)
    for mode in MODES:
        shutil.copyfile(work/f'coherent-original-{mode}.json',stage/f'original-accepted-{mode}.json')
        for i in range(16):shutil.copyfile(work/f'coherent-components-{mode}-{i}.json',stage/f'components-accepted-{mode}-{i}.json')
    value=boundary_component_checkpoint.pack(stage)
    value.update(format=FORMAT,scanners={},remaining=['Complete native rendering and CLI/ratchet administration.','Complete earlier inputs, retained edits, valid removal controls and strict full admission.'])
    for group in COUNTS:
        text=(work/(group+'-source.json')).read_text();prefix='public-accepted' if group=='public' else 'coherent-production'
        reports={m:[read_json(work/f'{prefix}-{m}-{i}.json') for i in range(PARTS[group])] for m in MODES}
        value['scanners'][group]=dict(source=dict(text=text,sha256=sha(text.encode()),value=json.loads(text)),reports=reports)
        for row in walk(reports):
            for p in row.get('programInputs',[]):
                text=(ROOT/p['path']).read_text() if p['sha256'] is not None else None
                require((sha(text.encode()) if text is not None else None)==p['sha256'],'Scanner program changed after execution');value['programs'][p['path']]=dict(text=text,sha256=p['sha256'])
            for path,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Scanner harness changed after execution');value['harness'][path]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate']);p.add_argument('path',type=Path);p.add_argument('--work',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.work);a.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as f:value=json.load(f)
    print(json.dumps(validate(value),sort_keys=True))
