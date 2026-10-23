#!/usr/bin/env python3
"""Profile compiled Any repeat groups in the bounded relational outcome kernel.

Counters include instrumentation. One matching statement, not the complete cold job.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def objects(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from objects(child)
    elif isinstance(value, list):
        for child in value:
            yield from objects(child)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--expect-sharing', action='store_true', help='Assert compiler output, without changing engine behavior.')
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve()
    sources = sorted((root / 'native/src/code_rule_dsl/sql/relational').rglob('*.rs'))
    sources += [root / f'native/src/code_rule_dsl/matcher/{p}.rs' for p in
                ('mod', 'run', 'value', 'value_probe', 'count', 'relation', 'memo', 'resolve', 'util')]
    sources += [root / 'native/src/code_rule_dsl/sql/terminals/fact.rs', root / 'native/src/code_rule_dsl/sql/nodes.rs',
                root / 'native/src/code_rule_dsl/matcher/analysis/required_text.rs', Path(__file__).resolve()]
    pinned = {p: digest(p) for p in [binary, *sources]}
    env = {k: v for k, v in os.environ.items() if not k.startswith('RERAIL_')}
    env.update(RUST_MIN_STACK='67108864', RERAIL_RELATIONAL_REPEAT_ANY_EVIDENCE=str(output / 'results.json'))
    with (output / 'semantics.log').open('w') as log:
        subprocess.run([str(binary), 'sql::relational::', '--nocapture', '--test-threads=1'],
                       cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    samples = json.loads((output / 'results.json').read_text())
    assert len(samples) == 18
    for sample in samples:
        assert sample['parity'] and sample['metrics']['execution']['matchingStatements'] == 1
        assert sample['metrics']['framePublications'] == 0
        query = output / f"{sample['dimension']}-{sample['size']}.sql"
        query.write_text(sample.pop('sql') + '\n')
        sample['querySha256'] = digest(query)
        m = sample['metrics']['execution']
        groups = [o['repeatTraversal'] for o in objects(sample['relations']) if 'repeatTraversal' in o]
        if args.expect_sharing:
            assert len(groups) == 1 and groups[0]['calls'] == sample['size'] and groups[0]['stopTruth'] == 1
        reads = 4 if not sample['dimension'].startswith('true') else 2
        assert len(sample['sqlCalls']) == reads
        assert m['memoStorage']['publications'] == reads
        if args.expect_sharing:
            assert m['memoStorage']['lookups'] == (4 if not sample['dimension'].startswith('true') else 2)
        flags = ['SQL replays proven stable utility costs; native work allowance differs from VM instructions.']
        if m['prepareWallNs'] > max(1_000_000, 2 * (m['prepareExecuteWallNs'] - m['prepareWallNs'])):
            flags.append('Preparation dominates: inspect remaining materialization sites and nested child programs.')
        sample['summary'] = {
            'sqlBytes': query.stat().st_size, 'repeatGroups': groups,
            'maxRecursiveColumns': max((o.get('recursiveColumns', 0) for o in objects(sample['relations'])), default=0),
            'materializationSitesInExplain': sum(line.startswith('MATERIALIZE ') for line in sample['metrics']['explain']),
            'flags': flags,
        }
    if args.expect_sharing:
        for dimension in {s['dimension'] for s in samples}:
            group = [s for s in samples if s['dimension'] == dimension]
            assert max(s['summary']['sqlBytes'] for s in group) - min(s['summary']['sqlBytes'] for s in group) < 128
            steps = [s['metrics']['execution']['vmSteps'] for s in group]
            assert max(steps) - min(steps) <= 1  # One extra constant-register initialization beyond repeats=1.
            assert len({s['metrics']['execution']['typedReturnCells'] for s in group}) == 1
    for path, sha in pinned.items():
        assert digest(path) == sha, f'Artifact changed during run: {path}'
    traces = []
    current_test = ''
    all_traces = 0
    for line in (output / 'semantics.log').read_text().splitlines():
        if line.startswith('test '):
            current_test = line.split()[1]
        if 'PUBLICATION_TRACE ' in line:
            all_traces += 1
            if '::repeat_any_tests::' in current_test:
                traces.append({'test': current_test.rsplit('::', 1)[1], **json.loads(line.split('PUBLICATION_TRACE ', 1)[1])})
    evidence = {
        'scope': __doc__.strip(), 'expectSharing': args.expect_sharing,
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'sourceHashes': {str(p.relative_to(root)): pinned[p] for p in sources},
        'executableSha256': pinned[binary], 'samples': samples, 'traces': traces,
        'allPublicationTraceCount': all_traces,
        'limitations': ['Test-only outcome kernel; production and complete-job 100-execution gate remain pending.',
                        'Repeat sharing requires adjacent capture-free descendant searches in Any, without stops, with a pure prefix and one value-bearing utility.',
                        'Native stack/depth limits, deadlines, decisions, enumeration, and general recursive outcomes remain pending.'],
        'build': 'loop profile, unbounded-work feature; diagnostic jemalloc shim; local LTO disabled for test executable only',
    }
    (output / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'profiles': len(samples), 'traces': len(traces), 'output': str(output)}, indent=2))


if __name__ == '__main__':
    main()
