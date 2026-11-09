"""Compare native CLI discovery, ordered strings, output bytes, and status with the source."""
import argparse
import contextlib
import importlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT,PIN,git,sha,write_json
from raw_execute_regression_cases import REPORTER_CASES
from replay import write_fixture,execute_workspace


FORMAT='rerail.raw-execute-cli/v1'
CASES=[dict(id=c['id'],files=c['files'],check=i%2==0) for i,c in enumerate(REPORTER_CASES)]
CASES.extend([
    dict(id='cli-empty',files=[],check=True),
    dict(id='cli-root-and-component-order',check=True,files=[
        dict(path=p,source='c.raw_execute()\n') for p in [
            'l2000_second/z.py','_internal/a.py','data_layer/a.py','impl/a.py',
            'impl/a/__init__.py','impl/z.py','l1000_first/z.py']]),
    dict(id='cli-all-text-tokens',check=False,files=[dict(path='impl/a.py',source='# _execute_sql_or_schema_statement_async _is_schema_ddl_statement compatibility router arbitrary grammar router\nc.raw_execute()\n')]),
    dict(id='cli-invalid-syntax-text',check=True,files=[dict(path='impl/a.py',source='# compatibility router\ndef broken(\n')]),
    dict(id='cli-colon-raw-and-formatted',check=True,files=[dict(path='impl/a:b.py',source='# compatibility router\nc.raw_execute()\n')]),
    dict(id='cli-no-check-positive',check=False,files=[dict(path='impl/a.py',source='c.raw_execute()\n')]),
    dict(id='cli-discovery-exclusions',check=True,files=[dict(path=p,source='c.raw_execute()\n') for p in [
        'impl/browser_build/a.py','impl/conform/a.py','impl/.ruff_cache/a.py',
        'impl/.mypy_cache/a.py','impl/.pytest_cache/a.py','impl/node_modules/a.py','impl/keep/a.py']]),
])


def source_case(source,case):
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_raw_execute_regression')
    with tempfile.TemporaryDirectory(prefix='rerail-offline-raw-cli-') as directory:
        root=Path(directory).resolve();write_fixture(root,case['files'])
        output=io.StringIO();errors=io.StringIO()
        with patch.object(module,'REPO_ROOT',root),patch.object(sys,'argv',['lint_raw_execute_regression.py',*(['--check'] if case['check'] else [])]):
            paths=[p.relative_to(root).as_posix() for p in module._iter_python_files()]
            violations=module.collect_violations()
            with contextlib.redirect_stdout(output),contextlib.redirect_stderr(errors):status=module.cli_main()
        return dict(paths=paths,violations=violations,text=output.getvalue(),stderr=errors.getvalue(),status=status)


def native_value(native):
    rows=native['diagnostics']
    def parameters(code):return [r['parameters'] for r in rows if r['code']==code]
    known={'RRE_CLI_FILE','RRE_CLI_TEXT','RRE_CLI_EXIT'}
    unexpected=[r for r in rows if r['code'] not in known]
    if unexpected:raise ValueError('Unexpected native CLI diagnostics: '+repr(unexpected))
    paths=sorted(parameters('RRE_CLI_FILE'),key=lambda p:p['ordinal'])
    values=sorted(parameters('RRE_CLI_TEXT'),key=lambda p:(p['file_order'],p['stage'],p['ordinal']))
    statuses=parameters('RRE_CLI_EXIT')
    return dict(paths=[p['path'] for p in paths],violations=[p['value'] for p in values],
                text=''.join(p['value']+'\n' for p in values),stderr='',
                status=statuses[0]['status'] if len(statuses)==1 else None)


def check_positives(rows):
    by_id={r['id']:r['expected'] for r in rows}
    if by_id['cli-root-and-component-order']['paths']!=[
        'impl/a/__init__.py','impl/a.py','impl/z.py','data_layer/a.py','_internal/a.py','l1000_first/z.py','l2000_second/z.py']:
        raise ValueError('Missing root and Path-component order counterexample')
    if by_id['cli-no-check-positive']['status']!=0 or not by_id['cli-no-check-positive']['violations']:
        raise ValueError('Missing non-check positive')
    if by_id['cli-invalid-syntax-text']['status']!=1 or not by_id['cli-invalid-syntax-text']['violations']:
        raise ValueError('Invalid syntax must retain text violations')
    if by_id['cli-discovery-exclusions']['paths']!=['impl/keep/a.py']:
        raise ValueError('CLI scope differs from plugin scope')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=name=='output')
    parser.add_argument('--rule',type=Path,default=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/raw-execute-regression.rr')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:parser.error('Invalid partition')
    result=dict(format=FORMAT,sourcePin=PIN,fullMigrationAcceptance=False,cases=[],complete=False,passed=False)
    if args.command=='source':
        if git(args.source,'rev-parse','HEAD')!=PIN:parser.error('Pinned source required')
        names=['scripts/lint_raw_execute_regression.py','scripts/lint_lib/discovery.py']
        for name in names:
            if (args.source/name).read_bytes()!=subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name]):parser.error('Modified pinned source')
        result['sourceFiles']={name:sha((args.source/name).read_bytes()) for name in names}
        write_json(args.output,result)
        for case in CASES:
            result['cases'].append(dict(case,expected=source_case(args.source,case)));write_json(args.output,result)
        check_positives(result['cases'])
    else:
        if any(name.startswith(('scripts.lint_','lint_raw_execute_regression')) for name in sys.modules):raise ValueError('Native CLI cannot import source detectors')
        baseline=json.loads(args.baseline.read_text())
        if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Complete pinned recording required')
        if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:parser.error('Complete unchanged corpus required')
        check_positives(baseline['cases'])
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),caseSelection=dict(part=args.part,parts=args.parts,total=len(CASES)))
        write_json(args.output,result)
        for case in baseline['cases'][args.part::args.parts]:
            native=execute_workspace(args.engine,args.rule,case['files'],dict(entryPoint='cli_main',cliCheck=case['check']))
            actual=native_value(native)
            result['cases'].append(dict(id=case['id'],expected=case['expected'],actual=actual,passed=actual==case['expected'],programInputs=native['programInputs']))
            write_json(args.output,result)
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']));write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c.get('passed',True)])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
