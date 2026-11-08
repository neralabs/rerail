"""Compare native source discovery, analysis, and complete report bytes with the pinned scanner."""
import argparse
import contextlib
import copy
import importlib
import io
import json
from pathlib import Path
import sys
import tempfile

from migration import PIN, ROOT, git, sha, write_json
from replay import write_fixture
from survey_discovery_contract import CASES as DISCOVERY_CASES, case, file, process_environment, program as discovery_program
from survey_analysis_contract import native_result, same_result


FORMAT='rerail.survey-source-reporting/v1'


def cases():
    weighted=copy.deepcopy(next(c for c in DISCOVERY_CASES if c['id']=='weighted-worker-order'))
    inputs=[case('empty',[],['impl']),
            case('nested',[file('impl/a.py','class Outer:\n class Nested: pass\n def method(self): pass\ndef outer():\n def inner(): pass\n return inner\n')],['impl']),
            case('unicode',[file('impl/é😀.py','class É:\n def méthode(self): pass\ndef façade(): pass\n')],['impl']),
            case('category-overlap',[file('impl/a.py','class SqlIntent(SqlGrammarBase): pass\nclass Other: pass\ndef dead(): pass\n')],['impl']),
            case('parse-failure',[file('impl/a.py'),file('impl/bad.py','def bad(\n')],['impl']),
            weighted]
    result=[]
    for item in inputs:
        for mode in ['text','json']:
            result.append(dict(copy.deepcopy(item),id='source-report-'+item['id']+'-'+mode,mode=mode,limit=1,excludedPaths=[]))
    excluded=copy.deepcopy(result[2]);excluded.update(id='source-report-excluded',excludedPaths=['impl/a.py'])
    result.append(excluded)
    negative=copy.deepcopy(result[2]);negative.update(id='source-report-negative-limit',limit=-1)
    result.append(negative)
    return result


CASES=cases()


def source_case(source,item):
    sys.path[:0]=[str(source),str(source/'scripts')]
    discovery=importlib.import_module('lint_lib.discovery')
    model=importlib.import_module('lint_lib.symbol_graph')
    linter=importlib.import_module('lint_internal_dead_code')
    with process_environment(item,python=True),tempfile.TemporaryDirectory(prefix='rerail-offline-source-report-') as temporary:
        root=Path(temporary);write_fixture(root,item['files'])
        paths=discovery.discover_python_files(root,item['directories'],include_tests=False)
        ordered=paths
        if len(paths)>50:
            workers=model.linter_worker_count(default_cap=8)
            ordered=[p for chunk in model._partition_symbol_scan_files(paths,worker_count=max(workers,(len(paths)+15)//16)) for p in chunk]
        graph=model.SymbolGraph(root,item['directories']);graph._scan(files=paths)
        graph._resolve_edges();graph._add_legacy_decorator_edges();graph._collect_data_layer_roots()
        analysis=linter.analyze_dead_code(graph,excluded_paths=tuple(item['excludedPaths']))
        output=io.StringIO()
        with contextlib.redirect_stdout(output):
            if item['mode']=='text':linter._print_text(analysis,graph,limit=item['limit'])
            else:linter._print_json(analysis,graph)
        return dict(paths=[p.relative_to(root).as_posix() for p in paths],assemblyPaths=[p.relative_to(root).as_posix() for p in ordered],analysis=analysis,text=output.getvalue())


def program():
    return discovery_program()+'''
// Observe the production analysis relation independently of its report formatter.
IDCGroup(@place("",0), category, i) :- idc_category(i, category), !idc_source_unproven().
IDCItem(@place("",0), category, sid, i) :- idc_sorted(category, sid, i), !idc_excluded(sid), !idc_source_unproven().
'''


def native_case(engine,rule,item):
    from replay import execute_workspace
    options=dict(entryPoint='report_'+item['mode'],reportLimit=str(item['limit']),excludedPaths=item['excludedPaths'],
                 env=[f'data-layer.scan.{i}={path}' for i,path in enumerate(item['directories'])])
    with process_environment(item):native=execute_workspace(engine,rule,item['files'],options)
    diagnostics=native['diagnostics']
    def paths(code):
        return [r['parameters']['path'] for r in sorted((r for r in diagnostics if r['code']==code),key=lambda r:r['parameters']['ordinal'])]
    text=''.join(r['text'] for r in sorted((r['parameters'] for r in diagnostics if r['code']=='IDC_REPORT_OUTPUT'),key=lambda r:(r['stage'],r['ordinal'])))
    known={'DISCOVERED_PATH','ASSEMBLY_PATH','DEMANDED_PATH','IDC_REPORT_OUTPUT'}
    analysis=native_result([r for r in diagnostics if r['code'] not in known])
    return dict(paths=paths('DISCOVERED_PATH'),assemblyPaths=paths('ASSEMBLY_PATH'),analysis=analysis,text=text),native['programInputs']


def check_positives(rows):
    by_id={c['id']:c['expected'] for c in rows}
    if not by_id['source-report-nested-text']['analysis']['unused_functions']:raise ValueError('Missing source-report positive findings')
    if 'méthode' not in by_id['source-report-unicode-text']['text']:raise ValueError('Unicode source qualifier lost')
    if '\\ud83d\\ude00' not in by_id['source-report-unicode-json']['text']:raise ValueError('ASCII JSON escaping lost')
    if by_id['source-report-excluded']['analysis']['unused_functions']:raise ValueError('Excluded source symbols leaked into the report')
    if len(by_id['source-report-weighted-worker-order-text']['paths'])!=52:raise ValueError('Large scanner report case lost')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=name=='output')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if not 0<=args.part<args.parts:parser.error('Invalid partition')
    result=dict(format=FORMAT,sourcePin=PIN,fullMigrationAcceptance=False,cases=[],complete=False,passed=False)
    if args.command=='source':
        if git(args.source,'rev-parse','HEAD')!=PIN:parser.error('Pinned source required')
        result['sourceFiles']={name:sha((args.source/name).read_bytes()) for name in ['scripts/lint_lib/discovery.py','scripts/lint_lib/symbol_graph.py','scripts/lint_internal_dead_code.py']}
        for item in CASES:result['cases'].append(dict(item,expected=source_case(args.source,item)))
        check_positives(result['cases'])
    else:
        baseline=json.loads(args.baseline.read_text())
        if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Complete pinned recording required')
        if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:parser.error('Complete unchanged source-report cases required')
        check_positives(baseline['cases'])
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),caseSelection=dict(part=args.part,parts=args.parts,total=len(CASES)))
        write_json(args.output,result)
        path=ROOT/'.rerail/rr-migration-20261106/internal-dead-code/reporting'/f'source-part-{args.part}.rr';path.parent.mkdir(parents=True,exist_ok=True);path.write_text(program())
        for item in baseline['cases'][args.part::args.parts]:
            actual,inputs=native_case(args.engine,path,item);expected=item['expected']
            passed=actual['paths']==expected['paths'] and actual['assemblyPaths']==expected['assemblyPaths'] and actual['text']==expected['text'] and same_result(expected['analysis'],actual['analysis'])
            result['cases'].append(dict(id=item['id'],expected=expected,actual=actual,programInputs=inputs,passed=passed));write_json(args.output,result)
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']))
    write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c.get('passed',True)])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
