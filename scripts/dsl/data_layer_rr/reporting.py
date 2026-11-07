"""Compare complete plugin reporter values with native diagnostics and compiled metadata.

This covers snapshot reporting. It does not certify the production frontend or persistent workspaces.
"""
import argparse
import importlib
import json
from pathlib import Path
import sys
import tempfile

from migration import ROOT, PIN, git, sha, read_json, write_json, compare
from replay import execute_native


def reporter_file_groups(rows):
    """Ignore worker completion order between files; retain each file's complete ordered sequence."""
    return sorted(rows, key=lambda row: row['path'])


def run(args):
    harness_root = ROOT / 'scripts/dsl'
    harness_before = {p.resolve(): sha(p.read_bytes()) for p in harness_root.rglob('*.py')}
    corpus = read_json(args.cases)
    native = args.command == 'native'
    oracle = read_json(args.oracle) if native else None
    report = dict(format='rerail.rr-plugin-report-contract/v1', fullMigrationAcceptance=False,
        originalPytestCases=0, execution='native-snapshot' if native else 'offline-source', sourcePin=PIN,
        casesSha256=sha(args.cases.read_bytes()), results=[])
    modules = {}
    if native:
        engine = args.engine.resolve()
        report['engineSha256'] = sha(engine.read_bytes())
        report['oracleSha256'] = sha(args.oracle.read_bytes())
        if oracle['sourcePin'] != PIN or oracle['casesSha256'] != report['casesSha256'] or oracle['execution'] != 'offline-source':
            raise ValueError('Reporter oracle belongs to different source or cases')
        if any(row['status'] != 'passed' for row in oracle['results']) or len(oracle['results']) != len(corpus['cases']):
            raise ValueError('Incomplete reporter oracle')
    else:
        source = args.source.resolve()
        if git(source, 'rev-parse', 'HEAD') != PIN:
            raise ValueError('Wrong source pin')
        sys.path[:0] = [str(source), str(source / 'scripts')]
        modules = {check: importlib.import_module('scripts.lint_' + check) for check in {c['check'] for c in corpus['cases']}}
        from scripts.lint_lib.fact_engine import FactEngine
        from scripts.lint_lib.fact_store import JsonFactStore
    paths = [Path(__file__), *[Path(__file__).with_name(n) for n in ['migration.py', 'replay.py']]]
    report['harnessFiles'] = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in paths}
    for index, case in enumerate(corpus['cases']):
        if native:
            rule = args.rule_root / (case['check'].replace('_', '-') + '.rr')
            result = execute_native(engine, rule, case['files'], {})
            report['sourceDetectorModules'] = [name for name in sys.modules if name.startswith('scripts.lint_')]
            if report['sourceDetectorModules']:
                raise ValueError('The native reporter imported source detectors')
            metadata = result['metadata']
            if len(metadata['policies']) != 1:
                raise ValueError('Reporter contract requires one policy')
            policy = metadata['policies'][0]
            modes = [text for key, text in metadata['settings'] if key == 'mode']
            if modes != ['block']:
                raise ValueError('No source severity adapter for the compiled mode: ' + repr(modes))
            identity = policy['id']
            prefix = next((p for p in ['data-layer-', 'data-layer:'] if identity.startswith(p)), None)
            if prefix is None or any(r['policy'] != identity for r in result['diagnostics']):
                raise ValueError('Unexpected native policy identity')
            severities = {}
            for key, text in metadata['settings']:
                if key == 'severity':
                    words = text.split()
                    if len(words) != 2:
                        raise ValueError('Unsupported compiled severity metadata: ' + text)
                    severities[words[0]] = words[1]
            native_rows = result['diagnostics']
            if case['check'] == 'removed_compat_surfaces':
                native_rows = sorted(native_rows, key=lambda r: (r['path'], r['parameters']['ordinal']))
            actual = [dict(linter_name=identity.removeprefix(prefix).replace('-', '_'), code=r['code'],
                path=r['path'], line=r['line'], message=r['message'],
                severity=severities.get(r['clause'], 'hard-fail')) for r in native_rows]
            program = result['programInputs']
            phases = result['phases']
        else:
            with tempfile.TemporaryDirectory(prefix='rr-source-report-') as temporary:
                root = Path(temporary)
                for file in case['files']:
                    path = root / file['path']
                    if path.is_absolute() and not path.is_relative_to(root) or '..' in Path(file['path']).parts:
                        raise ValueError('Invalid reporter fixture path')
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(file['source'])
                module = modules[case['check']]
                plugin = getattr(module, case['check'] + '_plugin')(root)
                fact_engine = FactEngine(root=root, fact_store=JsonFactStore(root / 'tmp/lint_fact_store'),
                    extractors=getattr(module, case['check'] + '_fact_extractors')(root))
                view = fact_engine.materialize((plugin,))
                actual = [dict(linter_name=v.linter_name, code=v.code,
                    path=v.path.relative_to(root).as_posix() if v.path.is_absolute() else v.path.as_posix(),
                    line=v.line, message=v.message, severity=v.severity) for v in plugin.reporter(view)]
            program, phases = None, None
        # Keep the source reporter order. Adapt native rows to that interface's order.
        if native and case['check'] != 'removed_compat_surfaces':
            last = {'type_safety_patterns': 'message', 'sync_async_signatures': 'message', 'forwardref': 'code', 'data_layer_compat_aliases': 'code', 'hnsw_architecture': 'code'}[case['check']]
            actual.sort(key=lambda row: (row['path'], row['line'], row[last], *([row['message']] if case['check'] == 'hnsw_architecture' else [])))
        row = dict(id=case['id'], value=actual, status='passed', programInputs=program, phases=phases)
        if native:
            row['metadata'] = metadata
        report['results'].append(row)
        if native:
            expected = oracle['results'][index]
            if expected['id'] != case['id']:
                raise ValueError('Reporter case identity changed')
            try:
                if case['check'] in {'hnsw_architecture', 'removed_compat_surfaces'}:
                    row['comparisonContract'] = 'File groups are unordered (source FactEngine.as_completed); every in-file value, duplicate, and order is exact'
                    compare(reporter_file_groups(expected['value']), reporter_file_groups(actual))
                else:
                    compare(expected['value'], actual)
            except AssertionError as error:
                row.update(status='failed', expected=expected['value'])
                write_json(args.out, report)
                raise AssertionError(case['id'] + ': ' + str(error)) from error
    if native:
        if sha(engine.read_bytes()) != report['engineSha256']:
            raise ValueError('Reporter executable changed')
        report['sourceDetectorModules'] = [name for name in sys.modules if name.startswith('scripts.lint_')]
        if report['sourceDetectorModules']:
            raise ValueError('The native reporter imported source detectors')
    else:
        from data_layer_probe_record import verify_sources
        report['sourceFilesChecked'] = verify_sources(source, source, PIN)
        report['sourceFiles'] = {f'scripts/lint_{check}.py': sha((source / f'scripts/lint_{check}.py').read_bytes()) for check in modules}
    for imported in tuple(sys.modules.values()):
        filename = getattr(imported, '__file__', None)
        if filename and Path(filename).suffix == '.py':
            path = Path(filename).resolve()
            if path.is_relative_to(harness_root):
                if path not in harness_before:
                    raise ValueError('Reporter imported new code during the run')
                report['harnessFiles'].setdefault(path.relative_to(ROOT).as_posix(), harness_before[path])
    if any(sha((ROOT / path).read_bytes()) != digest for path, digest in report['harnessFiles'].items()):
        raise ValueError('Reporter harness changed')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    parser.add_argument('--source', type=Path)
    parser.add_argument('--engine', type=Path)
    parser.add_argument('--oracle', type=Path)
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--rule-root', type=Path, default=ROOT / 'packs/data-layer/candidates/rr',
                        help='Policy directory; mutation controls supply disabled policies')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.rule_root = args.rule_root.resolve()
    report = run(args)
    write_json(args.out, report)
    print(json.dumps(dict(cases=len(report['results']), execution=report['execution'], fullMigrationAcceptance=False)))


if __name__ == '__main__':
    main()
