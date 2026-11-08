"""Compare production discovery and dead-code categories with the pinned source.

Native mode creates the physical fixture but supplies no selected paths to the
engine. Directories, links, exclusions, ordering, and graph inputs come from .rr.
"""
import argparse
import ast
import importlib
import json
from pathlib import Path
import re
import sys
import tempfile

from migration import PIN, ROOT, git, sha, write_json
from replay import write_fixture
from survey_analysis_contract import native_result, same_result


def file(path, source='def dead(): pass\n'):return dict(path=path,source=source)
def directory(path):return dict(path=path,directory=True)
def link(path,target):return dict(path=path,symlink=target)
def case(name, files, dirs=None):return dict(id=name,files=files,directories=dirs)


CASES=[
    case('legacy-default',[file('impl/b.py'),file('api/a.py'),file('other/a.py')]),
    case('modern-base-and-layers',[file('api/ignored.py'),file('_internal/a.py'),file('data_layer/a.py'),
         file('impl/a.py'),file('l9000_last/a.py'),file('l1000_first/a.py'),file('l1000_/ignored.py')]),
    case('empty-modern-root',[directory('data_layer'),file('api/ignored.py')]),
    case('skip-trees',[file('impl/kept.py')]+[file(f'impl/{name}/ignored.py') for name in
         ['__pycache__','.pytest_cache','node_modules','.git','.venv','tmp','browser_build','tests','test','conform']]),
    case('explicit-files-bypass-exclusions',[file('tests/one.py'),file('tmp/two.py'),file('notes.txt')],
         ['tests/one.py','tmp/two.py','notes.txt','missing.py']),
    case('explicit-repeated-root',[file('impl/a.py')],['impl','impl']),
    case('public-expansion-deduplicates',[file('impl/a.py'),file('data_layer/api.py'),file('_internal/a.py')],
         ['public','impl','public','_internal']),
    case('empty-options-use-public',[file('api/a.py'),file('impl/a.py')],[]),
    case('nested-directory-link-pruned',[file('pkg/kept.py'),file('other/hidden.py'),link('pkg/alias','../other')],['pkg']),
    case('explicit-directory-link-root',[file('other/visible.py'),link('alias','other')],['alias']),
    case('broken-explicit-file-link',[link('missing.py','absent.py'),file('impl/a.py')],['missing.py','impl']),
    case('component-path-order',[file('impl/a.py'),file('impl/a/z.py'),file('impl/a-/x.py'),file('impl/a_/y.py')]),
    case('suffix-and-hidden-files',[file(f'impl/{name}') for name in ['.py','..py','...py','.hidden.py','normal.py','upper.PY']]),
    case('facade-resource-outside-scan',[file('impl/a.py','def exported(): pass\ndef dead(): pass\n'),
         file('data_layer/api.py','from impl.a import exported\ndef facade_only(): pass\n')],['impl']),
    case('directory-cycle-pruned',[file('pkg/kept.py'),link('pkg/back','..')],['pkg']),
    case('normalized-relative-root',[file('impl/a.py')],['./impl//']),
]
FORMAT='rerail.survey-production-discovery/v1'


def source_case(source,item):
    sys.path[:0]=[str(source),str(source/'scripts')]
    discovery=importlib.import_module('lint_lib.discovery')
    model=importlib.import_module('lint_lib.symbol_graph')
    linter=importlib.import_module('lint_internal_dead_code')
    with tempfile.TemporaryDirectory(prefix='rerail-offline-discovery-') as temporary:
        root=Path(temporary);write_fixture(root,item['files'])
        paths=discovery.discover_python_files(root,item['directories'] or ['public'],include_tests=False)
        graph=model.SymbolGraph(root,item['directories'])
        scans=[]
        for path in paths:
            try:tree=ast.parse(path.read_text())
            except (OSError,SyntaxError,UnicodeDecodeError):continue
            scans.append(model._scan_symbol_tree(root,path,tree))
        graph._add_scanned_modules(scans);graph._resolve_edges();graph._add_legacy_decorator_edges();graph._collect_data_layer_roots()
        return dict(paths=[p.relative_to(root).as_posix() for p in paths],analysis=linter.analyze_dead_code(graph))


