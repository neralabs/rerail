"""Replay complete recorded DTOs through the production .rr engine, without fallback."""
from __future__ import annotations
import argparse
import gzip
import hashlib
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from migration import ROOT, compare, read_json, sha, write_json


def expected_result(record):
    value = record.get('expected', record.get('findings'))
    if value is None:
        raise ValueError('Missing original result')
    if record['check'] == 'generated_index_sources':
        return [re.sub(r'/[^ ]*/pytest-[0-9]+/[^/]+/', '', item) for item in value]
    return value


def program_inputs(files):
    result = []
    for file in files:
        path = Path(file['path'])
        marker = '/docs/rule-syntax/lib/'
        if marker in path.as_posix():
            logical = 'docs/rule-syntax/lib/' + path.as_posix().split(marker, 1)[1]
            if path.resolve() != (ROOT / logical).resolve():
                raise ValueError(f'Library is outside the active engine checkout: {path}')
        elif path.is_relative_to(ROOT):
            logical = path.relative_to(ROOT).as_posix()
        else:
            raise ValueError(f'Unbound program input: {path}')
        current = ROOT / logical
        digest = sha(current.read_bytes()) if current.is_file() else None
        if digest != file['sha256']:
            raise ValueError(f'Engine read a different library or policy: {logical}')
        result.append(dict(path=logical, sha256=digest))
    return sorted(result, key=lambda item: item['path'])


def result(record, diagnostics):
    """Public DTO contracts. No expected values or observed code sets enter this function."""
    rows = sorted(diagnostics, key=lambda r: (r['path'], r['line'], r['code'], r['message']))
    if record['check'] in {'removed_compat_surfaces', 'versioned_lifecycle_legacy_surfaces'}:
        return [dict(path=r['path'], line=r['line'], column=None, code=r['code'],
                     message=r['message'], severity=None) for r in rows]
    if record['check'] in {'hnsw_architecture', 'bounded_composed_graph_paths'}:
        return [dict(path=r['path'], line=r['line'], code=r['code'], message=r['message']) for r in rows]
    if record['check'] == 'generated_index_sources':
        return [f"{r['path']}:{r['line']}: {r['message']}" for r in rows]
    raise ValueError(f"No full DTO adapter for {record['check']}; this is a failure, never a skipped event")


def execute_native(engine, rule, files, options, sources=None, workspace_name='corpus'):
    """Direct scanner API: preserve the caller's exact batch, bypassing discovery."""
    if 'inputFiles' in options:
        raise ValueError('inputFiles is controlled by the supplied document batch')
    return _execute(engine,rule,files,dict(options,inputFiles=[file['path'] for file in files]),sources,workspace_name)


def execute_workspace(engine, rule, files, options, sources=None, workspace_name='corpus'):
    """Production workspace API: policy scope and native loading select documents."""
    if 'inputFiles' in options:
        raise ValueError('Workspace acceptance cannot override inputFiles')
    return _execute(engine,rule,files,dict(options),sources,workspace_name)


def _execute(engine, rule, files, options, sources, workspace_name):
    import migration
    manifest=migration.read_json(ROOT/migration.MANIFEST)
    migration.verify_source_cohort(ROOT,manifest['engineSource'])
    for retired in ('.rerail/rr-migration-20261106/engine','.rerail/data-layer-batch-seven/engine-source'):
        if engine.resolve().is_relative_to((ROOT/retired).resolve()):
            raise ValueError('Acceptance cannot use a binary from a retired engine checkout')
    migration.verify_engine_artifact(ROOT, manifest['engineSource'], engine)
    sources = sources or {}
    with tempfile.TemporaryDirectory(prefix='rerail-rr-replay-') as temporary:
        root = Path(temporary)
        if Path(workspace_name).name != workspace_name or workspace_name in {'', '.', '..'}:
            raise ValueError('Invalid fixture workspace name')
        corpus = root / workspace_name
        corpus.mkdir()
        for file in files:
            path = Path(file['path'])
            if path.is_absolute() or '..' in path.parts:
                raise ValueError(f'Buffer API required for this path: {path}')
            target = corpus / path
            target.parent.mkdir(parents=True, exist_ok=True)
            source = file.get('source', sources.get(file.get('sourceSha256')))
            if not isinstance(source, str):
                raise ValueError(f'Missing source for {path}')
            target.write_text(source)
        option_file = root / 'options.json'
        write_json(option_file, options)
        command = [str(engine), 'run-rr', str(rule), str(corpus), str(root / 'out'), str(option_file)]
        run = subprocess.run(command, text=True, capture_output=True, timeout=100)
        if run.returncode:
            raise RuntimeError(run.stderr)
        native = json.loads(run.stdout)
        native['programInputs'] = program_inputs(native['programInputs'])
        return native


