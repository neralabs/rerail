"""Record complete plugin reports from caller-supplied fact bundles."""
import argparse
from dataclasses import asdict
import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from first_ten_production_contract import CHECKS, normalize
from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_workspace
from fact_transport import encode_bundles


def cases():
    result = []
    for check in CHECKS:
        defaults = dict(name='gone', path='impl/a.py', file_path='impl/a.py', file='impl/a.py',
            line=4, code='CUSTOM', message='message', pattern_name='inspect.custom()',
            pattern_type='hasattr', method_name='custom_async')
        if check == 'serverplan_grammar_surface':
            values = [None, [], 'wrong-container', {}, dict(path='impl/a.py'),
                dict(path='impl/a.py', syntax_error='supplied parser error'),
                dict(path='impl/a.py', grammar_surface_hits=[dict(kind='helper_definition', line=4,
                     symbol='grammar_execute', message='supplied message')])]
        else:
            values = [None, {}, 'wrong-container', 0, [], [None, 0, 'wrong-row', []], [{}], [defaults],
                [defaults, defaults], [dict(defaults, line=value) for value in [None, True, False, 2.5, '3', -2, 0,
                    2**100, -(2**100), 2**63-1, -(2**63), -100, -99]],
                [dict(defaults, message=value, name=value, pattern_name=value, code=value)
                 for value in [None, True, 2.5, ['☃', 1], {'a': 1}]],
                [dict(defaults, file_path=p, file=p, path=p) for p in ['a/../b.py', '/absolute.py', '', '//network/a.py',
                    './a//././b.py/.', '///a/b.py/', '//', '/', '.']],
                [None, {}, [], defaults, defaults],
                [dict(defaults, message=m) for m in ['z', 'a', 'a']]]
        for i, value in enumerate(values):
            result.append(dict(id=check+':'+str(i), check=check,
                bundles=[dict(path='impl/a.py', value=value)]))
        if check != 'serverplan_grammar_surface':
            result.append(dict(id=check+':file-order', check=check, bundles=[
                dict(path=p, value=[dict(defaults, line=8, file_path=p, file=p, path=p),
                                    dict(defaults, line=2, file_path=p, file=p, path=p)])
                for p in ['a/z.py', 'a-.py', 'z.py']]))
            result.append(dict(id=check+':mixed-containers', check=check, bundles=[
                dict(path='a.py', value={'bad-container':defaults}),
                dict(path='b.py', value=[None, [], defaults]), dict(path='c.py', value=None)]))
    return result


