"""Exercise production edits, aliases, deletions, and repeats against one tape store."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
from unittest.mock import patch

import first_ten_production_contract as contract
from migration import ROOT, PIN, MANIFEST, compare, read_json, sha, write_json, verify_source_cohort, verify_engine_artifact
from replay import write_fixture, program_inputs

STEPS = ['empty', 'roots', 'clean', 'roots', 'file-links', 'directory-links', 'ordering',
         'duplicate-findings', 'syntax', 'clean', 'empty', 'roots', 'roots', 'empty']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['engine', 'baseline', 'out']: parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--part', type=int, default=0); parser.add_argument('--parts', type=int, default=1)
    args = parser.parse_args(); assert 0 <= args.part < args.parts
    baseline = read_json(args.baseline); assert baseline['passed'] and baseline['sourcePin'] == PIN
    cohort = read_json(ROOT/MANIFEST)['engineSource']; engine = args.engine.absolute()
    verify_source_cohort(ROOT, cohort); verify_engine_artifact(ROOT, cohort, engine)
    dependencies = [Path(__file__), *[Path(__file__).with_name(n) for n in ['first_ten_production_contract.py', 'replay.py', 'migration.py']]]
    hashes = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in dependencies}
    report = dict(format='rerail.first-ten-production-updates/v1', sourcePin=PIN, passed=False,
        fullMigrationAcceptance=False, engineSha256=sha(engine.read_bytes()), baselineSha256=sha(args.baseline.read_bytes()),
        part=args.part, parts=args.parts, harnessFiles=hashes, cases=[])
    source_cases = {r['input']['id']: r for r in baseline['cases']}
    for check in contract.CHECKS[args.part::args.parts]:
        with tempfile.TemporaryDirectory(prefix='first-ten-updates-') as temporary:
            root = Path(temporary); workspace = root/'corpus'; workspace.mkdir(); previous = {}; prior_times = None
            executions = []
            def execute(binary, policy, files, options):
                verify_engine_artifact(ROOT, cohort, binary)
                option_file = root/'options.json'; write_json(option_file, options)
                run = subprocess.run([str(binary), 'run-rr', str(policy), str(workspace), str(root/'out'), str(option_file)],
                    capture_output=True, text=True, timeout=100, env=dict(os.environ, RERAIL_TAPE_CACHE=str(root/'store.sqlite')))
                if run.returncode: raise RuntimeError(run.stderr)
                result = json.loads(run.stdout); result['programInputs'] = program_inputs(result['programInputs'])
                executions.append(result); return result
            with patch.object(contract, 'execute_workspace', execute):
                for step, label in enumerate(STEPS):
                    source = source_cases[check+':'+label]; case = source['input']
                    current = {f['path']: f for f in case['files']}
                    changed = {p for p in set(previous)|set(current) if previous.get(p) != current.get(p)}
                    for p in sorted((p for p in previous if p in changed), key=lambda p: p.count('/'), reverse=True):
                        (workspace/p).unlink()
                    write_fixture(workspace, [f for p,f in current.items() if p in changed])
                    executions.clear(); before = {p: (workspace/p).lstat().st_mtime_ns for p in current}
                    actual, _ = contract.native_case(engine, case)
                    compare(source['expected'], actual)
                    if step and label == STEPS[step-1]:
                        assert not changed and before == prior_times
                    report['cases'].append(dict(check=check, step=step, id=label, changedPaths=sorted(changed),
                        expected=source['expected'], actual=actual, native=list(executions), passed=True))
                    write_json(args.out, report); print(dict(check=check, step=step, passed=True), flush=True)
                    previous = current; prior_times = before
    for name, digest in hashes.items(): compare(digest, sha((ROOT/name).read_bytes()))
    report['passed'] = True; write_json(args.out, report)


if __name__ == '__main__': main()
