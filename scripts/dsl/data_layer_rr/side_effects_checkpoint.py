"""Retain complete kernel comparisons without claiming whole-rule migration acceptance."""
import argparse,json,gzip
from pathlib import Path
from migration import ROOT,PIN,sha,read_json


def main():
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    def load(name):return read_json(a.input/(name+'.json'))
    source=load('helpers-source')
    helpers=[load('helpers-final-snapshot-'+str(i))for i in [0,1]]
    helpers += [load('helpers-final-small-snapshot-'+str(i))for i in [2,3,6,7,10,11,14,15]]
    tape_source=load('tape-source');tape=[load('tape-snapshot-'+str(i))for i in range(8)]
    for baseline,reports in [(source,helpers),(tape_source,tape)]:
        assert baseline['sourcePin']==PIN and baseline['passed']
        rows=[r for report in reports for r in report['cases']]
        by_input={json.dumps(r['input'],sort_keys=True):r for r in rows}
        assert len(rows)==len(by_input)==len(baseline['cases'])
        for r in baseline['cases']:
            actual=by_input[json.dumps(r['input'],sort_keys=True)]
            assert actual['expected']==r['expected']
        for report in reports:
            assert not report.get('semanticFallbacks')
            assert report['sourceSha256']==baseline['sourceSha256']
            assert report['baselineSha256']==sha((a.input/('helpers-source.json'if baseline is source else 'tape-source.json')).read_bytes())
            for row in report['cases']:
                assert row['native'] and row['passed']==(row['expected']==row['actual'])
    assert all(r['passed']for r in helpers)
    failures=[r for report in tape for r in report['cases']if not r['passed']]
    assert len(failures)==1 and failures[0]['input']['id']=='shape-syntax'
    revisions={}
    # Earlier tape reports predate the helper include. Preserve the exact program, not a relabeled revision.
    policy='gate/data-layer/packs/data-layer/candidates/rr/plan-scope-side-effects.rr'
    current=(ROOT/policy).read_bytes();earlier=current.replace(b'#include "data-layer/side-effects-helpers.rr"\n',b'')
    for report in [*helpers,*tape]:
        for row in report['cases']:
            for call in row['native']:
                for item in call['programInputs']:
                    name,digest=item['path'],item['sha256'];path=ROOT/name
                    content=path.read_bytes()if path.is_file()else None
                    if content is not None and sha(content)!=digest:
                        assert name==policy and sha(earlier)==digest,(name,digest)
                        content=earlier
                    assert (sha(content)if content is not None else None)==digest
                    revisions[name+'@'+str(digest)]=dict(path=name,sha256=digest,text=content.decode()if content is not None else None)
    engines={r['engineSha256']for r in [*helpers,*tape]};assert len(engines)==1
    receipt=dict(format='rerail.side-effects-kernel-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,
        helpers=dict(source=source,native=helpers,cases=len(source['cases']),passed=True),
        tape=dict(source=tape_source,native=tape,cases=len(tape_source['cases']),passed=False,failedCases=1),
        engineSha256=engines.pop(),programSnapshots=list(revisions.values()),
        remaining=['Exact native syntax-error formatting','Complete incremental collector callback/state contracts',
                   'Full original suite through both APIs','Public scanner and DTO/report contracts',
                   'CLI contracts','Production facts and reports','Historical inputs','Retained updates','Effective removal controls','Strict admission'])
    a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_bytes(gzip.compress(json.dumps(receipt,sort_keys=True).encode(),mtime=0))
    print(dict(helperCases=receipt['helpers']['cases'],tapeCases=receipt['tape']['cases'],tapeFailures=1,fullMigrationAcceptance=False))

if __name__=='__main__':main()
