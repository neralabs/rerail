#!/usr/bin/env python3
"""Profile total predicate runs in shared SQL Boolean composition.

Measures the test-only outcome kernel, not the complete cold-job execution budget.
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
    env.update(RUST_MIN_STACK='67108864', RERAIL_RELATIONAL_PREDICATE_RUN_EVIDENCE=str(output / 'results.json'))
    with (output / 'semantics.log').open('w') as log:
        subprocess.run([str(binary), 'predicate_run_', '--nocapture', '--test-threads=1'],
                       cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    samples = json.loads((output / 'results.json').read_text())
    assert len(samples) == 54
    for sample in samples:
        m = sample['metrics']['execution']
        assert sample['parity'] and m['matchingStatements'] == 1
        assert sample['metrics']['framePublications'] == 0
        query = output / f"{sample['dimension']}-{sample['width']}-{sample['size']}.sql"
        query.write_text(sample.pop('sql') + '\n')
        sample['querySha256'] = digest(query)
        assert len(sample['sqlCalls']) == 2
        assert m['memoStorage']['publications'] == 0
        lower = sample['relations']['lowering'][0]
        assert lower['distinctUtilityBodies'] == lower['utilityBodyLowerings'] == 2
        assert lower['utilityCallSites'] == sample['size'] + 1
        assert lower['atomicTerminals'] == sample['width'] + 1
        assert lower['compositionOperators'] == sample['width'] + 9
        relations = list(objects(sample['relations']))
        runs = [o['predicateRun'] for o in relations if 'predicateRun' in o]
        groups = [o['repeatUtility'] for o in relations if 'repeatUtility' in o]
        assert len(runs) == 1 and runs[0]['predicates'] == sample['width']
        assert len(groups) == 1 and groups[0]['calls'] == sample['size']
        assert query.read_text().count('relation_value_budget(') == 1
        assert query.read_text().count('node_regex(') == sample['width']
        materializations = sum(line.startswith('MATERIALIZE ') for line in sample['metrics']['explain'])
        assert materializations == 28
        assert m['typedReturnCells'] == 12
        sample['summary'] = dict(sqlBytes=query.stat().st_size-1,
                                 materializationSitesInExplain=materializations,
                                 predicateRuns=runs, repeatGroups=groups, lowering=lower)
    for dimension in {s['dimension'] for s in samples}:
        group = [s for s in samples if s['dimension'] == dimension]
        assert len({s['summary']['materializationSitesInExplain'] for s in group}) == 1
        assert len({s['metrics']['execution']['typedReturnCells'] for s in group}) == 1
        for width in [2, 8, 32]:
            repeated = [s for s in group if s['width'] == width]
            vm = [s['metrics']['execution']['vmSteps'] for s in repeated]
            assert max(vm)-min(vm) <= 1
            assert max(s['summary']['sqlBytes'] for s in repeated)-min(s['summary']['sqlBytes'] for s in repeated) < 128
    for path, sha in pinned.items():
        assert digest(path) == sha, f'Artifact changed during run: {path}'
    traces = []
    current_test = ''
    for line in (output / 'semantics.log').read_text().splitlines():
        if line.startswith('test '):
            current_test = line.split()[1].rsplit('::', 1)[-1]
        if 'PUBLICATION_TRACE ' in line:
            traces.append({'test': current_test, **json.loads(line.split('PUBLICATION_TRACE ', 1)[1])})
    evidence = {
        'scope': __doc__.strip(), 'compositionLimit': 256,
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'sourceHashes': {str(p.relative_to(root)): pinned[p] for p in sources},
        'executableSha256': pinned[binary], 'samples': samples, 'traces': traces,
        'limitations': ['Test-only outcome kernel; production and complete-job 100-execution gate remain pending.',
                        'Timing samples include instrumentation and are not production CPU qualification.',
                        'A predicate run consumes its leaves plus one Boolean operator; all existing clause limits remain.',
                        'Native stack/depth limits, deadlines, and general recursive outcomes remain pending.'],
        'build': 'loop profile, unbounded-work feature; diagnostic jemalloc shim; local LTO disabled for test executable only',
    }
    (output / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'profiles': len(samples), 'traces': len(traces), 'output': str(output)}, indent=2))


if __name__ == '__main__':
    main()
