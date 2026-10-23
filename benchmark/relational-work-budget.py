#!/usr/bin/env python3
"""Profile native work accounting in the bounded relational outcome kernel.

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
    sources += [root / 'native/src/code_rule_dsl/sql/terminals/fact.rs', Path(__file__).resolve()]
    pinned = {p: digest(p) for p in [binary, *sources]}
    env = {k: v for k, v in os.environ.items() if not k.startswith('RERAIL_')}
    env.update(RUST_MIN_STACK='67108864', RERAIL_RELATIONAL_WORK_EVIDENCE=str(output / 'results.json'))
    with (output / 'semantics.log').open('w') as log:
        subprocess.run([str(binary), 'sql::relational::', '--nocapture', '--test-threads=1'],
                       cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    samples = json.loads((output / 'results.json').read_text())
    assert len(samples) == 12
    for sample in samples:
        assert sample['parity'] and sample['metrics']['execution']['matchingStatements'] == 1
        assert sample['metrics']['framePublications'] == 0
        query = output / f"{sample['dimension']}-{sample['size']}.sql"
        query.write_text(sample.pop('sql') + '\n')
        sample['querySha256'] = digest(query)
        metrics = sample['metrics']['execution']
        flags = ['Native semantic allowance is separate from SQLite VM instructions.']
        if sample['dimension'] == 'early-limit':
            assert len(sample['budgetCalls']) == 2
            flags.append('Candidate roots still enumerate after an early limit; value calls stop at the first failure per file.')
        if metrics['prepareWallNs'] > max(1_000_000, 2 * (metrics['prepareExecuteWallNs'] - metrics['prepareWallNs'])):
            flags.append('Preparation dominates: inspect repeated child CTEs before increasing clause batch width.')
        sample['summary'] = {
            'sqlBytes': query.stat().st_size,
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
    traces = [json.loads(line.split('PUBLICATION_TRACE ', 1)[1])
              for line in (output / 'semantics.log').read_text().splitlines()
              if 'PUBLICATION_TRACE ' in line]
    evidence = {
        'scope': __doc__.strip(),
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'sourceHashes': {str(p.relative_to(root)): pinned[p] for p in sources},
        'executableSha256': pinned[binary], 'samples': samples, 'traces': traces,
        'limitations': ['Test-only outcome kernel; production and complete-job 100-execution gate remain pending.',
                        'Value-bearing utility memo charge replay and two-valued charged search pruning are rejected.',
                        'Native stack/depth limits, deadlines, decisions, enumeration, and general recursive outcomes remain pending.'],
        'build': 'loop profile, unbounded-work feature; diagnostic jemalloc shim; local LTO disabled for test executable only',
    }
    (output / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'profiles': len(samples), 'traces': len(traces), 'output': str(output)}, indent=2))


if __name__ == '__main__':
    main()
