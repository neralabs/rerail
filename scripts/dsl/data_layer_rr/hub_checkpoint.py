"""Retain hub adoption and bounded kernel parity without claiming full-rule admission."""
import argparse,gzip,json,subprocess
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha


def main():
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--source-input',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    manifest=read_json(ROOT/MANIFEST);engine=manifest['engineSource'];binary=engine['cliArtifact'];groups={}
    programs={};harness={}
    for label,parts in [('tape',None),('collector',2),('helpers',4)]:
        source=read_json(a.source_input/(label+'-source.json'));assert source['passed'] and source['sourcePin']==PIN
        modes={}
        for mode in ['snapshot','workspace']:
            reports=([read_json(a.input/(label+'-'+mode+'-all.json'))]if parts is None else
                [read_json(a.input/(label+'-'+mode+'-final-'+str(i)+'.json'))for i in range(parts)])
            by_input={};count=0
            for report in reports:
                assert report['passed'] and not report.get('semanticFallbacks')
                assert report['executionMode']==mode and report['engineSha256']==binary['sha256']
                assert report['sourceSha256']==source['sourceSha256']
                assert report['baselineSha256']==sha((a.source_input/(label+'-source.json')).read_bytes())
                for name,digest in report['harnessFiles'].items():
                    data=(ROOT/name).read_bytes()
                    if sha(data)!=digest:
                        # The audit's scope grew during this cohort. Preserve that guard revision explicitly.
                        data=subprocess.check_output(['git','-C',str(ROOT),'show',binary['engineCommit']+':'+name])
                    assert sha(data)==digest,(name,digest)
                    harness[name+'@'+digest]=dict(path=name,sha256=digest,text=data.decode())
                for row in report['cases']:
                    key=json.dumps(row['input'],sort_keys=True);assert key not in by_input
                    by_input[key]=row;count+=1
                    assert row['passed'] and row['actual']==row['expected']
                    for call in row['native']:
                        for item in call['programInputs']:
                            name,digest=item['path'],item['sha256'];path=ROOT/name
                            content=path.read_bytes()if path.is_file()else None
                            assert (sha(content)if content is not None else None)==digest,(name,digest)
                            programs[name+'@'+str(digest)]=dict(path=name,sha256=digest,text=content.decode()if content is not None else None)
            assert count==len(source['cases'])
            for row in source['cases']:
                assert by_input[json.dumps(row['input'],sort_keys=True)]['expected']==row['expected']
            modes[mode]=reports
        groups[label]=dict(source=source,native=modes,cases=len(source['cases']))
    assert not subprocess.check_output(['git','-C',str(ROOT),'diff','--name-only',engine['upstreamCommit'],'--','native','accel','forest','tools/bitlex','Cargo.toml','Cargo.lock'],text=True).strip()
    survey=read_json(a.input/'root-rule-syntax-survey.json');assert survey['engineSha256']==binary['sha256'] and len(survey['rows'])==99
    comparisons=sum(g['cases']*2 for g in groups.values())
    receipt=dict(format='rerail.hub-integration-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,
        hubCommit=engine['upstreamCommit'],engineSource=engine,build=read_json(a.input/'root-build.json'),
        readerSurvey=survey,kernels=groups,comparisons=comparisons,programSnapshots=list(programs.values()),harnessSnapshots=list(harness.values()),
        boundaries=['The collector framework parses source before callbacks; the separate tape corpus verifies native syntax outcomes.',
                    'Historical admissions are preserved; this receipt does not re-admit them on the hub.',
                    'The expanded source-audit guard has two recorded revisions; detector and engine inputs are unchanged.'],
        remaining=['37 reader/type compatibility failures across the other authored policies',
                   'Side-effects incremental callbacks, public API, CLI, production, historical corpus, updates and controls',
                   'Full original assertions through both APIs and strict rule admission'])
    a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_bytes(gzip.compress(json.dumps(receipt,sort_keys=True).encode(),mtime=0))
    print(dict(comparisons=comparisons,fullMigrationAcceptance=False))

if __name__=='__main__':main()
