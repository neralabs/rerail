"""Compare both consumers through edits, deletion, discovery changes, and warm repeats."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from migration import ROOT,PIN,MANIFEST,read_json,sha,verify_source_cohort,verify_engine_artifact,write_json
from raw_execute_cli_contract import CASES,FORMAT,native_value,check_positives
from raw_execute_regression_cases import REPORTER_CASES
from replay import write_fixture,program_inputs


STEPS=['raw-reporter-0','raw-reporter-7','raw-reporter-0',
       'raw-reporter-compatibility-raise','raw-reporter-compatibility-wrapper','raw-reporter-compatibility-raise',
       'raw-reporter-18','raw-reporter-19','raw-reporter-4','raw-reporter-5',
       'raw-reporter-parse-failure','raw-reporter-0','raw-reporter-0']


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','cli-baseline','reporter-baseline','output']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    cli=read_json(args.cli_baseline);reporter=read_json(args.reporter_baseline)
    if cli['format']!=FORMAT or cli['sourcePin']!=PIN or not cli['complete'] or not cli['passed']:parser.error('Passing complete CLI source recording required')
    if [{k:c[k] for k in CASES[0]} for c in cli['cases']]!=CASES:parser.error('CLI corpus differs')
    check_positives(cli['cases'])
    if reporter['sourcePin']!=PIN or reporter['execution']!='offline-source' or any(r['status']!='passed' for r in reporter['results']):parser.error('Passing source reporter recording required')
    if [r['id'] for r in reporter['results']]!=[c['id'] for c in REPORTER_CASES]:parser.error('Reporter corpus differs')
    by_id={c['id']:c for c in cli['cases']};reported={r['id']:r for r in reporter['results']}
    source=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,source);verify_engine_artifact(ROOT,source,args.engine)
    result=dict(format='rerail.raw-execute-updates/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),baselineSha256=dict(cli=sha(args.cli_baseline.read_bytes()),reporter=sha(args.reporter_baseline.read_bytes())),cases=[],complete=False,passed=False)
    write_json(args.output,result)
    with tempfile.TemporaryDirectory(prefix='rerail-native-raw-updates-') as directory:
        root=Path(directory);workspace=root/'corpus';workspace.mkdir();previous=set()
        for index,label in enumerate(STEPS):
            case=by_id[label];current={f['path'] for f in case['files']}
            for path in previous-current:(workspace/path).unlink()
            write_fixture(workspace,case['files']);previous=current
            observed={}
            for mode,options in [('plugin',{}),('cli',dict(entryPoint='cli_main',cliCheck=case['check']))]:
                option_file=root/'options.json';write_json(option_file,options)
                command=[str(args.engine.absolute()),'run-rr',str(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/raw-execute-regression.rr'),str(workspace),str(root/'out'/mode),str(option_file)]
                run=subprocess.run(command,capture_output=True,text=True,timeout=100)
                if run.returncode:raise RuntimeError(run.stderr)
                native=json.loads(run.stdout)
                if mode=='cli':actual=native_value(native);expected=case['expected']
                else:
                    rows=sorted(native['diagnostics'],key=lambda r:(r['path'],r['line'],r['message']))
                    if any(r['policy']!='data-layer-raw-execute-regression' or r['clause']!='RRERemoved' for r in rows):raise ValueError('Unexpected plugin diagnostics')
                    actual=[dict(linter_name='raw_execute_regression',code=r['code'],path=r['path'],line=r['line'],message=r['message'],severity='hard-fail') for r in rows]
                    expected=reported[label]['value']
                observed[mode]=dict(actual=actual,expected=expected,passed=actual==expected,programInputs=program_inputs(native['programInputs']),phases=native['phases'])
            passed=all(v['passed'] for v in observed.values())
            result['cases'].append(dict(step=index,id=label,runs=observed,passed=passed));write_json(args.output,result)
            print(json.dumps(dict(step=index,id=label,passed=passed)),flush=True)
    result.update(complete=True,passed=all(c['passed'] for c in result['cases']));write_json(args.output,result)
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
