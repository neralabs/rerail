"""Prove PermissionError parity checks reject disabled native behavior."""
import argparse
from pathlib import Path
import tempfile

from migration import ROOT, PIN, compare, sha, write_json
from replay import execute_native


def run(engine):
    policy = ROOT/'gate/data-layer/packs/data-layer/candidates/rr/permission-error.rr'
    text = policy.read_text()
    files = [dict(path='input.py', source='raise PermissionError\n')]
    scope = dict(permissionBatch='true', permissionScanRoot='/project',
                 candidateDocs=['input.py'], candidatePaths=['/project/tests/input.py'])
    specs = [
        ('detector', files, dict(entryPoint='check_file'),
         text.replace('permission_raise(r) :-', 'permission_raise(r) :- permission_control_disabled(),') +
         '\n.decl permission_control_disabled()\n'),
        ('content-trigger', files, dict(entryPoint='check_file_content', permissionSuppliedSource='pass\n'),
         text.replace('option("permissionSuppliedSource",s), contains("PermissionError",s)', 'option("permissionSuppliedSource",s)')),
        ('cli-rendering', [], dict(entryPoint='_cli_report', sourceRoot='/project', checkMode='1',
                                 cliFile=['/project/a.py'], cliLine=['2'], cliMessage=['message']),
         text.replace('tcli_kind("permission").', 'tcli_kind("disabled").')),
        ('explicit-test-scope', files, dict(entryPoint='check_file', **scope),
         text + '\npermission_api_allowed(p) :- pp_doc(_,p).\n'),
        ('duplicate-inputs', [dict(path=p, source='raise PermissionError\n') for p in ['first.py', 'second.py']],
         dict(entryPoint='check_file', permissionBatch='true', permissionScanRoot='/project',
              candidateDocs=['first.py', 'second.py'], candidatePaths=['/project/a.py', '/project/a.py']),
         text + '\npermission_api_allowed(p) :- pp_doc(_,p).\n'),
    ]
    report = dict(format='rerail.permission-controls/v1', sourcePin=PIN, passed=False,
                  oracle=False, engineSha256=sha(engine.read_bytes()), policySha256=sha(policy.read_bytes()), cases=[])
    report['harnessFiles'] = {str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in
        [Path(__file__), *[Path(__file__).with_name(n) for n in ['migration.py','replay.py']]]}
    folder = ROOT/'.rerail/permission-finalec3'; folder.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='controls-', dir=folder) as temporary:
        for name, batch, options, mutation in specs:
            if mutation == text: raise AssertionError('Control did not change native behavior: '+name)
            mutated = Path(temporary)/(name+'.rr'); mutated.write_text(mutation)
            original = execute_native(engine, policy, batch, options)
            disabled = execute_native(engine, mutated, batch, options)
            a, b = original['diagnostics'], disabled['diagnostics']
            if name == 'detector': compare((len(a),len(b)), (1,0))
            if name in {'content-trigger','explicit-test-scope'}: compare((len(a),len(b)), (0,1))
            if name == 'duplicate-inputs': compare((len(a),len(b)), (1,2))
            if name == 'cli-rendering':
                assert any(r['code']=='TEXT_CLI_OUTPUT' for r in a)
                assert not any(r['code']=='TEXT_CLI_OUTPUT' for r in b)
            assert a != b
            report['cases'].append(dict(name=name, passed=True, original=original, disabled=disabled,
                                        mutation=mutation, mutationSha256=sha(mutation.encode())))
    report['passed'] = True
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    result = run(args.engine.absolute()); write_json(args.out, result)
    print(dict(controls=len(result['cases']), passed=result['passed']))


if __name__ == '__main__': main()
