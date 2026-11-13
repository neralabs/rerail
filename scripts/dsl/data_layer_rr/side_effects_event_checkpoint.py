"""Retain complete callback and syntax comparisons without claiming rule admission."""
import argparse
import gzip
import json
import subprocess
from pathlib import Path
from migration import ROOT, PIN, MANIFEST, read_json, sha
from validate_supplied_facts_checkpoint import require

COUNTS={'incremental':17,'collector':158,'helper':197,'tape':142,'scanner':35}
MODES=('snapshot','workspace')


def validate(value):
    require(value['format']=='rerail.side-effects-event-progress/v1'and value['sourcePin']==PIN,'Wrong callback progress scope')
    require(value['fullMigrationAcceptance']is False,'Callback parity is not full rule admission')
    digest=value['build']['sha256']
    for family,count in COUNTS.items():
        group=value['contracts'][family];source=group['source']
        require(source['passed']and source['sourcePin']==PIN and len(source['cases'])==count,'Incomplete source corpus: '+family)
        expected={json.dumps(r['input'],sort_keys=True):r['expected']for r in source['cases']}
        require(len(expected)==count,'Duplicate source case: '+family)
        for mode in MODES:
            actual={}
            for report in group[mode]:
                require(report['passed']and not report['semanticFallbacks'],'Native contract failed: '+family)
                require(report['engineSha256']==digest and report['executionMode']==mode,'Wrong engine or API: '+family)
                require(report['sourcePin']==PIN and report['sourceSha256']==source['sourceSha256'],'Wrong pinned source: '+family)
                require(report['baselineSha256']==sha((json.dumps(source,indent=2,ensure_ascii=False)+'\n').encode()),'Wrong source recording: '+family)
                for row in report['cases']:
                    key=json.dumps(row['input'],sort_keys=True)
                    require(key not in actual,'Duplicate native case: '+family)
                    require(key in expected and row['passed']and row['expected']==row['actual']==expected[key],'Complete native result differs: '+family)
                    actual[key]=row
            require(set(actual)==set(expected),'Native case omitted: '+family)
            if family=='incremental':
                batch=next(r for r in actual.values()if r['input']['id']=='batched-scope')
                require(len(batch['actual']['facts'])==64,'Batched positive omitted')
                require(len(batch['native'])==1,'Callback batch split into individual executions')
                require(sum(r['code']=='PSE_EVENT_FACT'for r in batch['native'][0]['diagnostics'])==64,'Native batch witness missing')
    original=value['originals']['source']
    require(original['exitCode']==0 and len(original['collected'])==7,'Incomplete unchanged original suite')
    for mode in MODES:
        report=value['originals'][mode]
        require(report['engineSha256']==digest and report['executionMode']==mode,'Wrong original engine or API')
        require(report['exitCode']==0 and not report['semanticFallbacks'],'Original assertion failed')
        for key in ['collected','cases','observations','sourceFiles']:
            require(report[key]==original[key],'Original assertion or observation omitted')
        require(set(report['cases'])==set(original['collected']),'Original assertion omitted')
    red=value['tdd']['native']
    require(not red['passed']and len(red['cases'])==11 and all(not r['passed']for r in red['cases']),'Missing pre-fix failure witnesses')
    require(red['engineSha256']==value['tdd']['build']['sha256'],'Wrong pre-fix TDD engine')
    for item in value['snapshots']:
        require((sha(item['text'].encode())if item['text']is not None else None)==item['sha256'],'Retained bytes differ')
    return dict(originalAssertionsPerAPI=7,comparisonsPerAPI=sum(COUNTS.values()),callbackBatch=64,fullMigrationAcceptance=False)


def retained_bytes(name,digest):
    path=ROOT/name
    if digest is None:return None
    if path.is_file()and sha(path.read_bytes())==digest:return path.read_text()
    revisions=subprocess.check_output(['git','-C',str(ROOT),'log','--format=%H','--',name],text=True).splitlines()
    for revision in revisions:
        old=subprocess.run(['git','-C',str(ROOT),'show',revision+':'+name],capture_output=True)
        if old.returncode==0 and sha(old.stdout)==digest:return old.stdout.decode()
    raise ValueError('Missing consumed revision: '+name+'@'+digest)


def pack(work):
    native=ROOT/'.rerail/side-effects-native';public=ROOT/'.rerail/side-effects-public-hub'
    sources={'incremental':work/'source-final.json',**{family:work/(family+'-source.json')for family in ['collector','helper','tape','scanner']}}
    contracts={family:dict(source=read_json(path),**{mode:[read_json(p)for p in sorted(work.glob(f'{family}-{mode}-[0-9]*.json'))]for mode in MODES})for family,path in sources.items()}
    originals=dict(source=read_json(work/'original-source.json'),**{mode:read_json(work/f'original-{mode}-0.json')for mode in MODES})
    tdd=dict(source=read_json(work/'source.json'),native=read_json(work/'red.json'),build=read_json(public/'root-build.json'))
    reports=[*originals.values(),tdd['source'],tdd['native'],*[r for group in contracts.values()for key,data in group.items()for r in ([data]if key=='source'else data)]]
    snapshots={}
    for report in reports:
        for name,digest in report['harnessFiles'].items():
            snapshots[name+'@'+digest]=dict(path=name,sha256=digest,text=retained_bytes(name,digest))
        calls=report.get('native',[])if isinstance(report['cases'],dict)else [n for row in report['cases']for n in row.get('native',[])]
        for call in calls:
            for item in call['programInputs']:
                name,digest=item['path'],item['sha256']
                snapshots[name+'@'+str(digest)]=dict(path=name,sha256=digest,text=retained_bytes(name,digest))
    value=dict(format='rerail.side-effects-event-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,contracts=contracts,
        originals=originals,tdd=tdd,build=read_json(work/'root-build.json'),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        snapshots=list(snapshots.values()),remaining=['Other public helpers and scanners','Raw plugin reporting','CLI','Production','History','Updates and removal controls','Strict admission'])
    value['summary']=validate(value)
    return value


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    value=pack(a.input);a.out.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0));print(value['summary'])
