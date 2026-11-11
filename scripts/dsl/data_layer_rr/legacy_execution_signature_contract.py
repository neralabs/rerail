"""Compare the legacy allowlist payload, verdict, and verification streams."""
from contextlib import redirect_stderr
import io

CHECK = 'legacy_execution_surfaces'


def cases(module):
    return [dict(kind='signature', id=name, signature=signature) for name, signature in [
        ('pinned', module._ALLOWLISTS_SIGNATURE),
        ('unsigned', 'UNSIGNED'),
        ('needs-signing', 'NEEDS_SIGNING'),
        ('empty', ''),
        ('malformed', 'a'),
    ]]


def exercise(module, case, _temporary):
    signature = case['signature']
    if hasattr(module, '_rerail_legacy_signature'):
        return module._rerail_legacy_signature(signature)
    from scripts.lint_lib.signing import compute_multi_allowlist_message

    saved = module._ALLOWLISTS_SIGNATURE
    output = io.StringIO()
    try:
        module._ALLOWLISTS_SIGNATURE = signature
        with redirect_stderr(output):
            passed = module._verify_allowlists()
    finally:
        module._ALLOWLISTS_SIGNATURE = saved
    payload = compute_multi_allowlist_message(module.LINTER_NAME, module._get_allowlists_for_signing())
    return dict(passed=passed, stderr=output.getvalue(), payload=payload.decode('utf-8'))


def install(backend, module, detectors=False):
    from legacy_execution_port import signature as read_signature

    def verify(signature):
        p = read_signature(backend, signature)
        return dict(passed=bool(p['passed']), stderr=p['stderr'], payload=p['payload'])

    module._rerail_legacy_signature = verify


if __name__ == '__main__':
    from contract_runner import main

    main(CHECK, 'rerail.legacy-execution-signature-contract/v1', cases, exercise, install,
         ['legacy_execution_signature_contract.py', 'legacy_execution_port.py'],
         guard_modules=('scripts.lint_lib.signing',))
