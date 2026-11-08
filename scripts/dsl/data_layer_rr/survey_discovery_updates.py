"""Compare source-graph updates through repeated runs in one persistent workspace."""
import argparse
import copy
import json
from pathlib import Path
import subprocess
import tempfile

from migration import PIN, ROOT, git, read_json, sha, verify_source_cohort, verify_engine_artifact, MANIFEST, write_json
from replay import program_inputs, write_fixture
from survey_discovery_contract import CASES, source_case, program, process_environment
from survey_analysis_contract import native_result, same_result


FORMAT='rerail.survey-discovery-updates/v1'


def steps():
    by_id={case['id']:case for case in CASES}
    result=[copy.deepcopy(by_id['weighted-workers-'+workers]) for workers in ['1','8','2']]
    smaller=copy.deepcopy(result[-1]);smaller['id']='shrink-file'
    smaller['files'][0]['source']='from impl.helpers import helper_a\ndef main(): helper_a()\n'
    result.append(smaller)
    deleted=copy.deepcopy(smaller);deleted['id']='remove-fillers-cross-threshold'
    deleted['files']=[file for file in deleted['files'] if '/filler_' not in file['path']]
    result.append(deleted)
    added=copy.deepcopy(smaller);added['id']='restore-fillers-cross-threshold'
    result.append(added)
    edited=copy.deepcopy(added);edited['id']='change-call-target'
    edited['files'][0]['source']=edited['files'][0]['source'].replace('helper_a','helper_b')
    result.append(edited)
    repeated=copy.deepcopy(edited);repeated['id']='repeat-without-changes'
    result.append(repeated)
    return result


def observed(native):
    rows=native['diagnostics']
    def paths(code):
        return [r['parameters']['path'] for r in sorted((r for r in rows if r['code']==code),key=lambda r:r['parameters']['ordinal'])]
    return dict(paths=paths('DISCOVERED_PATH'),assemblyPaths=paths('ASSEMBLY_PATH'),
                demandedPaths=sorted(r['parameters']['path'] for r in rows if r['code']=='DEMANDED_PATH'),
                analysis=native_result([r for r in rows if r['code'] not in {'DISCOVERED_PATH','ASSEMBLY_PATH','DEMANDED_PATH'}]))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=name=='output')
    args=parser.parse_args()
    result=dict(format=FORMAT,sourcePin=PIN,fullMigrationAcceptance=False,cases=[],complete=False,passed=False)
    if args.command=='source':
        if git(args.source,'rev-parse','HEAD')!=PIN:parser.error('Pinned source required')
        for step in steps():result['cases'].append(dict(step,expected=source_case(args.source,step)))
        expected=[c['expected'] for c in result['cases']]
        if expected[0]['assemblyPaths']==expected[1]['assemblyPaths']:raise ValueError('Worker refresh has no observable effect')
        if not len(expected[4]['paths'])<=50<len(expected[5]['paths']):raise ValueError('Updates must cross the scanner threshold in both directions')
        if same_result(expected[5]['analysis'],expected[6]['analysis']):raise ValueError('Source edit must change positive findings')
        if expected[6]!=expected[7]:raise ValueError('Unchanged source must preserve the full result')
    else:
        baseline=read_json(args.baseline)
        if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Complete pinned recording required')
        if [{k:c[k] for k in steps()[0]} for c in baseline['cases']]!=steps():parser.error('Complete unchanged update sequence required')
        source=read_json(ROOT/MANIFEST)['engineSource']
        verify_source_cohort(ROOT,source);verify_engine_artifact(ROOT,source,args.engine)
        rule=ROOT/'.rerail/rr-migration-20261106/internal-dead-code/weighted/updates.rr'
        rule.write_text(program())
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()))
        write_json(args.output,result)
        with tempfile.TemporaryDirectory(prefix='rerail-native-persistent-discovery-') as temporary:
            root=Path(temporary);workspace=root/'corpus';workspace.mkdir()
            previous=set()
            for index,step in enumerate(baseline['cases']):
                current={f['path'] for f in step['files']}
                for path in previous-current:(workspace/path).unlink()
                write_fixture(workspace,step['files'])
                previous=current
                options=dict(entryPoint='analyze_dead_code',env=[f'data-layer.scan.{i}={path}' for i,path in enumerate(step['directories'])])
                option_file=root/'options.json';write_json(option_file,options)
                command=[str(args.engine.absolute()),'run-rr',str(rule),str(workspace),str(root/'out'),str(option_file)]
                with process_environment(step):
                    run=subprocess.run(command,text=True,capture_output=True,timeout=100)
                if run.returncode:raise RuntimeError(run.stderr)
                native=json.loads(run.stdout);inputs=program_inputs(native['programInputs']);actual=observed(native)
                expected=step['expected']
                passed=all(actual[k]==expected[k] for k in ['paths','assemblyPaths','demandedPaths']) and same_result(expected['analysis'],actual['analysis'])
                result['cases'].append(dict(id=step['id'],expected=expected,actual=actual,passed=passed,programInputs=inputs,phases=native['phases']))
                write_json(args.output,result)
                print(json.dumps(dict(step=index,id=step['id'],passed=passed)),flush=True)
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']))
    write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
