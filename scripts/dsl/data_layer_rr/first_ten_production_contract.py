"""Compare production discovery, complete fact payloads, metadata, and findings."""
import argparse
import contextlib
from dataclasses import asdict
import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_workspace, write_fixture

CHECKS = ['removed_compat_surfaces', 'versioned_lifecycle_legacy_surfaces', 'no_api_root_imports',
          'backend_type_strings', 'hasattr_async', 'no_source_inspection', 'serverplan_name_inference',
          'no_blanket_grammar_bypass', 'serverplan_grammar_surface', 'permission_error']
ROOTS = ['api', 'impl', 'data_layer', '_internal', 'l1234_sample', 'l١٢٣٤_sample', 'l1234_', 'l123_bad',
         'l2070_batching', 'l2060_query_builder', 'l1100_exceptions', 'l4100_queue', 'l7000_versioned_schema',
         'other', 'tests', 'conform']


def cases(recording):
    result = []
    for check in CHECKS:
        witness = next(e['files'][0]['source'] for e in recording['events']
                       if e['check'] == check and e.get('findings') and e.get('files'))
        directory = {'hasattr_async': 'data_layer', 'no_source_inspection': 'tests',
                     'serverplan_name_inference': 'l2070_batching'}.get(check, 'impl')
        def file(path, source=witness): return dict(path=path, source=source)
        groups = [
            ('empty', []),
            ('clean', [file(p+'/a.py', 'pass\n') for p in ROOTS]),
            ('roots', [file(p+'/a.py') for p in ROOTS]),
            ('nested', [file(directory+'/'+p+'/a.py') for p in ['normal', 'tests', 'test', 'conform',
                'venv', '.venv', 'tmp', '__pycache__', '.git', '.hidden', 'node_modules', '.pytest_cache']]),
            ('comment-trigger', [file(directory+'/a.py', '# '+repr(witness)+'\n')]),
            ('syntax', [file(directory+'/a.py', 'def :\n'+witness)]),
            ('ordering', [file(directory+'/'+p) for p in ['z.py', 'a/z.py', 'a-.py', 'a.py']]),
            ('extensions', [file(directory+'/'+p) for p in ['.py', '.hidden.py', 'a.py', 'a.PY', 'a.pyi', 'notes.txt']]),
            ('file-links', [file(directory+'/a.py'), dict(path=directory+'/alias.py', symlink='a.py')]),
            ('directory-links', [file(directory+'/sub/a.py'), dict(path=directory+'/link', symlink='sub')]),
            ('duplicate-findings', [file(directory+'/a.py', witness+'\n'+witness)]),
            ('other-owner', [file('scripts/lint_hasattr_async.py'), file('impl/lint_hasattr_async.py'),
                file('impl/common/l2200_sql_dialect/sql_connection.py', 'class Conn:\n    def execute_grammar(self): pass\n'),
                file('impl/sqlite/sql_column_type_hints.py', 'if column.endswith("_id"): pass\n')]),
        ]
        result.extend(dict(id=check+':'+name, check=check, files=files) for name, files in groups)
    return result


def normalize(value, root):
    if isinstance(value, dict): return {str(k).replace(str(root), '$ROOT'): normalize(v, root) for k, v in value.items()}
    if isinstance(value, (list, tuple)): return [normalize(v, root) for v in value]
    if isinstance(value, (str, Path)): return str(value).replace(str(root), '$ROOT')
    return value


def canonical_findings(check, rows):
    # This collector returns file groups in FactEngine.as_completed order.
    # Preserve every in-file element and duplicate; only whole groups may permute.
    if check != 'removed_compat_surfaces': return rows
    groups = {}
    for row in rows: groups.setdefault(row['path'], []).append(row)
    return [row for path in sorted(groups) for row in groups[path]]


def source_case(module, case):
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    check = case['check']
    with tempfile.TemporaryDirectory(prefix='first-ten-production-') as temporary, contextlib.ExitStack() as stack:
        root = Path(temporary); write_fixture(root, case['files'])
        for name in ['ROOT', '_REPO_ROOT']:
            if hasattr(module, name): stack.enter_context(patch.object(module, name, root))
        if hasattr(module, '_SCAN_ROOTS'):
            stack.enter_context(patch.object(module, '_SCAN_ROOTS', tuple(root/p for p in module._SCAN_DIRECTORIES)))
        if check == 'serverplan_grammar_surface':
            from scripts.lint_lib.serverplan_facts import serverplan_fact_extractors
            plugin = module.serverplan_grammar_surface_plugin(); extractors = serverplan_fact_extractors(root)
        else:
            plugin = getattr(module, check+'_plugin')(root); extractors = getattr(module, check+'_fact_extractors')(root)
        engine = FactEngine(root=root, fact_store=JsonFactStore(root/'tmp/facts'), extractors=extractors, enable_fact_cache=False)
        plan = engine.plan([plugin]); view = engine.materialize((plugin,), max_workers=1)
        return normalize(dict(selectedFiles=[p.relative_to(root).as_posix() for p in plan.files],
            facts={p.relative_to(root).as_posix(): b.facts for p, b in sorted(view.files.items())},
            findings=canonical_findings(check, [asdict(v) for v in plugin.reporter(view)]),
            plugin=dict(name=plugin.name, enforcement=plugin.enforcement, description=plugin.description)), root)