def execute(engine, rule, record, sources):
    native = execute_native(engine, rule, record['files'],
                            dict(entryPoint=record['entry'], **record.get('parameters', {})), sources)
    return result(record, native['diagnostics']), native


def worker(engine):
    """Controlled native test transport, started before upstream audit hooks are installed."""
    for line in sys.stdin:
        try:
            request = json.loads(line)
            response = dict(result=execute_native(engine, Path(request['rule']), request['files'], request['options']))
        except Exception as error:
            response = dict(error=f'{type(error).__name__}: {error}')
        print(json.dumps(response), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--oracle', type=Path, required=True)
    parser.add_argument('--check', required=True)
    parser.add_argument('--start', type=int, default=0)
    parser.add_argument('--count', type=int, default=10)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    raw = args.oracle.read_bytes()
    corpus = json.loads(gzip.decompress(raw) if args.oracle.suffix == '.gz' else raw)
    if 'records' in corpus:
        records = [record for record in corpus['records'] if record['check'] == args.check]
    else:
        if corpus.get('commit') != '2b1504adc4291daaf080b2e932962b134da1a69a' or corpus.get('pytest_exit_code') != 0:
            raise ValueError('Invalid or failing source recording')
        records = [{**event, 'expected': event['findings']} for event in corpus['events']
                   if event['check'] == args.check]
        if any(not record.get('upstream_passed') or record.get('retained_reason') for record in records):
            raise ValueError('The selected recording contains retained or failing original cases; provide full assertion adapters')
    # The recorder makes input paths relative, but check_file's string DTO embeds pytest's
    # temporary directory. Use the same transport normalization as readTenOracle; keep all
    # filename, line, message and multiplicity comparisons.
    if args.check == 'generated_index_sources':
        records = [{**record, 'expected': expected_result(record)} for record in records]
    if not records:
        raise ValueError('The selected check has no recorded cases')
    rule = ROOT / 'gate/data-layer/packs/data-layer/candidates/rr' / (args.check.replace('_', '-') + '.rr')
    report = dict(format='rerail.rr-full-dto-replay/v1', rule=str(rule.relative_to(ROOT)),
                  ruleSha256=sha(rule.read_bytes()), corpusSha256=sha(raw),
                  engineSha256=sha(args.engine.read_bytes()), selectedCases=len(records),
                  start=args.start, count=args.count, results=[])
    failed = 0
    for index, record in list(enumerate(records))[args.start:args.start + args.count]:
        entry = dict(index=index, test=record['test'], entry=record['entry'],
                     positive=bool(record['expected']), expected=record['expected'])
        try:
            actual, native = execute(args.engine.resolve(), rule, record, corpus.get('sources', {}))
            if 'programInputs' in report and report['programInputs'] != native['programInputs']:
                raise ValueError('The program or a library changed during replay')
            report['programInputs'] = native['programInputs']
            entry.update(actual=actual, measurements=native['phases'])
            compare(record['expected'], actual)
            entry['status'] = 'passed'
        except Exception as error:
            entry.update(status='failed', error=str(error))
            failed += 1
        report['results'].append(entry)
        write_json(args.out, report)
    if not report['results']:
        raise ValueError('The requested range contains no cases')
    if sha(args.engine.read_bytes()) != report['engineSha256']:
        raise ValueError('The executable changed during replay')
    print(json.dumps(dict(cases=len(report['results']), failed=failed,
                          positive=sum(r['positive'] for r in report['results']))))
    raise SystemExit(1 if failed else 0)


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--worker':
        worker(Path(sys.argv[2]))
    else:
        main()
