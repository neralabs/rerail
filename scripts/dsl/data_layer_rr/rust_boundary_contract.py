"""Source-derived contracts for a rule that has no dedicated upstream pytest suite.

`source` records unchanged Python APIs offline. `native` never imports that detector.
"""
import argparse
import importlib
import json
from pathlib import Path
import sys
import tempfile
import subprocess

from dto import canonical
from migration import ROOT, PIN, git, sha, read_json, write_json, compare, fixtures_path
from replay import execute_native

CHECK = 'rust_sql_grammar_boundary'
RULE = ROOT / 'packs/data-layer/candidates/rr/rust-sql-grammar-boundary.rr'


def cases():
    previous = read_json(fixtures_path('host', 'data-layer-port', 'batch-04-probes.json'))
    result = [c for c in previous['cases'] if c['check'] == CHECK]
    # Raw source scanning includes comments, malformed Rust, Unicode boundaries and every occurrence's line.
    for name, source in [
        ('duplicates-per-line', 'pyo3 pyo3 PyResult PyResult\npyo3\n'),
        ('all-line-separators', 'pyo3\r\npythonize\rPyResult\vpyo3\fpyo3\x1cpyo3\x1dpyo3\x1epyo3\x85pyo3\u2028pyo3\u2029pyo3\n'),
        ('unicode-word-boundary', 'αpyo3 pyo3α\n²pyo3 pyo3²\n\u0301pyo3 pyo3\u0301\n'),
        ('comments-and-invalid-syntax', '// pyo3 pythonize PyResult ast.dump\nthis is not { valid Rust pyo3\n'),
        ('long-line', 'x' * 400 + ' pyo3\n'),
        ('empty', ''),
    ]:
        result.append(dict(test='fresh:' + name, check=CHECK,
            files=[dict(path='rust/nera_compiler_sql/src/fresh.rs', source=source)]))
    result.append(dict(test='fresh:scope-and-sorted-discovery', check=CHECK, files=[
        dict(path='rust/nera_compiler_sql/src/z.rs', source='pyo3\n'),
        dict(path='rust/nera_compiler_sql/src/generated_grammar/a.rs', source='PyResult\n'),
        dict(path='rust/nera_sql_compiler/src/allowed.rs', source='pyo3\n'),
        dict(path='rust/nera_compiler_sql/src/ignored.py', source='pyo3\n')]))
    return result


def record(source):
    if git(source, 'rev-parse', 'HEAD') != PIN:
        raise ValueError('Wrong original source pin')
    sys.path.insert(0, str(source))
    module = importlib.import_module('scripts.lint_' + CHECK)
    module_path = source / 'scripts/lint_rust_sql_grammar_boundary.py'
    pinned = subprocess.check_output(['git', '-C', str(source), 'show', PIN + ':scripts/lint_rust_sql_grammar_boundary.py'])
    if pinned != module_path.read_bytes():
        raise ValueError('Original source differs from the pinned blob')
    records = []
    for case in cases():
        with tempfile.TemporaryDirectory(prefix='rerail-rust-original-') as temporary:
            root = Path(temporary)
            for file in case['files']:
                path = root / file['path']
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(file['source'])
            module.ROOT = root
            module.CORE_ROOT = root / 'rust/nera_compiler_sql'
            module.GENERATED_ROOT = module.CORE_ROOT / 'src/generated_grammar'
            module.SCANNED_ROOTS = (module.CORE_ROOT / 'src', module.GENERATED_ROOT)
            for file in case['files']:
                records.append(dict(**case, entry='check_file', selected=[file['path']],
                    expected=canonical(module.check_file(root / file['path']), root)))
            selected = [p.relative_to(root).as_posix() for p in module._rust_files()]
            records.append(dict(**case, entry='check_all', selected=selected,
                expected=canonical(module.check_all(), root)))
    path = source / 'scripts/lint_rust_sql_grammar_boundary.py'
    return dict(format='rerail.rr-source-derived-api-contract/v1', sourcePin=PIN,
        sourcePath=path.relative_to(source).as_posix(), sourceSha256=sha(path.read_bytes()),
        existingProbeCases=7, freshCases=7, originalPytestCases=0,
        records=records)


def findings(rows):
    if any(r['code'] != CHECK for r in rows):
        raise AssertionError('Unexpected diagnostic code')
    return [dict(kind='dataclass', type='scripts.lint_rust_sql_grammar_boundary.Finding', fields=dict(
        path=canonical(Path(r['path'])), line=r['line'], rule=r['parameters']['rule'], message=r['message']))
        for r in sorted(rows, key=lambda r: (r['path'], r['line'], r['parameters']['ordinal']))]


def run(engine, oracle):
    if oracle['sourcePin'] != PIN:
        raise ValueError('Wrong source contract pin')
    report = dict(format='rerail.rr-source-derived-native-contract/v1', sourcePin=PIN,
        engineSha256=sha(engine.read_bytes()), ruleSha256=sha(RULE.read_bytes()),
        sourceSha256=oracle['sourceSha256'], originalPytestCases=0, results=[])
    harness = [Path(__file__), *[Path(__file__).with_name(name) for name in ['migration.py', 'replay.py', 'dto.py']]]
    report['harnessFiles'] = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in harness}
    positive = None
    for index, record in enumerate(oracle['records']):
        # check_all receives every supplied document. The native policy selects its own Rust scope.
        files = (record['files'] if record['entry'] == 'check_all' else
                 [f for name in record['selected'] for f in record['files'] if f['path'] == name])
        native = execute_native(engine, RULE, files, dict(entryPoint=record['entry']))
        actual = findings(native['diagnostics'])
        compare(record['expected'], actual)
        report['results'].append(dict(index=index, test=record['test'], entry=record['entry'],
            positive=bool(actual), expected=record['expected'], actual=actual, status='passed'))
        report.setdefault('programInputs', native['programInputs'])
        if report['programInputs'] != native['programInputs']:
            raise AssertionError('Program changed during replay')
        if actual and positive is None:
            positive = (record, files)
    if positive is None:
        raise AssertionError('No positive mutation control')
    with tempfile.TemporaryDirectory(prefix='disabled-rust-', dir=ROOT / '.rerail') as temporary:
        policy = Path(temporary) / 'disabled.rr'
        policy.write_text('.policy "disabled" revision 1\n.language rust\n.violation Disabled(at: span) "disabled"\nDisabled(@place("disabled.rs", 1)) :- 1 = 0.\n')
        disabled = execute_native(engine, policy, positive[1], dict(entryPoint=positive[0]['entry']))
        if findings(disabled['diagnostics']) == positive[0]['expected']:
            raise AssertionError('Disabled detector escaped the positive control')
        report['positiveMutationControl'] = dict(test=positive[0]['test'], rejected=True)
    if sha(engine.read_bytes()) != report['engineSha256']:
        raise AssertionError('Engine changed during contracts')
    if any(sha((ROOT / path).read_bytes()) != digest for path, digest in report['harnessFiles'].items()):
        raise AssertionError('Harness changed during contracts')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    parser.add_argument('--source', type=Path)
    parser.add_argument('--engine', type=Path)
    parser.add_argument('--oracle', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'source':
        result = record(args.source.resolve())
    else:
        result = run(args.engine.resolve(), read_json(args.oracle))
        result['oracleSha256'] = sha(args.oracle.read_bytes())
    write_json(args.out, result)
    print(json.dumps(dict(cases=len(result.get('records', result.get('results', []))), originalPytestCases=0)))


if __name__ == '__main__':
    main()
