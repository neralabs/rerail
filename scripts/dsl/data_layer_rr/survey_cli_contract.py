"""Record and compare the real survey CLI, including its directory and root decisions."""
import argparse
import contextlib
import importlib
import io
import json
from pathlib import Path
import re
import sys
import tempfile
from unittest.mock import patch

from migration import PIN, ROOT, git, sha, write_json
from replay import write_fixture
from survey_discovery_contract import case, file, directory, process_environment
from survey_analysis_contract import native_result, same_result


FORMAT='rerail.survey-cli/v1'


def cli_case(name,files,args=(),install_dirs=('impl',),omit_root=False):
    value=case(name,list(files)+[directory('__installation__')]+[directory('__installation__/'+d) for d in install_dirs])
    value.update(args=list(args),omitRoot=omit_root)
    return value


BASE=[file('impl/a.py','def dead(): pass\nclass Other: pass\n'),
      file('impl/browser/b.py','def browser_dead(): pass\n'),file('impl/browserish/c.py','def neighbor(): pass\n')]
DYNAMIC=[file('impl/a.py','def live(): pass\ndef dead(): pass\n'),
         file('impl/loader.py','# import_module("impl.a").live\n')]
CLASS=[file('impl/a.py','class Base:\n def inherited(self): pass\nclass Widget(Base):\n def run(self): pass\n def _private(self): pass\n def __init__(self): pass\ndef dead(): pass\n'),
       file('impl/loader.py','load_python_module("impl.a").Widget\n')]
CASES=[
    cli_case('empty',[]),
    cli_case('default-excludes',BASE),
    cli_case('without-default-excludes',BASE,['--no-default-excludes']),
    cli_case('check-positive',BASE,['--check']),
    cli_case('check-with-zero-limit',BASE,['--check','--limit','0']),
    cli_case('check-clean',[file('impl/a.py','value=1\n')],['--check']),
    cli_case('json',BASE,['--json','--check']),
    cli_case('negative-limit',BASE,['--limit','-1']),
    cli_case('normalized-exclusion',BASE,['--exclude-paths',' \u2003impl\\a.py\u2003 ']),
    cli_case('directory-exclusion',BASE,['--exclude-paths','impl/browserish']),
    cli_case('empty-exclusions-keep-default',BASE,['--exclude-paths']),
    cli_case('slash-exclusion',BASE,['--exclude-paths','/']),
    cli_case('exclude-all',BASE,['--exclude-paths','impl','--check']),
    cli_case('explicit-dirs',[*BASE,file('api/a.py')],['--dirs','api']),
    cli_case('empty-dirs-graph-falls-back',[*BASE,file('api/a.py')],['--dirs']),
    cli_case('public-expansion',[*BASE,file('data_layer/api.py','from impl.a import dead\n')],['--dirs','public','impl']),
    cli_case('installation-defaults',[file('impl/ignored.py'),file('l1234_target/a.py')],install_dirs=('l1234_target',)),
    cli_case('empty-installation-defaults',[file('api/a.py'),*DYNAMIC],install_dirs=()),
    cli_case('without-root',BASE,omit_root=True),
    cli_case('dynamic-function-comment',DYNAMIC),
    cli_case('dynamic-class-methods',CLASS),
    cli_case('dynamic-reexport',[file('impl/a.py','def live(): pass\ndef dead(): pass\n'),
             file('impl/facade.py','from impl.a import live\n'),file('impl/loader.py','importlib.import_module("impl.facade").live\n')]),
    cli_case('dynamic-invalid-file',[file('impl/a.py','def live(): pass\ndef dead(): pass\n'),
             file('impl/bad.py','def broken(\n# import_module("impl.a").live\n')]),
    cli_case('absolute-dirs',DYNAMIC,['--dirs','$ROOT/impl']),
    cli_case('absolute-valid-candidates-skip-invalid',[file('impl/a.py','def live(): pass\ndef dead(): pass\n'),
             file('impl/bad.py','def broken(\n# import_module("impl.a").live\n')],['--dirs','$ROOT/impl']),
    cli_case('absolute-prefix-collision',[file('impl/a.py','# import_module("impl.a").live\ndef live(): pass\n'),
             file('impl/a/__init__.py','def other(): pass\n')],['--dirs','$ROOT/impl']),
    cli_case('empty-dirs-no-dynamic-roots',DYNAMIC,['--dirs']),
    cli_case('unicode',[file('impl/é😀.py','def façade(): pass\n')],['--json']),
]


