#!/usr/bin/env python3
"""Profile the bounded workspace clause/file failure publication kernel.

One matching statement per job. This is not the complete-job 100-execution gate.
Timing includes counters; diagnostics and native reference runs are separate.
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
    sources += [root / p for p in (
        'native/src/code_rule_dsl/matcher/mod.rs', 'native/src/code_rule_dsl/matcher/run.rs',
        'native/src/code_rule_dsl/matcher/value.rs', 'native/src/code_rule_dsl/matcher/value_probe.rs',
        'native/src/code_rule_dsl/sql/terminals/mod.rs', 'native/src/code_rule_dsl/sql/terminals/fact.rs',
        'native/src/code_rule_dsl/workspace/tests.rs', 'native/src/code_rule_dsl/workspace/results.rs',
        'native/src/code_rule_dsl/workspace/evaluate.rs', 'native/src/code_rule_dsl/eval/mod.rs',
        'native/src/code_rule_dsl/eval/query.rs')]
    sources += [Path(__file__).resolve()]
    pinned = {p: digest(p) for p in [binary, *sources]}
    env = {k: v for k, v in os.environ.items() if not k.startswith('RERAIL_')}
    env.update(RUST_MIN_STACK='67108864', RERAIL_RELATIONAL_PUBLICATION_EVIDENCE=str(output / 'results.json'))
    for label, selection in [('semantics', 'sql::relational::'),
                             ('workspace', 'relational_publication_native_workspace_')]:
        with (output / f'{label}.log').open('w') as log:
            subprocess.run([str(binary), selection, '--nocapture', '--test-threads=1'],
                           cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    samples = json.loads((output / 'results.json').read_text())
    assert len(samples) == 12
    for sample in samples:
        assert sample['parity'] and sample['metrics']['execution']['matchingStatements'] == 1
        assert sample['metrics']['framePublications'] == 0
        query = output / f"{sample['dimension']}-{sample['size']}.sql"
        query.write_text(sample['sql'] + '\n')
        sample['querySha256'] = digest(query)
        metrics = sample['metrics']['execution']
        flags = ['Root enumeration still materializes candidate keys after an early failure.'] if sample['dimension'] == 'early-failure' else []
        execution = metrics['prepareExecuteWallNs'] - metrics['prepareWallNs']
        if metrics['prepareWallNs'] > max(1_000_000, 2 * execution):
            flags.append('Preparation dominates: inspect clause and child CTE expansion before increasing batch width.')
        sample['summary'] = {
            'sqlBytes': len(sample['sql'].encode()),
            'clauses': len(sample['relations']['clauses']),
            'publishedMatches': sum(row[4] == 0 for row in sample['output']),
            'publishedFailures': sum(row[4] < 0 for row in sample['output']),
            'valueCalls': len(sample['sqlCalls']),
            'materializationSitesInExplain': sum(line.startswith('MATERIALIZE ') for line in sample['metrics']['explain']),
            'flags': flags,
            'typedPublicationColumns': sample['relations'].get('publicationWidth'),
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
        'executableSha256': pinned[binary],
        'samples': samples, 'traces': traces,
        'notes': ['Workspace failure suppresses one clause/file pair, including earlier matches.',
                  'The stateless API separately suppresses the entire clause result on a reached file limit.',
                  'Atomic limits use negative integer tags. Unexpected host/SQL errors still abort the statement.',
                  'SQL sequences roots within each fallible clause/file pair; independent pairs can interleave.',
                  'Captures stay typed until final publication. No mutable failure table or host retry loop.',
                  'Native work/depth/stack limits, decisions, enumeration and full production integration remain open.'],
    }
    (output / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps([{'dimension': s['dimension'], 'size': s['size'], **s['metrics']['execution'], **s['summary']} for s in samples], indent=2))


if __name__ == '__main__':
    main()
