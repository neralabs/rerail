#!/usr/bin/env python3
"""Profile grouped direct utility replays in the bounded relational outcome kernel.

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
    env.update(RUST_MIN_STACK='67108864', RERAIL_RELATIONAL_COMPOSITION_BUDGET_EVIDENCE=str(output / 'results.json'),
               RERAIL_RELATIONAL_REPEAT_UTILITY_EVIDENCE=str(output / 'simple.json'))
    with (output / 'semantics.log').open('w') as log:
        subprocess.run([str(binary), 'sql::relational::', '--nocapture', '--test-threads=1'],
                       cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    samples = json.loads((output / 'results.json').read_text())
    samples += json.loads((output / 'simple.json').read_text())
    assert len(samples) == 36
    for sample in samples:
        assert sample['parity'] and sample['metrics']['execution']['matchingStatements'] == 1
        assert sample['metrics']['framePublications'] == 0
        query = output / f"{sample['dimension']}-{sample['size']}.sql"
        query.write_text(sample.pop('sql') + '\n')
        sample['querySha256'] = digest(query)
        m = sample['metrics']['execution']
        assert len(sample['sqlCalls']) == 2
        assert m['memoStorage']['publications'] == 0
        lower = sample['relations']['lowering'][0]
        wide = sample['dimension'].endswith('wide')
        assert lower['distinctUtilityBodies'] == lower['utilityBodyLowerings'] == (2 if wide else 1)
        assert lower['utilityCallSites'] == sample['size'] + int(wide)
        assert lower['atomicTerminals'] == (9 if wide else 1)
        assert lower['compositionOperators'] == (16 if wide else 6)
        relations = list(objects(sample['relations']))
        misses = sum(o.get('operator') == 'nested utility entry or replay charge' for o in relations)
        replays = sum(o.get('operator') == 'utility memo statically identical replay' for o in relations)
        groups = [o['repeatUtility'] for o in relations if 'repeatUtility' in o]
        assert misses == int(wide)
        assert len(groups) == 1 and groups[0]['calls'] == sample['size'] and replays == 0
        assert groups[0]['stopTruth'] == (0 if sample['dimension'].startswith('all-') else 1)
        assert query.read_text().count('relation_value_budget(') == 1
        sample['summary'] = {
            'sqlBytes': query.stat().st_size, 'lowering': lower,
            'generationWallNs': sample['generationWallNs'],
            'emittedOuterMissBodies': misses, 'staticReplays': replays, 'repeatGroups': groups,
            'materializationSitesInExplain': sum(line.startswith('MATERIALIZE ') for line in sample['metrics']['explain']),
            'flags': [('One body and one guarded replay group.' if groups else 'One body plus one replay/barrier per remaining source call; SQL size still grows with call sites.'),
                      'Counters include instrumentation; one matching statement does not prove the complete cold-job budget.'],
        }
    for dimension in {s['dimension'] for s in samples}:
        group = [s for s in samples if s['dimension'] == dimension]
        assert max(s['summary']['sqlBytes'] for s in group) - min(s['summary']['sqlBytes'] for s in group) < 128
        assert len({s['summary']['materializationSitesInExplain'] for s in group}) == 1
        assert len({s['metrics']['execution']['typedReturnCells'] for s in group}) == 1
        vm = [s['metrics']['execution']['vmSteps'] for s in group]
        assert max(vm)-min(vm) <= 1
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
            if '::composition_budget_tests::' in current_test or '::repeat_utility_tests::' in current_test:
                traces.append({'test': current_test.rsplit('::', 1)[1], **json.loads(line.split('PUBLICATION_TRACE ', 1)[1])})
    evidence = {
        'scope': __doc__.strip(), 'compositionLimit': 256,
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'sourceHashes': {str(p.relative_to(root)): pinned[p] for p in sources},
        'executableSha256': pinned[binary], 'samples': samples, 'traces': traces,
        'allPublicationTraceCount': all_traces,
        'limitations': ['Test-only outcome kernel; production and complete-job 100-execution gate remain pending.',
                        'Utility bodies and answers retain their existing sharing proofs; each emitted operator consumes the same clause budget.',
                        'All nested programs share a 256 emitted-operator budget per clause; each direct replay group consumes one operator while semantic charges retain every reached call.',
                        'Native stack/depth limits, deadlines, decisions, enumeration, and general recursive outcomes remain pending.'],
        'build': 'loop profile, unbounded-work feature; diagnostic jemalloc shim; local LTO disabled for test executable only',
    }
    (output / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'profiles': len(samples), 'traces': len(traces), 'output': str(output)}, indent=2))


if __name__ == '__main__':
    main()