def program():
    text=(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/internal-dead-code.rr').read_text()
    text=re.sub(r'^\.scope include .*\n\.scope exclude[^\n]*','.scope dirs "__survey_no_primary__"',text,count=1,flags=re.M)
    return text+'''
#include "data-layer/survey-discovery.rr"
.violation DiscoveredPath(at: span, ordinal: number, path: symbol) code "DISCOVERED_PATH" "discovered"
DiscoveredPath(@place("", 0), i, path) :- sgs_scan_path(path, i).
'''


def native_case(engine,rule,item):
    from replay import execute_workspace
    options=dict(entryPoint='analyze_dead_code')
    if item['directories'] is not None:options['scanDirectories']=item['directories']
    native=execute_workspace(engine,rule,item['files'],options)
    discovered=sorted((r['parameters'] for r in native['diagnostics'] if r['code']=='DISCOVERED_PATH'),key=lambda r:r['ordinal'])
    result=dict(paths=[r['path'] for r in discovered],analysis=native_result([r for r in native['diagnostics'] if r['code']!='DISCOVERED_PATH']))
    return result,native['programInputs']


def check_positives(cases):
    by_id={c['id']:c['expected'] for c in cases}
    if by_id['nested-directory-link-pruned']['paths']!=['pkg/kept.py']:raise ValueError('Directory link must be pruned')
    if by_id['explicit-directory-link-root']['paths']!=['alias/visible.py']:raise ValueError('Explicit link root must be scanned')
    if by_id['explicit-repeated-root']['paths']!=['impl/a.py','impl/a.py']:raise ValueError('Explicit root multiplicity lost')
    if by_id['public-expansion-deduplicates']['paths']!=['impl/a.py','data_layer/api.py','_internal/a.py']:raise ValueError('Public expansion must deduplicate')
    scoped=by_id['facade-resource-outside-scan']
    if scoped['paths']!=['impl/a.py'] or scoped['analysis']['unused_functions']!=['impl.a::dead']:raise ValueError('Missing export negative or unreachable positive')
    if by_id['component-path-order']['paths']!=['impl/a/z.py','impl/a-/x.py','impl/a.py','impl/a_/y.py']:raise ValueError('Path component order lost')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:p.add_argument('--'+name,type=Path,required=name=='output')
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1)
    args=p.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:p.error('Invalid partition')
    result=dict(format=FORMAT,sourcePin=PIN,fullMigrationAcceptance=False,cases=[],complete=False,passed=False)
    if args.command=='source':
        if not args.source or git(args.source,'rev-parse','HEAD')!=PIN:p.error('Pinned source required')
        result['sourceFiles']={name:sha((args.source/name).read_bytes()) for name in ['scripts/lint_lib/discovery.py','scripts/lint_lib/symbol_graph.py','scripts/lint_internal_dead_code.py']}
        for item in CASES:result['cases'].append(dict(item,expected=source_case(args.source,item)))
        check_positives(result['cases'])
    else:
        baseline=json.loads(args.baseline.read_text())
        if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:p.error('Complete pinned recording required')
        if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:p.error('Complete unchanged corpus required')
        check_positives(baseline['cases'])
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),caseSelection=dict(part=args.part,parts=args.parts,total=len(CASES)))
        write_json(args.output,result)
        rule=ROOT/'.rerail/rr-migration-20261106/internal-dead-code/discovery'/f'part-{args.part}.rr';rule.parent.mkdir(parents=True,exist_ok=True);rule.write_text(program())
        for item in baseline['cases'][args.part::args.parts]:
            actual,inputs=native_case(args.engine,rule,item)
            result['cases'].append(dict(id=item['id'],expected=item['expected'],actual=actual,programInputs=inputs,
                                       passed=actual['paths']==item['expected']['paths'] and same_result(item['expected']['analysis'],actual['analysis'])))
            write_json(args.output,result)
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']))
    write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c.get('passed',True)])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
