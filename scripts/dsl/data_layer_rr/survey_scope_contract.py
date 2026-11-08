"""Record and compare graph inputs separately from native facade resources.

The caller supplies ordered scan paths. This tests workspace loading and scope
isolation, not production directory discovery or persistent workspace updates.
Native mode consumes recorded Python results and never calls the source scanner.
"""
import argparse
import json
from pathlib import Path
import re

from migration import PIN, ROOT, git, sha, write_json
from survey_source_contract import source_rows, native_rows
from survey_analysis_contract import source_result, native_result, same_result


def case(name, scans, resources):
    return dict(id=name, scanPaths=list(scans),
                files=[dict(path=p, source=t) for p,t in dict(scans, **resources).items()],
                extraRoots=[], excludedPaths=[])


IMPLEMENTATION={'impl/a.py':'def exported(): pass\ndef dead(): pass\n'}
CASES=[
    case('unscanned-facade-definition', IMPLEMENTATION,
         {'data_layer/api.py':'from impl.a import exported\ndef facade_only(): pass\n'}),
    case('empty-scan', {}, {'data_layer/api.py':'def facade_only(): pass\n'}),
    case('no-facade', IMPLEMENTATION, {}),
    case('nested-facade-excluded', IMPLEMENTATION,
         {'data_layer/nested/api.py':'from impl.a import dead\n'}),
    case('relative-package-import', {'data_layer/impl.py':'def exported(): pass\ndef dead(): pass\n'},
         {'data_layer/__init__.py':'from .impl import exported\ndef facade_only(): pass\n'}),
    case('explicit-duplicate-exports', IMPLEMENTATION,
         {'data_layer/api.py':'from impl.a import exported\n__all__ = ["exported", "exported"]\ndef facade_only(): pass\n'}),
    case('empty-all-blocks', IMPLEMENTATION,
         {'data_layer/api.py':'from impl.a import exported\n__all__ = []\n'}),
    case('import-over-local-definition', IMPLEMENTATION,
         {'data_layer/api.py':'from impl.a import exported\ndef exported(): pass\n'}),
    case('missing-export-target', IMPLEMENTATION,
         {'data_layer/api.py':'from missing import absent\ndef facade_only(): pass\n'}),
    case('facade-in-scan', dict(IMPLEMENTATION, **{'data_layer/api.py':'from impl.a import exported\ndef facade_only(): pass\n'}), {}),
    case('module-collision-package-last', {'impl/a.py':'def first(): pass\n', 'impl/a/__init__.py':'def last(): pass\n'},
         {'data_layer/api.py':'from impl.a import first, last\n'}),
    case('module-collision-file-last', {'impl/a/__init__.py':'def last(): pass\n', 'impl/a.py':'def first(): pass\n'},
         {'data_layer/api.py':'from impl.a import first, last\n'}),
    case('unicode-decorated-resource', {'impl/a.py':'class K:\n def method(self): pass\ndef dead(): pass\n'},
         {'data_layer/api.py':'from impl.a import K\n@decorate\ndef ﬁeld(): pass\n'}),
    case('facades-path-order', IMPLEMENTATION,
         {'data_layer/z.py':'from impl.a import dead as Zed\n', 'data_layer/a.py':'from impl.a import exported as Alpha\n'}),
    case('unscanned-local-definition-fq', {'data_layer/api/__init__.py':'def exported(): pass\n'},
         {'data_layer/api.py':'def exported(): pass\n'}),
]
FORMAT='rerail.survey-workspace-scopes/v1'


def program(layer, item):
    path=(ROOT/'test/fixtures/rr/survey-source.rr' if layer=='producer' else
          ROOT/'gate/data-layer/packs/data-layer/candidates/rr/internal-dead-code.rr')
    text=path.read_text()
    text=re.sub(r'^\.scope include .*\n\.scope exclude[^\n]*',
                '.scope dirs "__survey_no_primary__"', text, count=1, flags=re.M)
    text+='\n#include "data-layer/survey-scope.rr"\n'
    for i,path in enumerate(item['scanPaths']):
        text+=f'sgs_scan_path({json.dumps(path)}, {i}).\n'
        text+=f'sgs_graph_path({json.dumps(path)}).\n'
    return text


