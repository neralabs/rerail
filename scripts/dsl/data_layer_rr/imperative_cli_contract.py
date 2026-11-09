"""Compare real-source imperative CLI output and status with the native rule."""
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

from imperative_source_contract import CASES as SOURCE_CASES
from migration import ROOT,PIN,git,sha,read_json,write_json
from replay import write_fixture,execute_workspace

by_id={c['id']:c for c in SOURCE_CASES}
CASES=[]
for name in ['empty','direct','private-eager','diamond-reversed','legacy-insertion-phases','unicode-layer','reason-priority','weighted-source-order']:
    for use_json in [False,True]:
        CASES.append(dict(id=name+('-json' if use_json else '-text'),files=by_id[name]['files'],json=use_json,check=True,limit=100,dirs=None))
for label,files,dirs,limit,check in [
    ('limit-zero',by_id['private-eager']['files'],None,0,True),
    ('limit-negative',by_id['private-eager']['files'],None,-1,False),
    ('empty-explicit',by_id['direct']['files'],[],100,True),
    ('empty-modern',[dict(path='api/a.py',source='def ignored(c): c.execute()\n'),dict(path='data_layer/a.py',source='def public(c): c.fetchall()\n')],[],100,True),
    ('explicit-other',[dict(path='other/a.py',source='def public(c): c.execute()\n'),dict(path='impl/ignored.py',source='def ignored(c): c.fetchall()\n')],['other'],100,True),
    ('explicit-test-file',[dict(path='impl/tests/a.py',source='def public(c): c.execute()\n')],['impl/tests/a.py'],100,True),
    ('public-legacy',[dict(path='api/a.py',source='def public(c): c.execute()\n'),dict(path='impl/a.py',source='def private(c): c.fetchall()\n')],['public'],100,True),
    ('public-modern',[dict(path='api/a.py',source='def ignored(c): c.execute()\n'),dict(path='data_layer/a.py',source='def public(c): c.execute()\n')],['public'],100,True),
    ('public-unicode',by_id['unicode-layer']['files'],['public'],100,True),
    ('duplicate-directories',by_id['direct']['files'],['impl','impl'],100,True),
    ('public-deduplicates',by_id['direct']['files'],['impl','public','impl'],100,True),
]:CASES.append(dict(id=label,files=files,json=False,check=check,limit=limit,dirs=dirs))


def source_case(source,item):
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_imperative_private_functions')
    with tempfile.TemporaryDirectory(prefix='rerail-imperative-cli-source-') as directory:
        root=Path(directory);write_fixture(root,item['files'])
        argv=['lint_imperative_private_functions.py','--root',str(root),'--limit',str(item['limit'])]
        if item['json']:argv.append('--json')
        if item['check']:argv.append('--check')
        if item['dirs'] is not None:argv+=['--dirs',*item['dirs']]
        output=io.StringIO();errors=io.StringIO()
        # Relocate only the function's bound repository path into the physical
        # fixture. Its implementation and all graph/report logic stay intact.
        with patch.object(module._default_dirs,'__defaults__',(root,)),patch.object(sys,'argv',argv),contextlib.redirect_stdout(output),contextlib.redirect_stderr(errors):
            status=module.cli_main()
        return dict(status=status,stdout=output.getvalue(),stderr=errors.getvalue())


def native_case(engine,item):
    options=dict(entryPoint='cli_main',cliCheck=item['check'],cliJson=item['json'],reportLimit=str(item['limit']))
    if item['dirs'] is not None:options['env']=['ipf.cli.directories=true',*[f'data-layer.scan.{i}={p}' for i,p in enumerate(item['dirs'])]]
    native=execute_workspace(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/imperative-private-functions.rr',item['files'],options)
    rows=native['diagnostics']
    if any(r['code'] not in {'IPF_OUTPUT','IPF_EXIT'} for r in rows):raise ValueError('Unexpected native CLI diagnostic')
    exits=[r['parameters']['status'] for r in rows if r['code']=='IPF_EXIT']
    if len(exits)!=1:raise ValueError('One native CLI status required')
    output=''.join(r['parameters']['text'] for r in sorted((r for r in rows if r['code']=='IPF_OUTPUT'),key=lambda r:(r['parameters']['stage'],r['parameters']['ordinal'])))
    return dict(status=exits[0],stdout=output,stderr=''),native['programInputs']


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=name=='output')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    if not 0<=args.part<args.parts:parser.error('Invalid partition')
    result=dict(format='rerail.imperative-cli/v1',sourcePin=PIN,fullMigrationAcceptance=False,sourceFixtureRelocation='Only _default_dirs.__defaults__[0] is rebound to the physical source fixture.',cases=[],complete=False,passed=False)
    if args.command=='source':
        args.source=args.source.absolute()
        if git(args.source,'rev-parse','HEAD')!=PIN:parser.error('Pinned source required')
        for name in ['scripts/lint_imperative_private_functions.py','scripts/lint_lib/symbol_graph.py','scripts/lint_lib/discovery.py']:
            if (args.source/name).read_bytes()!=subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name]):parser.error('Modified source')
        write_json(args.output,result)
        for item in CASES:result['cases'].append(dict(item,expected=source_case(args.source,item)));write_json(args.output,result)
    else:
        if any(n.startswith(('scripts.lint_','lint_imperative_private_functions')) for n in sys.modules):raise ValueError('Native CLI cannot import detectors')
        baseline=read_json(args.baseline)
        if baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Complete source recording required')
        if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:parser.error('Complete unchanged corpus required')
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),selection=dict(part=args.part,parts=args.parts,total=len(CASES)));write_json(args.output,result)
        for item in baseline['cases'][args.part::args.parts]:
            actual,inputs=native_case(args.engine,item);result['cases'].append(dict(id=item['id'],expected=item['expected'],actual=actual,passed=actual==item['expected'],programInputs=inputs));write_json(args.output,result)
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']));write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c.get('passed',True)])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
