"""A second policy must not remove raw-execute findings from its own workspace scope."""
import argparse
import csv
import json
from pathlib import Path
import subprocess
import tempfile

from migration import ROOT,PIN,MANIFEST,read_json,sha,verify_source_cohort,verify_engine_artifact,write_json
from replay import write_fixture,program_inputs
from raw_execute_regression_cases import REPORTER_CASES


NOISE='''.policy "raw-scope-control" revision 1
.language python
.mode log
.scope include "other/**/*.py"
#include "python/ast.rr"
.decl noise(name: symbol)
noise(name) :- kind(n,"function_definition"), def_name(n,name).
.output noise
'''
OBSERVATION='''
.decl observed_raw(path: symbol, line: number, detail: symbol, ordinal: number)
observed_raw(path,line,detail,i) :- rre_fact(_,i,path,line,detail).
.output observed_raw
'''


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','oracle','output']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();manifest=read_json(ROOT/MANIFEST)
    verify_source_cohort(ROOT,manifest['engineSource']);verify_engine_artifact(ROOT,manifest['engineSource'],args.engine)
    oracle=read_json(args.oracle)
    if oracle['sourcePin']!=PIN or oracle['execution']!='offline-source' or any(r['status']!='passed' for r in oracle['results']):parser.error('Passing pinned reporter recording required')
    if [r['id'] for r in oracle['results']]!=[c['id'] for c in REPORTER_CASES]:parser.error('Complete reporter recording required')
    expected_by_id={r['id']:r['value'] for r in oracle['results']}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/raw-execute-regression.rr'
    result=dict(format='rerail.raw-execute-combined-scope/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),ruleSha256=sha(policy.read_bytes()),oracleSha256=sha(args.oracle.read_bytes()),cases=[],complete=False,passed=False)
    write_json(args.output,result)
    programs=args.output.parent/'combined-scope-programs';single=programs/'single';pack=programs/'pack'
    single.mkdir(parents=True,exist_ok=True);pack.mkdir(parents=True,exist_ok=True)
    text=policy.read_text()+OBSERVATION
    for directory in [single,pack]:(directory/'a-raw.rr').write_text(text)
    (pack/'b-noise.rr').write_text(NOISE)
    command=[str(args.engine.absolute()),'check-rr',str(single/'a-raw.rr'),str(pack/'a-raw.rr'),str(pack/'b-noise.rr')]
    readers=subprocess.run(command,capture_output=True,text=True,timeout=100)
    if readers.returncode:raise RuntimeError(readers.stderr)
    result['readers']=json.loads(readers.stdout)
    for label in ['raw-reporter-0','raw-reporter-18','raw-reporter-4']:
        case=next(c for c in REPORTER_CASES if c['id']==label)
        expected=[[r['path'],str(r['line']),r['message'],str(i)] for i,r in enumerate(expected_by_id[label])]
        if len(expected)!=2:raise ValueError('Both duplicate positive observations must be retained')
        with tempfile.TemporaryDirectory(prefix='rerail-native-combined-scope-') as temporary:
            root=Path(temporary);corpus=root/'corpus';corpus.mkdir()
            write_fixture(corpus,[*case['files'],dict(path='other/probe.py',source='def probe(): pass\n')])
            runs={}
            for mode,directory in [('single',single),('combined',pack)]:
                out=root/mode
                command=[str(args.engine.absolute()),'logic',str(directory),str(corpus),str(out)]
                run=subprocess.run(command,capture_output=True,text=True,timeout=100)
                if run.returncode:raise RuntimeError(run.stderr)
                path=out/('observed_raw.csv' if mode=='single' else 'p0_observed_raw.csv')
                if not path.is_file():raise ValueError('Missing native observation output: '+str(sorted(p.name for p in out.iterdir()))+'\n'+run.stdout)
                rows=list(csv.reader(path.open(),delimiter='\t'))
                rows.sort(key=lambda r:int(r[3]))
                runs[mode]=dict(rows=rows,stdout=run.stdout,stderr=run.stderr)
            passed=all(r['rows']==expected for r in runs.values())
            result['cases'].append(dict(id=label,expected=expected,runs=runs,passed=passed))
            write_json(args.output,result)
    result.update(complete=True,passed=all(c['passed'] for c in result['cases']))
    inputs=[]
    for reader in result['readers']:inputs.extend(program_inputs(reader['files']))
    result['programInputs']=[r for r in inputs if r['sha256'] is not None]
    write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c['passed']])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
