"""Validate complete scanner observations and original assertions without claiming full admission."""
import argparse
import gzip
import json
import subprocess
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha
from validate_supplied_facts_checkpoint import require


def validate(value):
    require(value['format']=='rerail.side-effects-public-progress/v1'and value['sourcePin']==PIN,'Wrong public progress scope')
    require(value['fullMigrationAcceptance']is False,'Public scanning is not full rule admission')
    source=value['originals']['source'];require(source['exitCode']==0 and len(source['collected'])==7,'Incomplete original source suite')
    for mode in ['snapshot','workspace']:
        actual=value['originals'][mode]
        require(actual['engineSha256']==value['build']['sha256']and actual['executionMode']==mode,'Wrong engine or execution API')
        require(actual['exitCode']==0 and not actual['semanticFallbacks'],'Original assertion failed')
        for key in ['collected','cases','observations','sourceFiles']:require(actual[key]==source[key],'Original observations differ')
        require(set(actual['cases'])==set(source['collected']),'Original assertion omitted')
        require(all(any(p['phase']=='call'and p['outcome']=='passed'for p in phases)for phases in actual['cases'].values()),'Original assertion did not pass')
    source=value['scanners']['source'];require(source['passed']and len(source['cases'])==35,'Incomplete scanner source corpus')
    for mode in ['snapshot','workspace']:
        actual=value['scanners'][mode]
        require(actual['engineSha256']==value['build']['sha256']and actual['executionMode']==mode,'Wrong engine or execution API')
        require(actual['passed']and not actual['semanticFallbacks'],'Public scanner failed')
        require(len(actual['cases'])==len(source['cases']),'Scanner case omitted')
        for before,after in zip(source['cases'],actual['cases']):
            require(before['input']==after['input']and before['expected']==after['expected']==after['actual']and after['passed'],'Complete scanner result differs')
    errors=[r for r in source['cases']if r['expected'].get('error',{}).get('type')=='UnicodeDecodeError']
    require(len(errors)==8,'Decode error witnesses omitted')
    aliases=next(r for r in source['cases']if r['input']['id']=='file_links')
    require(len(aliases['expected']['value'])==2,'Alias positive omitted')
    for snapshot in value['snapshots']:
        require((sha(snapshot['text'].encode())if snapshot['text']is not None else None)==snapshot['sha256'],'Retained program or harness bytes differ')
    return dict(originalAssertionsPerAPI=7,scannerCasesPerAPI=35,fullMigrationAcceptance=False)


def pack(work):
    originals=dict(source=read_json(work/'original-source.json'),**{mode:read_json(work/f'original-{mode}-final.json')for mode in ['snapshot','workspace']})
    scanners=dict(source=read_json(work/'scanners-source-final.json'),**{mode:read_json(work/f'scanners-{mode}-final.json')for mode in ['snapshot','workspace']})
    incremental=dict(source=read_json(work/'incremental-source.json'),red=read_json(work/'incremental-red.json'))
    directory=dict(source=read_json(ROOT/'.rerail/hub-directory-metadata/discovery-source-final.json'),**{mode:read_json(work/f'directory-{mode}.json')for mode in ['snapshot','workspace']})
    snapshots=read_json(work/'pre-fix-snapshots.json')
    for family in [originals,scanners,incremental,directory]:
        for report in family.values():
            for name,digest in report['harnessFiles'].items():
                key=name+'@'+digest
                if key in snapshots:continue
                text=(ROOT/name).read_text()
                if sha(text.encode())!=digest:
                    revisions=subprocess.check_output(['git','-C',str(ROOT),'log','--format=%H','--',name],text=True).splitlines()
                    for revision in revisions:
                        old=subprocess.run(['git','-C',str(ROOT),'show',revision+':'+name],capture_output=True)
                        if old.returncode==0 and sha(old.stdout)==digest:text=old.stdout.decode();break
                require(sha(text.encode())==digest,'Changed harness: '+name)
                snapshots[key]=dict(path=name,sha256=digest,text=text)
            calls=report.get('native',[])if isinstance(report['cases'],dict)else [n for row in report['cases']for n in row.get('native',[])]
            for call in calls:
                for item in call['programInputs']:
                    name,digest=item['path'],item['sha256'];path=ROOT/name;text=path.read_text()if path.is_file()else None
                    require((sha(text.encode())if text is not None else None)==digest,'Changed program: '+name)
                    snapshots[name+'@'+str(digest)]=dict(path=name,sha256=digest,text=text)
    value=dict(format='rerail.side-effects-public-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,
        originals=originals,scanners=scanners,build=read_json(work/'root-build.json'),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        snapshots=list(snapshots.values()),incremental=incremental,directory=directory,
        tddLogs={name:(work/name).read_text()for name in ['file-error-red.log','file-error-green.log','ground-request-red.log','ground-request-green.log','stated-request-red.log','stated-request-green.log','request-identity-red-2.log','request-identity-green.log']},redOriginals=read_json(work/'original-red.json'),redScanners=read_json(work/'scanners-red.json'),
        groundedRequestRegression=dict(build=read_json(work/'root-build-hub473.json'),reports={kind+'-'+mode:read_json(work/f'{kind}-{mode}-hub473-grounded-red.json')for kind in ['original','scanners']for mode in ['snapshot','workspace']}),
        remaining=['Incremental collector callbacks and mutable state','Other public scanners and helpers','Raw fact and plugin reporting',
                   'CLI','Production facts and reports','Historical inputs','Persistent updates and effective removal controls','Strict rule admission'])
    value['summary']=validate(value);return value


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    value=pack(a.input);a.out.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0));print(value['summary'])
