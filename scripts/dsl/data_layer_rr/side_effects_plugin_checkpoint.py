"""Retain complete supplied-fact parity, including failures that drove the port."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT, PIN, MANIFEST, read_json, sha
from side_effects_event_checkpoint import calls, retained_bytes
from validate_supplied_facts_checkpoint import equal, require

COUNTS={'plugin':97,'values':7,'demand':3,'scanner':35,'incremental':17,'directory':15}
MODES=('snapshot','workspace')


def reports(value):
    yield from value['originals'].values()
    for group in value['contracts'].values():
        yield group['source']
        for mode in MODES:yield from group[mode]
    for key in ['source','native','pathOrder','rootJoin','demand']:yield value['tdd'][key]


def validate(value):
    require(value['format']=='rerail.side-effects-plugin-progress/v1'and value['sourcePin']==PIN,'Wrong plugin progress scope')
    require(value['fullMigrationAcceptance']is False,'Plugin parity is not full rule admission')
    for family,count in COUNTS.items():
        group=value['contracts'][family];source=group['source']
        require(source['passed']and source['sourcePin']==PIN and len(source['cases'])==count,'Incomplete source corpus: '+family)
        expected={json.dumps(r['input'],sort_keys=True):r['expected']for r in source['cases']}
        require(len(expected)==count,'Duplicate source case')
        baseline=sha((json.dumps(source,indent=2,ensure_ascii=False)+'\n').encode())
        for mode in MODES:
            seen=set()
            for report in group[mode]:
                require(report['passed']and not report['semanticFallbacks'],'Native contract failed')
                require(report['engineSha256']==value['build']['sha256']and report['executionMode']==mode,'Wrong engine or API')
                require(report['sourceSha256']==source['sourceSha256']and report['baselineSha256']==baseline,'Wrong source recording')
                for row in report['cases']:
                    key=json.dumps(row['input'],sort_keys=True)
                    require(key not in seen,'Duplicate native case')
                    require(key in expected and row['passed']and equal(row['expected'],expected[key])and equal(row['actual'],expected[key]),'Complete native observation differs')
                    seen.add(key)
            require(seen==set(expected),'Native case omitted: '+family)
            if family=='demand':
                batch=next(r for report in group[mode]for r in report['cases']if r['input']['id']=='whole-positive-batch')
                require(len(batch['actual']['value'])==100 and len(batch['native'])==2,'Whole positive batch was split or omitted')
                for code in ['PSE_PLUGIN_ROW','PSE_PLUGIN_FINDING']:
                    require(sum(r['code']==code for call in batch['native']for r in call['diagnostics'])==100,'Native batch witness omitted')
    source=value['originals']['source']
    require(source['exitCode']==0 and len(source['collected'])==7,'Incomplete unchanged suite')
    for mode in MODES:
        report=value['originals'][mode]
        require(report['engineSha256']==value['build']['sha256']and report['executionMode']==mode,'Wrong original engine or API')
        require(report['exitCode']==0 and not report['semanticFallbacks'],'Original assertion failed')
        for key in ['collected','cases','observations','sourceFiles']:require(equal(report[key],source[key]),'Original observation omitted or changed')
        require(set(report['cases'])==set(source['collected']),'Original assertion omitted')
    require(not value['tdd']['native']['passed']and len(value['tdd']['native']['cases'])==97,'Missing pre-port failures')
    for key,count in [('pathOrder',1),('rootJoin',2)]:
        require(sum(not r['passed']for r in value['tdd'][key]['cases'])==count,'Missing exact path failure witnesses')
    require(sum(not r['passed']for r in value['tdd']['demand']['cases'])==2,'Missing unread-field failure witnesses')
    retained=set()
    for item in value['snapshots']:
        key=item['path'],item['sha256'];require(key not in retained,'Duplicate retained revision');retained.add(key)
        require((sha(item['text'].encode())if item['text']is not None else None)==item['sha256'],'Retained bytes differ')
    for report in reports(value):
        for name,digest in report['harnessFiles'].items():require((name,digest)in retained,'Consumed bytes omitted')
        for call in calls(report):
            for item in call['programInputs']:require((item['path'],item['sha256'])in retained,'Consumed bytes omitted')
    return dict(pluginCasesPerAPI=97,valueCasesPerAPI=7,demandCasesPerAPI=3,originalAssertionsPerAPI=7,fullMigrationAcceptance=False)


def pack(work):
    old=ROOT/'.rerail/side-effects-events-hubaab'
    sources={'plugin':work/'source.json','values':work/'values-source.json','demand':work/'demand-source.json','scanner':old/'scanner-source.json',
             'incremental':old/'source-final.json','directory':ROOT/'.rerail/hub-directory-metadata/discovery-source-final.json'}
    groups={family:dict(source=read_json(path),**{mode:[read_json(p)for p in sorted(work.glob(f'{family}-{mode}-[0-9]*.json'))]for mode in MODES})for family,path in sources.items()}
    value=dict(format='rerail.side-effects-plugin-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,contracts=groups,
        originals=dict(source=read_json(old/'original-source.json'),**{mode:read_json(work/f'original-{mode}-0.json')for mode in MODES}),
        tdd=dict(source=read_json(work/'source-red.json'),native=read_json(work/'red.json'),pathOrder=read_json(work/'path-order-red.json'),rootJoin=read_json(work/'roots-red.json'),demand=read_json(work/'demand-red.json')),
        build=read_json(work/'root-build.json'),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        remaining=['Other public helpers and scanners','CLI','Production facts and reports','Historical inputs','Updates and removal controls','Strict admission'])
    snapshots={r['path']+'@'+str(r['sha256']):r for r in read_json(work/'path-order-red-programs.json')}
    for report in reports(value):
        for name,digest in report['harnessFiles'].items():
            key=name+'@'+digest
            if key not in snapshots:snapshots[key]=dict(path=name,sha256=digest,text=retained_bytes(name,digest))
        for call in calls(report):
            for item in call['programInputs']:
                name,digest=item['path'],item['sha256'];key=name+'@'+str(digest)
                if key not in snapshots:snapshots[key]=dict(path=name,sha256=digest,text=retained_bytes(name,digest))
    value['snapshots']=list(snapshots.values());value['summary']=validate(value)
    return value


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    v=pack(a.input);a.out.write_bytes(gzip.compress(json.dumps(v,ensure_ascii=False,separators=(',',':')).encode(),mtime=0));print(v['summary'])
