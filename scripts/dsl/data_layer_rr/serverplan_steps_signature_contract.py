"""Record complete verification results and compare batches without signing artifacts."""
import argparse
import contextlib
import io
import itertools
from pathlib import Path
import subprocess
import sys

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_native


def variants(signature):
    return [signature, '!!! tampered !!!', '', 'UNSIGNED', 'NEEDS_SIGNING',
            '=='+signature, signature+'===', signature+'ignored', signature+'\N{SNOWMAN}',
            signature[:20]+' \n!?'+signature[20:], signature[:85]+'R==',
            signature.rstrip('='), 'A', 'AA', 'AAA', 'AAAA', 'AA==', 'AAA=',
            'A'*84, 'A'*86+'==', 'A'*88, 'A'*101, 'A'*10001, 'é']


def source_record(source):
    import base64
    import nacl
    import nacl.signing
    sys.path.insert(0, str(source.absolute()))
    from scripts import lint_serverplan_steps as linter
    from scripts.lint_lib.signing import compute_multi_allowlist_message, PUBLIC_KEY_HEX
    signature = linter.ALLOWLIST_SIGNATURE
    message = compute_multi_allowlist_message(linter.LINTER_NAME,
        {'can_batch_false': [], 'lifecycle_runtime_state': []}).decode()
    signatures = variants(signature)
    generic = list(dict.fromkeys(signatures + [''.join(v) for n in range(7)
        for v in itertools.product('A=?', repeat=n)]))
    key = nacl.signing.VerifyKey(bytes.fromhex(PUBLIC_KEY_HEX))
    rows = []
    for value in generic:
        try:
            key.verify(message.encode(), base64.b64decode(value))
            error = ''
        except Exception as exc:
            error = str(exc)
        rows.append(dict(signature=value, message=message, error=error, valid=int(not error)))
    groups = []
    for can_batch, runtime in [([], []), (['Å', 'Alpha'], []), ([], ['RuntimeOwner'])]:
        linter.CAN_BATCH_FALSE_ALLOWLIST = frozenset(can_batch)
        linter.LIFECYCLE_RUNTIME_STATE_ALLOWLIST = frozenset(runtime)
        cases = []
        for value in signatures:
            linter.ALLOWLIST_SIGNATURE = value
            stdout, stderr = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                result = linter._verify_allowlists()
            cases.append(dict(signature=value, expected=dict(result=result,
                              stdout=stdout.getvalue(), stderr=stderr.getvalue())))
        groups.append(dict(canBatchAllowlist=can_batch, runtimeStateAllowlist=runtime, cases=cases))
    return dict(generic=dict(python=sys.version, pynacl=nacl.__version__, key=PUBLIC_KEY_HEX, cases=rows), groups=groups)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    for name in ['source', 'engine', 'baseline', 'out']:
        parser.add_argument('--'+name, type=Path, required=name in ['source', 'out'])
    args = parser.parse_args()
    assert git(args.source, 'rev-parse', 'HEAD') == PIN
    names = ['scripts/lint_serverplan_steps.py', 'scripts/lint_lib/signing.py']
    for name in names:
        compare((args.source/name).read_bytes(), subprocess.check_output(['git', '-C', str(args.source), 'show', PIN+':'+name]))
    report = dict(format='rerail.serverplan-steps-signature-contract/v1', sourcePin=PIN,
        fullMigrationAcceptance=False, passed=False,
        sourceFiles={n:sha((args.source/n).read_bytes()) for n in names},
        harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in
            [Path(__file__), *[Path(__file__).with_name(n) for n in ['replay.py', 'migration.py']]]})
    if args.command == 'source':
        report.update(source_record(args.source))
    else:
        baseline = read_json(args.baseline)
        assert baseline['passed'] and baseline['sourcePin'] == PIN
        report.update(engineSha256=sha(args.engine.read_bytes()), groups=[])
        for group in baseline['groups']:
            options = {k:v for k,v in group.items() if k!='cases'}
            options.update(entryPoint='_verify_allowlists', verificationSignatures=[c['signature'] for c in group['cases']])
            native = execute_native(args.engine.absolute(), ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps.rr', [], options)
            if 'programs' not in report:
                report['programs'] = {p['path']:dict(sha256=p['sha256'],text=(ROOT/p['path']).read_text()) for p in native['programInputs']}
            for p in native['programInputs']:
                compare(p['sha256'], sha(report['programs'][p['path']]['text'].encode()))
            rows = native['diagnostics']
            assert all(r['code']=='SSP_VERIFY' for r in rows)
            by_index = {r['parameters']['ordinal']:r['parameters'] for r in rows}
            assert len(rows)==len(by_index)==len(group['cases'])
            cases = []
            for i, case in enumerate(group['cases']):
                p = by_index[i]
                actual = dict(result=bool(p['valid']), stdout=p['stdout'], stderr=p['stderr'])
                cases.append(dict(case, actual=actual))
            report['groups'].append(dict(group,cases=cases,native=native))
            write_json(args.out, report)
            for case in cases: compare(case['expected'],case['actual'])
    report['passed'] = True
    write_json(args.out, report)
    print(dict(groups=len(report['groups']),cases=sum(len(g['cases']) for g in report['groups']),passed=True))


if __name__ == '__main__':
    main()