def expand(value,root):return value.replace('$ROOT',str(root))


def source_case(source,item):
    sys.path[:0]=[str(source),str(source/'scripts')]
    discovery=importlib.import_module('lint_lib.discovery')
    linter=importlib.import_module('lint_internal_dead_code')
    with process_environment(item,python=True),tempfile.TemporaryDirectory(prefix='rerail-offline-cli-') as temporary:
        root=Path(temporary).resolve();write_fixture(root,item['files'])
        args=[expand(value,root) for value in item['args']]
        if not item['omitRoot']:args=['--root',str(root),*args]
        observation={};build=linter.build_symbol_graph;dynamic=linter._build_dynamic_import_root_ids;analyze=linter.analyze_dead_code
        def graph(**kwargs):
            observation['directories']=[str(v).replace(str(root),'$ROOT') for v in kwargs['directories']]
            value=build(**kwargs);observation['graphPaths']=sorted(str(m.path.relative_to(root)) for m in value.modules.values())
            return value
        def roots(*args,**kwargs):
            value=dynamic(*args,**kwargs);observation['dynamicRoots']=sorted(value);return value
        def analysis(*args,**kwargs):
            value=analyze(*args,**kwargs);observation['analysis']=value;return value
        out=io.StringIO();err=io.StringIO()
        with patch.object(linter,'ROOT',root/'__installation__'), \
             patch.object(discovery,'__file__',str(root/'scripts/lint_lib/discovery.py')), \
             patch.object(sys,'argv',['lint_internal_dead_code.py',*args]), \
             patch.object(linter,'build_symbol_graph',graph), \
             patch.object(linter,'_build_dynamic_import_root_ids',roots), \
             patch.object(linter,'analyze_dead_code',analysis), \
             contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            status=linter.cli_main()
        return dict(observation,text=out.getvalue(),stderr=err.getvalue(),status=status)


def input_options(item,root):
    """Parse supplied flags only. Native rules perform every semantic selection."""
    parser=argparse.ArgumentParser()
    parser.add_argument('--dirs',nargs='*');parser.add_argument('--exclude-paths',nargs='*')
    parser.add_argument('--check',action='store_true');parser.add_argument('--json',action='store_true')
    parser.add_argument('--no-default-excludes',action='store_true');parser.add_argument('--limit',default='100')
    args=parser.parse_args(item['args'])
    env=['data-layer.cli.root='+str(root.resolve()),'data-layer.cli.install=__installation__']
    if args.dirs is not None:
        env.append('data-layer.cli.dirs=true')
        env.extend(f'data-layer.cli.dir.{i}={expand(value,root.resolve())}' for i,value in enumerate(args.dirs))
    return dict(entryPoint='cli_main',env=env,cliJson=args.json,cliCheck=args.check,
                cliNoDefaultExcludes=args.no_default_excludes,cliExcludePaths=args.exclude_paths or [],reportLimit=args.limit)