def check_positives(cases):
    by_id={c['id']:c for c in cases}
    first=by_id['unscanned-facade-definition']
    rows=first['producer']
    if not any(r['kind']=='graph_export' and r['sid']=='impl.a::exported' for r in rows):
        raise ValueError('Missing positive facade export')
    if any('facade_only' in str(r) for r in rows):
        raise ValueError('An unscanned facade definition entered the graph')
    if first['analysis']['unused_functions']!=['impl.a::dead']:
        raise ValueError('Missing unreachable positive or exported negative')
    if by_id['empty-scan']['producer'] or any(by_id['empty-scan']['analysis'].values()):
        raise ValueError('Empty scan unexpectedly acquired resource symbols')
    if not any(r['kind']=='graph_symbol' and r['sid']=='data_layer.api::facade_only'
               for r in by_id['facade-in-scan']['producer']):
        raise ValueError('Explicitly scanned facade lost its definitions')
    if not any(r['kind']=='graph_export' and r['sid']=='data_layer.api::exported'
               for r in by_id['unscanned-local-definition-fq']['producer']):
        raise ValueError('Unscanned facade definitions must resolve against scanned fqnames')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('command',choices=['source','native'])
    p.add_argument('--source',type=Path)
    p.add_argument('--engine',type=Path)
    p.add_argument('--baseline',type=Path)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--part',type=int,default=0)
    p.add_argument('--parts',type=int,default=1)
    args=p.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:p.error('Invalid shard')
    report=dict(format=FORMAT,sourcePin=PIN,cases=[],complete=False,passed=False)
    if args.command=='source':
        if not args.source or git(args.source,'rev-parse','HEAD')!=PIN:p.error('Pinned source required')
        for item in CASES:
            files={f['path']:f['source'] for f in item['files']}
            report['cases'].append(dict(item,producer=source_rows(args.source,files,item['scanPaths']),
                                        analysis=source_result(args.source,item)))
        check_positives(report['cases'])
    else:
        from replay import execute_workspace
        baseline=json.loads(args.baseline.read_text())
        if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:
            p.error('Complete pinned recording required')
        if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:p.error('Full unchanged corpus required')
        check_positives(baseline['cases'])
        report.update(baselineSha256=sha(args.baseline.read_bytes()),engineSha256=sha(args.engine.read_bytes()),
                      caseSelection=dict(part=args.part,parts=args.parts,total=len(CASES)))
        write_json(args.output,report)
        for item in baseline['cases'][args.part::args.parts]:
            result=dict(id=item['id'])
            for layer in ['producer','analysis']:
                rule=ROOT/'.rerail/rr-migration-20261106/internal-dead-code/scope'/f'{item["id"]}-{layer}.rr'
                rule.parent.mkdir(parents=True,exist_ok=True);rule.write_text(program(layer,item))
                native=execute_workspace(args.engine,rule,item['files'],
                                         {} if layer=='producer' else dict(entryPoint='analyze_dead_code'))
                actual=(native_rows if layer=='producer' else native_result)(native['diagnostics'])
                result[layer]=dict(expected=item[layer],actual=actual,
                                   passed=same_result(item[layer],actual) if layer=='analysis' else actual==item[layer],
                                   programInputs=native['programInputs'])
            result['passed']=all(result[layer]['passed'] for layer in ['producer','analysis'])
            report['cases'].append(result);write_json(args.output,report)
    report.update(complete=True,passed=all(c.get('passed',True) for c in report['cases']))
    write_json(args.output,report)
    print(json.dumps(dict(cases=len(report['cases']),passed=report['passed'],
                         failures=[c['id'] for c in report['cases'] if not c.get('passed',True)])))
    return 0 if report['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
