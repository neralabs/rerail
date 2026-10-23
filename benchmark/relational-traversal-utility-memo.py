#!/usr/bin/env python3
"""Profile traversal value-utility memo replay in the bounded relational outcome kernel.

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


def states(value):
    if isinstance(value, dict):
        if 'typedStateColumns' in value or 'recursiveColumns' in value:
            yield value
        for child in value.values():
            yield from states(child)
    elif isinstance(value, list):
        for child in value:
            yield from states(child)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve()
    sources = sorted((root / 'native/src/code_rule_dsl/sql/relational').rglob('*.rs'))
    sources += [root / f'native/src/code_rule_dsl/matcher/{p}.rs' for p in
                ('mod', 'run', 'value', 'value_probe', 'count', 'relation', 'memo', 'resolve', 'util')]
    sources += [root / 'native/src/code_rule_dsl/sql/terminals/fact.rs',
                root / 'native/src/code_rule_dsl/sql/nodes.rs',
                root / 'native/src/code_rule_dsl/matcher/analysis/required_text.rs', Path(__file__).resolve()]
    pinned = {p: digest(p) for p in [binary, *sources]}
    env = {k: v for k, v in os.environ.items() if not k.startswith('RERAIL_')}
    env.update(RUST_MIN_STACK='67108864', RERAIL_RELATIONAL_TRAVERSAL_MEMO_EVIDENCE=str(output / 'results.json'))
    with (output / 'semantics.log').open('w') as log:
        subprocess.run([str(binary), 'sql::relational::', '--nocapture', '--test-threads=1'],
                       cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    samples = json.loads((output / 'results.json').read_text())
    assert len(samples) == 15
    for sample in samples:
        assert sample['parity'] and sample['metrics']['execution']['matchingStatements'] == 1
        assert sample['metrics']['framePublications'] == 0
        query = output / f"{sample['dimension']}-{sample['size']}.sql"
        query.write_text(sample.pop('sql') + '\n')
        sample['querySha256'] = digest(query)
        metrics = sample['metrics']['execution']
        flags = ['Native semantic allowance is separate from SQLite VM instructions.']
        roots = sample['size'] if sample['dimension'] in ('overlap', 'disjoint') else 1
        expected_reads = 2 * sample['size'] if sample['dimension'] == 'disjoint' else (2 * (sample['size'] + 1) if sample['dimension'] == 'changed-captures' else 2)
        assert len(sample['sqlCalls']) == expected_reads
        storage = metrics['memoStorage']
        assert storage['resets'] == 1
        expected_answers = expected_reads
        assert storage['publications'] == expected_answers
        expected_lookups = {'repeats': 2 * sample['size'], 'overlap': 4 * sample['size'],
                            'disjoint': 4 * sample['size'], 'changed-captures': 2 * (1 + 2 * sample['size']),
                            'early-stop': 4}[sample['dimension']]
        assert storage['lookups'] == expected_lookups
        assert storage['hits'] == expected_lookups - expected_answers
        flags.append('SQL controls composition and cost replay; an indexed scratch cursor stores integer keys and completed answers.')
        flags.append('No memo map is copied through recursive traversal rows.')
        if metrics['prepareWallNs'] > max(1_000_000, 2 * (metrics['prepareExecuteWallNs'] - metrics['prepareWallNs'])):
            flags.append('Preparation dominates: inspect repeated child CTEs before increasing clause batch width.')
        sample['summary'] = {
            'sqlBytes': query.stat().st_size,
            'maxTypedStateColumns': max((s.get('typedStateColumns', 0) for s in states(sample['relations'])), default=0),
            'maxMaterializedStateColumns': max((s.get('typedStateColumns', 0) for s in states(sample['relations']) if s.get('storage') == 'materialized'), default=0),
            'maxRecursiveColumns': max((s.get('recursiveColumns',0) for s in states(sample['relations'])), default=0),
            'clauses': len(sample['relations']['clauses']),
            'allowancePerClauseFile': sample['allowance'],
            'publishedMatches': sum(row[4] == 0 for row in sample['output']),
            'publishedFailures': sum(row[4] < 0 for row in sample['output']),
            'valueCalls': len(sample['budgetCalls']),
            'materializationSitesInExplain': sum(line.startswith('MATERIALIZE ') for line in sample['metrics']['explain']),
            'flags': flags,
        }
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
            if '::traversal_memo_tests::' in current_test:
                traces.append({'test': current_test.rsplit('::', 1)[1],
                               **json.loads(line.split('PUBLICATION_TRACE ', 1)[1])})
    evidence = {
        'scope': __doc__.strip(),
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'sourceHashes': {str(p.relative_to(root)): pinned[p] for p in sources},
        'executableSha256': pinned[binary], 'samples': samples, 'traces': traces,
        'allPublicationTraceCount': all_traces,
        'limitations': ['Test-only outcome kernel; production and complete-job 100-execution gate remain pending.',
                        'Traversal depth remains bounded to four; utility cycles remain rejected.',
                        'Native stack/depth limits, deadlines, decisions, enumeration, and general recursive outcomes remain pending.'],
        'build': 'loop profile, unbounded-work feature; diagnostic jemalloc shim; local LTO disabled for test executable only',
    }
    (output / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'profiles': len(samples), 'traces': len(traces), 'output': str(output)}, indent=2))


if __name__ == '__main__':
    main()