def source_case(module, case):
    from scripts.lint_lib.linter_plugin import PerFileFactBundle, ProjectFactView
    with tempfile.TemporaryDirectory(prefix='supplied-facts-') as temporary:
        root = Path(temporary)
        with patch.object(module, 'ROOT', root, create=True):
            plugin = module.serverplan_grammar_surface_plugin() if case['check'] == 'serverplan_grammar_surface' else getattr(module, case['check']+'_plugin')(root)
            spec = plugin.required_facts[0]
            bundles = {root/b['path']: PerFileFactBundle(root/b['path'], 'test-digest', facts={spec.extractor_id:b['value']})
                       for b in case['bundles']}
            view = ProjectFactView(root, bundles, plugin.required_facts)
            try: value = dict(findings=[asdict(row) for row in plugin.reporter(view)])
            except Exception as error:
                value = dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__, args=list(error.args)))
                if hasattr(error, 'errors'): value['error']['details'] = error.errors(include_url=False, include_context=False)
            return normalize(value, root)


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('command', choices=['source', 'native', 'controls'])
    parser.add_argument('--source', type=Path)
    parser.add_argument('--engine', type=Path); parser.add_argument('--baseline', type=Path)
    parser.add_argument('--check'); parser.add_argument('--part', type=int, default=0); parser.add_argument('--parts', type=int, default=1)
    parser.add_argument('--out', type=Path, required=True); args = parser.parse_args()
    if not 0 <= args.part < args.parts: parser.error('Expected 0 <= part < parts')
    dependencies = [Path(__file__), *[Path(__file__).with_name(p) for p in [
        'fact_transport.py', 'first_ten_production_contract.py', 'replay.py', 'migration.py']]]
    hashes = {p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in dependencies}
    if args.command in {'native', 'controls'}:
        baseline = read_json(args.baseline); compare(baseline['sourcePin'], PIN)
        compare(baseline['passed'], True)
        compare([r['input'] for r in baseline['cases']], cases())
        report = dict(format='rerail.first-ten-supplied-fact-'+args.command+'/v1', sourcePin=PIN, passed=False,
            fullMigrationAcceptance=False, baselineSha256=sha(args.baseline.read_bytes()),
            harnessFiles=hashes, engineSha256=sha(args.engine.read_bytes()), part=args.part, parts=args.parts, cases=[])
        selected = [r for r in baseline['cases'] if not args.check or r['input']['check'] in args.check.split(',')]
        if args.command == 'controls': selected = [r for r in selected if r['input']['id'] == r['input']['check']+':7']
        for row in selected[args.part::args.parts]:
            if args.command == 'controls':
                relative = Path('gate/data-layer/packs/data-layer/candidates/rr')/(row['input']['check'].replace('_', '-')+'.rr')
                original = (ROOT/relative).read_text()
                changed = original+'\nsf_bad(b,r) :- sf_row(b,r).\n'
                with tempfile.TemporaryDirectory(prefix='supplied-fact-control-', dir=ROOT/'.rerail') as folder:
                    target = Path(folder)/relative; target.parent.mkdir(parents=True); target.write_text(changed)
                    with patch(__name__+'.ROOT', Path(folder)):
                        actual, execution = native_case(args.engine, row['input'])
                compare(bool(row['expected'].get('findings')), True)
                compare(actual == row['expected'], False)
            else: actual, execution = native_case(args.engine, row['input'])
            item = dict(id=row['input']['id'], expected=row['expected'], actual=actual, execution=execution)
            if args.command == 'controls': item.update(mutatedPolicy=changed, originalPolicySha256=sha(original.encode()), rejected=True)
            report['cases'].append(item); write_json(args.out, report)
            # JSON equality distinguishes bool from int, unlike Python's ==.
            if args.command == 'native':
                compare(json.dumps(row['expected'], sort_keys=True, ensure_ascii=False),
                        json.dumps(actual, sort_keys=True, ensure_ascii=False))
        for p,digest in hashes.items(): compare(digest, sha((ROOT/p).read_bytes()))
        report['passed'] = True; write_json(args.out, report); print(dict(cases=len(report['cases']),passed=True)); return
    source = args.source.absolute()
    compare(git(source, 'rev-parse', 'HEAD'), PIN); sys.path[:0] = [str(source),str(source/'scripts')]
    modules = {c:importlib.import_module('scripts.lint_'+c) for c in CHECKS}; hashes = {}
    for c in CHECKS:
        path = source/('scripts/lint_'+c+'.py')
        compare(path.read_bytes(), subprocess.check_output(['git','-C',str(source),'show',PIN+':'+path.relative_to(source).as_posix()]))
        hashes[c] = sha(path.read_bytes())
    report = dict(format='rerail.first-ten-supplied-fact-source/v1', sourcePin=PIN, passed=False,
        fullMigrationAcceptance=False, sourceFiles=hashes, harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in dependencies}, cases=[])
    for case in cases():
        report['cases'].append(dict(input=case, expected=source_case(modules[case['check']],case)))
    report['passed'] = True; write_json(args.out,report); print(dict(cases=len(report['cases']),passed=True))


def native_case(engine, case):
    if case['check']=='serverplan_grammar_surface':
        from surface_reporter_port import native_case as surface_case
        return surface_case(engine,case)
    policy = ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/(case['check'].replace('_', '-')+'.rr')
    execution = execute_workspace(engine, policy,
        [dict(path='reporter-facts.json', source=json.dumps(dict(bundles=encode_bundles(case['bundles'])), ensure_ascii=False))],
        dict(entryPoint='_report_facts'))
    rows = execution['diagnostics']
    roots = [r['parameters']['root'] for r in rows if r['code'] == 'SF_ROOT']
    compare(len(roots), 1)
    findings = []
    for row in sorted((r for r in rows if r['code'] == 'SF_REPORT'),
            key=lambda r: tuple(r['parameters'][k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal'])):
        p = row['parameters']
        findings.append(dict(linter_name=case['check'], code=p['source_code'], path=p['file'],
            line=json.loads(p['line_json']), message=p['source_message'], severity=p['severity']))
    return normalize(dict(findings=findings), roots[0]), execution


if __name__ == '__main__': main()
