"""Exercise scoped production findings and exact CLI streams for legacy execution."""
from contextlib import redirect_stderr, redirect_stdout
import io
from pathlib import Path
import sys
from unittest.mock import patch

from replay import write_fixture

CHECK = 'legacy_execution_surfaces'


def cases(_module):
    stub = 'def _legacy_delegate_stub(): pass\n'
    return [
        dict(kind='production', id='selection', files=[
            dict(path='l4000_permissions/main.py', source=stub),
            dict(path='l4000_permissions/tests/test_module.py', source=stub),
            dict(path='l4000_permissions/test/test_module.py', source=stub),
            dict(path='api/l4000_permissions/outside.py', source=stub),
            dict(path='other/outside.py', source=stub),
            dict(path='l8000_admin/tools/no_trigger.py', source='def find_common_ancestor(): pass\n'),
            dict(path='l8000_admin/tools/triggered.py', source='# resolve\ndef find_common_ancestor(): pass\n'),
            dict(path='l4000_permissions/broken.py', source='def _legacy_delegate_stub(:\n'),
        ]),
        dict(kind='production', id='empty', files=[]),
        *[dict(kind='cli', id=name, check=check, verbose=verbose, verified=verified,
               findings=findings)
          for name, check, verbose, verified, findings in [
              ('clean', True, False, True, []),
              ('blocked', True, False, False, []),
              ('verbose', True, True, True, [
                  ['LEG-080', 'l2070_batching/a.py', 7, 'bad $value'],
                  ['LEG-001', 'l4000_permissions/z.py', 1, 'second failure\ncontinuation'],
              ]),
              ('plain', False, False, True, [
                  ['LEG-050', 'l4000_permissions/a.py', 3, 'bad'],
              ]),
          ]],
    ]


def exercise(module, case, temporary):
    if case['kind'] == 'production':
        root = temporary / 'repo'
        root.mkdir()
        write_fixture(root, case['files'])
        if hasattr(module, '_rerail_legacy_production'):
            return module._rerail_legacy_production(case['files'])
        plugin = module.legacy_execution_surfaces_plugin(root)
        engine = module.FactEngine(root=root, fact_store=module.JsonFactStore(root/'tmp'/'facts'),
                                   extractors=module.legacy_execution_surfaces_fact_extractors(root))
        view = engine.materialize((plugin,))
        return [dict(path=r.path.relative_to(root).as_posix(), line=r.line, code=r.code, message=r.message)
                for r in plugin.reporter(view)]
    findings = [module.Violation(*row) for row in case['findings']]
    if hasattr(module, '_rerail_legacy_cli'):
        return module._rerail_legacy_cli(findings, case)
    argv = ['lint_legacy_execution_surfaces.py']
    argv.extend(flag for key, flag in [('check', '--check'), ('verbose', '--verbose')] if case[key])
    stdout, stderr = io.StringIO(), io.StringIO()
    with (patch.object(sys, 'argv', argv),
          patch.object(module, '_verify_allowlists', lambda: case['verified']),
          patch.object(module, 'scan_repo', lambda _root: findings),
          redirect_stdout(stdout), redirect_stderr(stderr)):
        status = module.cli_main()
    return dict(status=status, stdout=stdout.getvalue(), stderr=stderr.getvalue())


def install(backend, module, detectors=False):
    from legacy_execution_port import findings, report

    def production(files):
        rows = backend.native(CHECK, None, files)
        return [dict(path=r.file_path, line=r.line_number, code=r.code, message=r.message)
                for r in findings(module, rows, None)]

    def cli(violations, case):
        stdout, status = report(backend, violations, check=case['check'], verbose=case['verbose'],
                                verified=case['verified'])
        return dict(status=status, stdout=stdout, stderr='')

    module._rerail_legacy_production = production
    module._rerail_legacy_cli = cli


if __name__ == '__main__':
    from contract_runner import main

    main(CHECK, 'rerail.legacy-execution-production-contract/v1', cases, exercise, install,
         ['legacy_execution_production_contract.py', 'legacy_execution_port.py'])