def native_case(engine, case):
    check = case['check']; policy = ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/(check.replace('_', '-')+'.rr')
    result = execute_workspace(engine, policy, case['files'], dict(entryPoint='_production_contract'))
    rows = result['diagnostics']; meta = {r['parameters']['key']: r['parameters']['value'] for r in rows if r['code'] == 'DL_META'}
    files = sorted((r['parameters'] for r in rows if r['code'] == 'DL_FILE'), key=lambda p: p['path_order'])
    facts = {p['path']: {} for p in files}
    for p in files:
        if p['fact_present']:
            facts[p['path']][meta['extractor']] = ([] if p['fact_kind'] == 'list' else dict(path=p['path'],
                classes=[], functions=[], compile_methods=[], grammar_surface_hits=[], syntax_error=None))
    for row in sorted((r for r in rows if r['code'] == 'DL_FACT'), key=lambda r: tuple(r['parameters'][k] for k in ['path_order', 'order0', 'order1', 'order2'])):
        p = row['parameters']; payload = {k.removeprefix('f_'): v for k, v in p.items() if k.startswith('f_')}
        target = facts[p['path']][meta['extractor']]
        if isinstance(target, list): target.append(payload)
        else: target['grammar_surface_hits'].append(payload)
    findings = []
    for row in sorted((r for r in rows if r['code'] == 'DL_REPORT'), key=lambda r: tuple(r['parameters'][k] for k in ['order0', 'order1', 'order2', 'order3'])):
        p = row['parameters']; findings.append(dict(linter_name=meta['name'], code=p['source_code'], path=p['file'],
            line=p['source_line'], message=p['source_message'], severity=meta['findingSeverity']))
    actual = dict(selectedFiles=[p['path'] for p in files], facts=facts, findings=canonical_findings(check, findings),
                  plugin={k: meta.get(k) for k in ['name', 'enforcement', 'description']})
    return normalize(actual, meta.get('root', '<unbound-root>')), result


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('command', choices=['source', 'native'])
    for name in ['source', 'engine', 'baseline', 'out']:
        parser.add_argument('--'+name, type=Path, required=name in ['source', 'out'])
    parser.add_argument('--recording', type=Path, default=ROOT/'.rerail/rr-batch1-final93/source.json')
    parser.add_argument('--part', type=int, default=0); parser.add_argument('--parts', type=int, default=1)
    args = parser.parse_args(); source = args.source.absolute(); compare(git(source, 'rev-parse', 'HEAD'), PIN)
    corpus = cases(read_json(args.recording)); assert 0 <= args.part < args.parts
    paths = [Path(__file__), *[Path(__file__).with_name(n) for n in ['replay.py', 'migration.py']]]
    hashes = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in paths}
    source_files = {}
    for check in CHECKS:
        path = source/('scripts/lint_'+check+'.py')
        compare(path.read_bytes(), subprocess.check_output(['git', '-C', str(source), 'show', PIN+':'+path.relative_to(source).as_posix()]))
        source_files[check] = sha(path.read_bytes())
    if args.command == 'source':
        assert (args.part, args.parts) == (0, 1)
        sys.path[:0] = [str(source), str(source/'scripts')]
        modules = {c: importlib.import_module('scripts.lint_'+c) for c in CHECKS}
    else:
        baseline = read_json(args.baseline); assert baseline['passed']; compare(source_files, baseline['sourceFiles'])
        compare(corpus, [r['input'] for r in baseline['cases']])
        assert not any(n.startswith('scripts.lint_') for n in sys.modules)
    report = dict(format='rerail.first-ten-production-contract/v1', sourcePin=PIN, execution=args.command,
        sourceFiles=source_files, harnessFiles=hashes, passed=False, fullMigrationAcceptance=False,
        part=args.part, parts=args.parts, totalCases=len(corpus), cases=[])
    if args.command == 'native': report['engineSha256'] = sha(args.engine.read_bytes())
    for index in range(args.part, len(corpus), args.parts):
        case = corpus[index]; row = dict(index=index, input=case)
        if args.command == 'source': row['expected'] = source_case(modules[case['check']], case)
        else:
            actual, result = native_case(args.engine.absolute(), case)
            row.update(expected=baseline['cases'][index]['expected'], actual=actual, native=result,
                       passed=actual == baseline['cases'][index]['expected'])
        report['cases'].append(row); write_json(args.out, report)
    for path, digest in hashes.items(): compare(digest, sha((ROOT/path).read_bytes()))
    report['passed'] = all(row.get('passed', True) for row in report['cases']); write_json(args.out, report)
    print(dict(cases=len(report['cases']), passed=report['passed'])); raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__': main()
