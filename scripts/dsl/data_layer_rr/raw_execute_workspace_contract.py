"""Compare real workspace selection with the pinned plugin's complete reporter values."""
import argparse
import json
from pathlib import Path
import sys

from migration import ROOT,PIN,sha,read_json,write_json
from replay import execute_workspace
from raw_execute_regression_cases import REPORTER_CASES


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','cases','oracle','output']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--rule',type=Path,default=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/raw-execute-regression.rr')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:parser.error('Invalid partition')
    corpus=read_json(args.cases);oracle=read_json(args.oracle)
    if corpus['cases']!=REPORTER_CASES:parser.error('Complete unchanged reporter corpus required')
    if oracle['sourcePin']!=PIN or oracle['execution']!='offline-source' or oracle['casesSha256']!=sha(args.cases.read_bytes()):parser.error('Pinned source recording required')
    if [r['id'] for r in oracle['results']]!=[c['id'] for c in REPORTER_CASES] or any(r['status']!='passed' for r in oracle['results']):parser.error('Complete passing reporter recording required')
    if any(set(r.get('sourcePlugin',{}))!={'name','enforcement','description'} for r in oracle['results']):parser.error('Complete source plugin metadata required')
    if not any(r['value'] for r in oracle['results']):parser.error('Reporter positives required')
    if any(name.startswith('scripts.lint_') for name in sys.modules):raise ValueError('Native workspace acceptance cannot import source detectors')
    result=dict(format='rerail.raw-execute-workspace/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),casesSha256=sha(args.cases.read_bytes()),oracleSha256=sha(args.oracle.read_bytes()),
                caseSelection=dict(part=args.part,parts=args.parts,total=len(REPORTER_CASES)),cases=[],complete=False,passed=False)
    write_json(args.output,result)
    rule=args.rule
    for index in range(args.part,len(REPORTER_CASES),args.parts):
        case=REPORTER_CASES[index]
        native=execute_workspace(args.engine,rule,case['files'],{},workspace_name=case.get('workspaceName','corpus'))
        metadata=native['metadata']
        if len(metadata['policies'])!=1:raise ValueError('Expected one compiled policy')
        identity=metadata['policies'][0]['id'];prefix='data-layer-'
        if not identity.startswith(prefix):raise ValueError('Unexpected policy identity')
        modes=[value for key,value in metadata['settings'] if key=='mode']
        descriptions=[value for key,value in metadata['settings'] if key=='description']
        if len(modes)!=1 or len(descriptions)!=1 or modes[0] not in {'block','log'}:raise ValueError('Incomplete compiled plugin metadata')
        plugin=dict(name=identity.removeprefix(prefix).replace('-','_'),enforcement={'block':'hard-fail','log':'info'}[modes[0]],description=json.loads(descriptions[0]))
        severities={value.split()[0]:value.split()[1] for key,value in metadata['settings'] if key=='severity'}
        rows=sorted(native['diagnostics'],key=lambda r:(r['path'],r['line'],r['message']))
        actual=[dict(linter_name=plugin['name'],code=r['code'],path=r['path'],line=r['line'],message=r['message'],severity=severities.get(r['clause'],'info' if modes[0]=='log' else 'hard-fail')) for r in rows]
        expected=oracle['results'][index]['value']
        expected_plugin=oracle['results'][index]['sourcePlugin']
        result['cases'].append(dict(id=case['id'],expected=expected,actual=actual,sourcePlugin=plugin,expectedSourcePlugin=expected_plugin,
                                  passed=actual==expected and plugin==expected_plugin,programInputs=native['programInputs'],phases=native['phases'],metadata=metadata))
        write_json(args.output,result)
    result.update(complete=True,passed=all(c['passed'] for c in result['cases']));write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c['passed']])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
