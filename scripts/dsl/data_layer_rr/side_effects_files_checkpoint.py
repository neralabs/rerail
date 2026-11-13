"""Retain complete filesystem API observations and their source-driven failures."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT, PIN, MANIFEST, read_json, sha
from side_effects_event_checkpoint import calls, retained_bytes
from validate_supplied_facts_checkpoint import equal, require

MODES=('snapshot','workspace')
COUNTS={'files':39,'scanner':35}


def reports(value):
    yield from value['originals'].values()
    for group in value['contracts'].values():
        yield group['source']
        for mode in MODES:yield from group[mode]
    yield from value['tdd'].values()


def validate(value):
    require(value['format']=='rerail.side-effects-files-progress/v1'and value['sourcePin']==PIN,'Wrong filesystem progress scope')
    require(value['fullMigrationAcceptance']is False,'Filesystem parity is not full rule admission')
    for family,count in COUNTS.items():
        group=value['contracts'][family];source=group['source']
        require(source['passed']and source['sourcePin']==PIN and len(source['cases'])==count,'Incomplete source corpus')
        expected={json.dumps(r['input'],sort_keys=True):r['expected']for r in source['cases']}
        require(len(expected)==count,'Duplicate source case')
        baseline=sha((json.dumps(source,indent=2,ensure_ascii=False)+'\n').encode())
        for mode in MODES:
            seen=set()
            for report in group[mode]:
                require(report['passed']and not report['semanticFallbacks'],'Native filesystem contract failed')
                require(report['engineSha256']==value['build']['sha256']and report['executionMode']==mode,'Wrong engine or API')
                require(report['sourceSha256']==source['sourceSha256']and report['baselineSha256']==baseline,'Wrong source recording')
                for row in report['cases']:
                    key=json.dumps(row['input'],sort_keys=True)
                    require(key not in seen,'Duplicate native case')
                    require(key in expected and row['passed']and equal(row['expected'],expected[key])and equal(row['actual'],expected[key]),'Complete filesystem observation differs')
                    seen.add(key)
            require(seen==set(expected),'Native filesystem case omitted')
            if family=='files':
                batch=next(r for report in group[mode]for r in report['cases']if r['input']['id']=='explicit-batch')
                require(len(batch['actual']['value'])==64 and len(batch['native'])==1,'Whole positive file batch was split or omitted')
                require(sum(r['code']=='PSE_FACT'for call in batch['native']for r in call['diagnostics'])==64,'Native positive batch witness omitted')
    source=value['originals']['source']
    require(source['exitCode']==0 and len(source['collected'])==7,'Incomplete original suite')
    for mode in MODES:
        report=value['originals'][mode]
        require(report['engineSha256']==value['build']['sha256']and report['executionMode']==mode,'Wrong original engine or API')
        require(report['exitCode']==0 and not report['semanticFallbacks'],'Original assertion failed')
        for key in ['collected','cases','observations','sourceFiles']:require(equal(report[key],source[key]),'Original assertion or observation differs')
        require(set(report['cases'])==set(source['collected']),'Original assertion omitted')
    red=value['tdd']['native']
    require(not red['passed']and len(red['cases'])==36 and all(not r['passed']for r in red['cases']),'Missing pre-port failures')
    external=next(r for r in red['cases']if r['input']['id']=='repo-external-link')
    require(len(external['expected']['value'])==1 and external['actual']=={'value':[]},'Missing external file-link positive')
    before=[r for i in range(3)for r in value['tdd'][f'preDrain{i}']['cases']]
    require(len(before)==39 and len({r['input']['id']for r in before})==39,'Incomplete pre-repair corpus')
    require({r['input']['id']for r in before if not r['passed']}=={'explicit-error-order','explicit-external','repo-external-link'},'Missing exact three-failure checkpoint')
    logs={r['name']:r['text']for r in value['logs']}
    require('left: 0' in logs['complete-answer-red-test.log']and 'right: 1000' in logs['complete-answer-red-test.log'],'Missing deferred-positive failure')
    require('1 passed; 0 failed'in logs['complete-answer-exact-test.log'],'Missing exact deferred-batch regression')
    require('4 passed; 0 failed'in logs['world83-test.log'],'Missing filesystem capability regressions')
    for item in value['logs']:require(sha(item['text'].encode())==item['sha256'],'Retained log bytes differ')
    retained=set()
    for item in value['snapshots']:
        key=item['path'],item['sha256'];require(key not in retained,'Duplicate retained revision');retained.add(key)
        require((sha(item['text'].encode())if item['text']is not None else None)==item['sha256'],'Retained bytes differ')
    for report in reports(value):
        for name,digest in report['harnessFiles'].items():require((name,digest)in retained,'Consumed bytes omitted')
        for call in calls(report):
            for item in call['programInputs']:require((item['path'],item['sha256'])in retained,'Consumed bytes omitted')
    return dict(filesystemCasesPerAPI=39,scannerCasesPerAPI=35,originalAssertionsPerAPI=7,positiveFileBatch=64,fullMigrationAcceptance=False)


def pack(work):
    contracts={family:dict(source=read_json(work/filename),**{mode:[read_json(p)for p in sorted(work.glob(f'{family}-{mode}-[0-9]*.json'))]for mode in MODES})
        for family,filename in [('files','source-final.json'),('scanner','scanner-source.json')]}
    value=dict(format='rerail.side-effects-files-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,contracts=contracts,
        originals=dict(source=read_json(work/'original-source.json'),**{mode:read_json(work/f'original-{mode}.json')for mode in MODES}),
        tdd=dict(source=read_json(work/'source-red.json'),native=read_json(work/'red.json'),
            **{f'preDrain{i}':read_json(work/f'pre-drain-{i}.json')for i in range(3)},
            shapeCycle=read_json(work/'first-repo.json'),shapeOptions=read_json(work/'first-repo-finite.json'),
            scopeAnchor=read_json(work/'explicit-parent.json')),
        build=read_json(work/'root-build.json'),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        preRepairBuild=read_json(work/'pre-drain-root-build.json'),
        logs=[dict(name=p.name,sha256=sha(p.read_bytes()),text=p.read_text())for p in sorted(work.glob('*.log'))],
        remaining=['Other public conversion helpers','CLI','Production facts and reports','Historical inputs','Updates and removal controls','Strict admission'])
    snapshots={}
    for report in reports(value):
        pairs=list(report['harnessFiles'].items())
        pairs.extend((r['path'],r['sha256'])for call in calls(report)for r in call['programInputs'])
        for name,digest in pairs:
            key=name,digest
            if key not in snapshots:snapshots[key]=dict(path=name,sha256=digest,text=retained_bytes(name,digest))
    value['snapshots']=list(snapshots.values());value['summary']=validate(value)
    return value


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    value=pack(a.input);a.out.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0));print(value['summary'])
