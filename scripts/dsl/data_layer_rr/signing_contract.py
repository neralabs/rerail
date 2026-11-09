"""Compare native signed messages with the pinned data-layer signing implementation."""
import argparse
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_native


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'engine', 'out']:
        parser.add_argument('--'+name, type=Path, required=True)
    args = parser.parse_args()
    assert git(args.source, 'rev-parse', 'HEAD') == PIN
    for name in ['scripts/lint_lib/signing.py', 'scripts/lint_serverplan_steps.py']:
        compare(subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name]),
                (args.source/name).read_bytes())
    sys.path.insert(0, str(args.source.absolute()))
    from scripts.lint_lib.signing import compute_multi_allowlist_message, verify_multi_allowlist
    from scripts.lint_serverplan_steps import ALLOWLIST_SIGNATURE
    cases = [
        ('empty-names', {'can_batch_false': [], 'lifecycle_runtime_state': []}, True),
        ('reverse-names', {'lifecycle_runtime_state': [], 'can_batch_false': []}, True),
        ('missing-name', {'can_batch_false': []}, True),
        ('no-names', {}, True),
        ('changed-entry', {'can_batch_false': ['DifferentStep'], 'lifecycle_runtime_state': []}, True),
        ('entry-infers-name', {'alpha': ['z', 'a'], 'beta': ['value']}, False),
        ('entries-and-names', {'alpha': ['z', 'a'], 'beta': ['value']}, True),
        ('unicode-and-empty', {'é': ['quote"', 'λ'], 'empty': []}, True),
    ]
    report = dict(format='rerail.native-multi-allowlist-contract/v1', sourcePin=PIN,
                  fullMigrationAcceptance=False, cases=[])
    for identity, lists, explicit in cases:
        expected_message = compute_multi_allowlist_message('serverplan_steps', lists).decode()
        with contextlib.redirect_stderr(io.StringIO()):
            expected_status = 'valid' if verify_multi_allowlist('serverplan_steps', lists, ALLOWLIST_SIGNATURE) else 'invalid'
        text = '\n'.join([
            '.policy "signing-contract" revision 1', '.language python',
            '#include "data-layer/signing.rr"',
            *[f'multi_name("steps",{json.dumps(name)}).' for name in lists if explicit],
            *[f'multi_entry("steps",{json.dumps(name)},{json.dumps(entry)}).' for name, entries in lists.items() for entry in entries],
            f'multi_sig("steps","serverplan_steps",{json.dumps(ALLOWLIST_SIGNATURE)}).',
            '.violation Message(at: span, value: symbol) code "MESSAGE" "{value}"',
            'Message(@place("",0),text) :- multi_message("steps",text).',
            '.violation Status(at: span, value: symbol) code "STATUS" "{value}"',
            'Status(@place("",0),status) :- multi_status("steps",status).',
        ])+'\n'
        # A root-local temporary policy keeps the consumed program audit intact.
        with tempfile.TemporaryDirectory(prefix='signing-contract-', dir=ROOT/'.rerail') as directory:
            rule = Path(directory)/'signing.rr'
            rule.write_text(text)
            native = execute_native(args.engine.absolute(), rule, [], {})
        actual = {r['code']: r['parameters']['value'] for r in native['diagnostics']}
        expected = dict(MESSAGE=expected_message, STATUS=expected_status)
        compare(expected, actual)
        report['cases'].append(dict(name=identity, program=text, expected=expected, actual=actual,
                                    diagnostics=native['diagnostics'], programInputs=native['programInputs']))
    report.update(engineSha256=sha(args.engine.read_bytes()),
                  engine=read_json(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/migration.json')['engineSource']['cliArtifact'],
                  sourceFiles={name:sha((args.source/name).read_bytes()) for name in
                               ['scripts/lint_lib/signing.py', 'scripts/lint_serverplan_steps.py']},
                  harnessFiles={str(Path(__file__).relative_to(ROOT)):sha(Path(__file__).read_bytes())})
    write_json(args.out, report)
    print(json.dumps(dict(cases=len(report['cases']), failed=0)))


if __name__ == '__main__':
    main()
