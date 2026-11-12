"""Bind complete shared-helper parity and effective controls without admitting the whole rule."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha
from structural_helper_controls import EDITS,POLICY,LIBRARY
from validate_supplied_facts_checkpoint import equal,require

MODES=['snapshot','workspace']


def validate(value):
    require(value['format']=='rerail.structural-helper-complete/v1'and value['helperComplete']is True,'Wrong helper proof')
    require(value['fullRuleMigrationAcceptance']is False and value['fullMigrationAcceptance']is False,'Helper cannot admit whole rule')
    raw=value['source']['text'].encode();source=json.loads(raw)
    require(sha(raw)==value['source']['sha256']and equal(source,value['source']['value']),'Changed helper source recording')
    require(source['passed']is True and source['sourcePin']==PIN and len(source['cases'])==117,'Incomplete helper source corpus')
    expected={r['input']['id']:r for r in source['cases']};require(len(expected)==117,'Repeated source input')
    require({'forbidden','attribute','duplicates','first-core-wins','first-core-bad','lines-unicode-lines','combined'}<=set(expected),'Missing helper witnesses')
    engine=value['engineSha256'];executions=0
    for mode in MODES:
        reports=value['reports'][mode]
        require(Counter(r['part']for r in reports)==Counter(range(8)),'Missing helper shard')
        seen=[]
        for report in reports:
            require(report['passed']is True and report['executionMode']==mode and report['parts']==8 and report['sourcePin']==PIN and
                    report['engineSha256']==engine and report['baselineSha256']==sha(raw)and not report['semanticFallbacks'],'Failed helper shard')
            for row in report['cases']:
                key=row['input']['id'];seen.append(key);original=expected[key]
                require(equal(row['input'],original['input'])and equal(row['expected'],original['expected'])and
                        row['passed']is True and equal(row['actual'],original['expected']),'Changed helper comparison')
                require(row['native']or 'error'in original['expected'],'No native helper execution')
                for call in row['native']:
                    require(call['programInputs']and isinstance(call['phases'],list),'Missing consumed helper program')
                    require(any(p['path']==POLICY for p in call['programInputs']),'Wrong helper policy')
                    for p in call['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Mixed helper program')
                    executions+=1
            for path,digest in report['harnessFiles'].items():require(value['harness'][path]['sha256']==digest,'Mixed helper adapter')
        require(Counter(seen)==Counter(expected.keys()),'Dropped helper input')
    controls=value['controls']
    require(Counter((r['mode'],r['control'])for r in controls)==Counter((m,n)for m in MODES for n in EDITS),'Missing helper mutation')
    for row in controls:
        identity,before,after=EDITS[row['control']];original=expected[identity]
        require(row['passed']is True and row['complete']is True and row['engineSha256']==engine and row['sourcePin']==PIN and
                row['baselineSha256']==sha(raw)and not row['semanticFallbacks'],'Failed helper mutation')
        require(equal(row['input'],original['input'])and equal(row['expected'],original['expected'])and
                not equal(row['actual'],original['expected'])and 'error'not in row['actual']and 'unexpected'not in row['actual'],'Ineffective or invalid helper mutation')
        main=value['programs'][POLICY]['text'].replace('#include "data-layer/structural-connection-helper.rr"','#include "control.rr"')
        library=value['programs'][LIBRARY]['text'];require(library.count(before)==1,'Changed helper mutation target')
        copies={sha(t.encode()):t for t in [main,library.replace(before,after,1)]}
        require(row['programCopies']==copies and row['native'],'Unrelated or unexecuted helper mutation')
        for call in row['native']:
            require(call['programInputs']and isinstance(call['phases'],list),'Unbound mutation execution')
            require(any(p['sha256']in copies for p in call['programInputs']),'Mutation program not consumed')
            for p in call['programInputs']:
                require(p['sha256']in copies or value['programs'][p['path']]['sha256']==p['sha256'],'Unbound control library')
            executions+=1
        for path,digest in row['harnessFiles'].items():require(value['harness'][path]['sha256']==digest,'Mixed mutation adapter')
    for area in ['programs','harness']:
        for item in value[area].values():
            if item['sha256']is None:require(area=='programs'and item['text']is None and item.get('exists')is False,'Invalid absent include dependency')
            else:require(sha(item['text'].encode())==item['sha256'],'Changed bound text')
    return dict(helperComplete=True,casesPerAPI=117,controls=len(controls),nativeExecutions=executions,fullRuleMigrationAcceptance=False)


def pack(work):
    path=work/'structural-complete-source.json';raw=path.read_bytes()
    reports={m:[read_json(work/f'structural-complete-{m}-{i}.json')for i in range(8)]for m in MODES}
    controls=[read_json(work/f'structural-controls-final-{m}-{i}.json')for m in MODES for i in range(len(EDITS))]
    value=dict(format='rerail.structural-helper-complete/v1',helperComplete=True,fullRuleMigrationAcceptance=False,fullMigrationAcceptance=False,
        source=dict(text=raw.decode(),sha256=sha(raw),value=json.loads(raw)),reports=reports,controls=controls,
        engineSource=read_json(ROOT/MANIFEST)['engineSource'],engineSha256=reports['snapshot'][0]['engineSha256'],programs={},harness={})
    for report in [*reports['snapshot'],*reports['workspace'],*controls]:
        for name,digest in report['harnessFiles'].items():
            text=(ROOT/name).read_text();require(sha(text.encode())==digest,'Moving helper harness')
            value['harness'][name]=dict(text=text,sha256=digest)
        calls=report.get('native',[])if 'control'in report else [c for r in report['cases']for c in r['native']]
        copies=report.get('programCopies',{})
        for call in calls:
            for p in call['programInputs']:
                if p['sha256']in copies:continue
                if p['sha256']is None:
                    require(not (ROOT/p['path']).exists(),'Absent include dependency appeared')
                    value['programs'][p['path']]=dict(text=None,sha256=None,exists=False)
                    continue
                text=(ROOT/p['path']).read_text();require(sha(text.encode())==p['sha256'],'Moving helper program')
                value['programs'][p['path']]=dict(text=text,sha256=p['sha256'])
    value['summary']=validate(value);return value


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--work',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    value=pack(a.work);a.out.write_bytes(gzip.compress(json.dumps(value,sort_keys=True).encode(),mtime=0));print(value['summary'])
