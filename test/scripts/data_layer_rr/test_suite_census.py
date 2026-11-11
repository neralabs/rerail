"""The census keeps pytest outcomes and native detector coverage visible."""
from pathlib import Path
import sys

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from migration import PIN
from suite_census import census


def example():
    module = 'scripts/tests/test_lint_example.py'
    positive, host, pending = (f'{module}::test_{name}' for name in ('positive', 'host', 'pending'))
    manifest = dict(engineSource=dict(cliArtifact=dict(sha256='cli')),
                    rules=[dict(check='example', status='draft', ruleSha256='policy')])
    inventory = dict(sourcePin=PIN, files={module: dict(detectors=['example'], sha256='source')},
                     cases=dict.fromkeys([positive, host, pending]))
    phases = [dict(phase=phase, outcome='passed') for phase in ('setup', 'call', 'teardown')]
    report = dict(mode='workspace', oracle=False, sourcePin=PIN, engineSha256='cli',
                  policySha256={'example': 'policy'}, sourceFiles={module: 'source'},
                  exitCode=0, missingResults=[], semanticFallbacks=[], cases={positive: phases, host: phases},
                  calls=[dict(check='example', test=positive, status='passed')],
                  detectorObservations=[dict(check='example', test=positive, status='passed',
                      expected=['finding'], actual=['finding'],
                      expectedSourceResult=['original DTO'], actualSourceResult=['original DTO'])])
    return manifest, inventory, report


def test_census_keeps_pending_and_host_cases_out_of_native_coverage():
    manifest, inventory, report = example()
    results = census(manifest, inventory, [report])['checks'][0]['originalSuite']
    assert results['workspace'][0] == dict(module=next(iter(inventory['files'])), originalCases=3,
        passed=2, failed=[], pending=1, passedWithNativeCalls=1, matchedPositiveObservations=1)
    assert results['snapshot'][0]['pending'] == 3


def test_census_rejects_duplicate_assertion_reports():
    manifest, inventory, report = example()
    with pytest.raises(ValueError, match='duplicate result'):
        census(manifest, inventory, [report, report])