def program():
    text=(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/internal-dead-code.rr').read_text()
    text=re.sub(r'^\.scope include .*\n\.scope exclude[^\n]*','.scope dirs "__survey_no_primary__"',text,count=1,flags=re.M)
    return text+'''
.decl scli_directory(i: number, path: symbol)
.decl scli_survey()
.decl scli_dynamic_root(sid: symbol)
.violation CliDirectory(at: span, ordinal: number, path: symbol) code "CLI_DIRECTORY" "directory"
CliDirectory(@place("",0), i, path) :- scli_directory(i,path).
.violation CliGraphPath(at: span, path: symbol) code "CLI_GRAPH_PATH" "graph path"
CliGraphPath(@place("",0), path) :- sgc_module(_,_,_,path).
.violation CliDynamicRoot(at: span, sid: symbol) code "CLI_DYNAMIC_ROOT" "dynamic root"
CliDynamicRoot(@place("",0), sid) :- scli_dynamic_root(sid).
IDCGroup(@place("",0), category, i) :- scli_survey(), idc_category(i,category), !idc_source_unproven().
IDCItem(@place("",0), category, sid, i) :- scli_survey(), idc_sorted(category,sid,i), !idc_excluded(sid), !idc_source_unproven().
'''


def native_case(engine,rule,item):
    from replay import execute_workspace_context
    context={}
    def options(root):
        context['root']=str(root.resolve());return input_options(item,root)
    with process_environment(item):native=execute_workspace_context(engine,rule,item['files'],options)
    rows=native['diagnostics']
    def params(code):return [r['parameters'] for r in rows if r['code']==code]
    statuses=params('IDC_CLI_EXIT')
    output=sorted(params('IDC_REPORT_OUTPUT'),key=lambda r:(r['stage'],r['ordinal']))
    known={'CLI_DIRECTORY','CLI_GRAPH_PATH','CLI_DYNAMIC_ROOT','IDC_CLI_EXIT','IDC_REPORT_OUTPUT'}
    return dict(directories=[r['path'].replace(context['root'],'$ROOT') for r in sorted(params('CLI_DIRECTORY'),key=lambda r:r['ordinal'])],
                graphPaths=sorted(r['path'] for r in params('CLI_GRAPH_PATH')),
                dynamicRoots=sorted(r['sid'] for r in params('CLI_DYNAMIC_ROOT')),
                analysis=native_result([r for r in rows if r['code'] not in known]),
                text=''.join(r['text'] for r in output),stderr='',
                status=statuses[0]['status'] if len(statuses)==1 else None),native['programInputs']


def check_positives(rows):
    values={c['id']:c['expected'] for c in rows}
    if values['check-positive']['status']!=1 or values['check-clean']['status']!=0:raise ValueError('Missing CLI status counterexamples')
    if values['check-with-zero-limit']['status']!=1:raise ValueError('Output truncation must not affect status')
    if values['dynamic-function-comment']['dynamicRoots']!=['impl.a::live']:raise ValueError('Missing dynamic import positive')
    if 'impl.a::Widget.run' not in values['dynamic-class-methods']['dynamicRoots']:raise ValueError('Missing public method root')
    if values['dynamic-invalid-file']['dynamicRoots']==values['absolute-valid-candidates-skip-invalid']['dynamicRoots']:raise ValueError('Missing fallback distinction')
    if values['empty-dirs-no-dynamic-roots']['dynamicRoots']:raise ValueError('Empty CLI dirs differ from graph defaults')
    if values['empty-installation-defaults']['directories']:raise ValueError('Installation defaults must remain empty')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=name=='output')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:parser.error('Invalid partition')
    result=dict(format=FORMAT,sourcePin=PIN,fullMigrationAcceptance=False,cases=[],complete=False,passed=False)
    if args.command=='source':
        if git(args.source,'rev-parse','HEAD')!=PIN:parser.error('Pinned source required')
        result['sourceFiles']={name:sha((args.source/name).read_bytes()) for name in ['scripts/lint_lib/discovery.py','scripts/lint_lib/symbol_graph.py','scripts/lint_internal_dead_code.py']}
        write_json(args.output,result)
        for item in CASES:
            result['cases'].append(dict(item,expected=source_case(args.source,item)));write_json(args.output,result)
        check_positives(result['cases'])
    else:
        baseline=json.loads(args.baseline.read_text())
        if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Complete pinned recording required')
        if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:parser.error('Complete unchanged CLI corpus required')
        check_positives(baseline['cases'])
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),caseSelection=dict(part=args.part,parts=args.parts,total=len(CASES)))
        path=ROOT/'.rerail/rr-migration-20261106/internal-dead-code/cli'/f'part-{args.part}.rr';path.parent.mkdir(parents=True,exist_ok=True);path.write_text(program())
        write_json(args.output,result)
        for item in baseline['cases'][args.part::args.parts]:
            actual,inputs=native_case(args.engine,path,item);expected=item['expected']
            passed=same_result(expected['analysis'],actual['analysis']) and {k:v for k,v in actual.items() if k!='analysis'}=={k:v for k,v in expected.items() if k!='analysis'}
            result['cases'].append(dict(id=item['id'],expected=expected,actual=actual,programInputs=inputs,passed=passed));write_json(args.output,result)
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']));write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c.get('passed',True)])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
