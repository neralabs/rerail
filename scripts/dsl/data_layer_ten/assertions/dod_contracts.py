# SPDX-License-Identifier: Apache-2.0
"""Original configured approval results with explicitly isolated, public fixture keys."""
import argparse
from dataclasses import asdict
import hashlib
import importlib
import inspect
import json
from pathlib import Path
import sys
import tempfile


def record(source, output):
    sys.path[:0] = [str(source), str(source / 'scripts')]
    from nacl.signing import SigningKey
    from scripts.lint_lib import signing
    from scripts.lint_lib.fact_engine import FactEngine, _run_plugin_reporter
    from scripts.lint_lib.fact_store import JsonFactStore
    from data_layer_probe_record import verify_sources
    from data_layer_nine.evidence import PIN
    schema = importlib.import_module('scripts.lint_schema_introspection_data_paths')
    # This published deterministic vector is not a deployed approval credential.
    fixture_key = SigningKey(bytes(range(32)))
    public = fixture_key.verify_key.encode().hex()
    other_public = SigningKey(bytes(reversed(range(32)))).verify_key.encode().hex()
    path = 'l2050_transaction/consumer.py'
    text = 'def f():\n    table_exists()\n    table_exists_async()\n'
    first, second = path + ':2:table_exists', path + ':3:table_exists_async'
    rows = [
        ('empty', [], ''),
        ('invalid-internal', ['impl/bad.py:1:table_exists'], 'NEEDS_SIGNING'),
        ('invalid-test', ['l2050_transaction/tests/bad.py:1:table_exists'], 'NEEDS_SIGNING'),
        ('invalid-multiple', ['impl/tests/bad.py:1:table_exists', 'scripts/bad.py:1:table_exists'], ''),
        ('invalid-duplicates', ['impl/bad.py:1:table_exists'] * 2, ''),
        ('unsigned', [first], 'NEEDS_SIGNING'),
        ('empty-signature', [first], ''),
        ('malformed-signature', [first], 'not-a-signature'),
        ('approved-one', [first], None),
        ('approved-all', [first, second], None),
        ('approved-duplicates', [first, first], None),
        ('approved-unicode', [first, 'l2050_transaction/\ue000.py:1:table_exists', 'l2050_transaction/😀.py:1:table_exists'], None),
        ('tampered-entries', [first, second], 'tampered'),
        ('wrong-trust-key', [first], None),
    ]
    import base64
    def sign(entries):
        return base64.b64encode(fixture_key.sign(signing.compute_allowlist_message(schema.LINTER_NAME, sorted(entries))).signature).decode()
    cases = []
    old_root, old_entries, old_signature, old_key = schema.ROOT, schema.APPROVED_USER_SCHEMA_CHECK_CALLSITES, schema.APPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE, signing.PUBLIC_KEY_HEX
    try:
        for label, entries, signature in rows:
            signature = sign(entries) if signature is None else sign([first]) if signature == 'tampered' else signature
            with tempfile.TemporaryDirectory(prefix='rerail-approvals-') as directory:
                root = Path(directory)
                file = root / path
                file.parent.mkdir(parents=True)
                file.write_text(text)
                schema.ROOT = root
                schema.APPROVED_USER_SCHEMA_CHECK_CALLSITES = entries
                schema.APPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE = signature
                signing.PUBLIC_KEY_HEX = other_public if label == 'wrong-trust-key' else public
                plugin = schema.schema_introspection_data_paths_plugin(root)
                engine = FactEngine(root=root, fact_store=JsonFactStore(root / '.facts'), extractors=schema.schema_introspection_data_paths_fact_extractors(root))
                findings = _run_plugin_reporter(plugin, engine.materialize((plugin,), max_workers=1)).violations
                expected = [dict(code=f.code, path=f.path.relative_to(root).as_posix() if f.path.is_absolute() else f.path.as_posix(), line=f.line, message=f.message) for f in findings]
                cases.append(dict(id=label, entries=entries, signature=signature, trustedPublicKey=signing.PUBLIC_KEY_HEX,
                                  canonicalMessage=signing.compute_allowlist_message(schema.LINTER_NAME, sorted(entries)).decode(),
                                  files=[dict(path=path, source=text, language='python')], expected=expected))
    finally:
        schema.ROOT, schema.APPROVED_USER_SCHEMA_CHECK_CALLSITES, schema.APPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE, signing.PUBLIC_KEY_HEX = old_root, old_entries, old_signature, old_key
    signer = '\n\n'.join(inspect.getsource(getattr(signing, name)) for name in ['compute_allowlist_message', 'sign_allowlist', 'update_signature_in_file', 'sign_and_update_allowlist'])
    output.write_text(json.dumps(dict(format='rerail.data-layer-dod-contracts/v1', sourcePin=PIN, sourceFilesChecked=verify_sources(source, source, PIN),
                                     deployedPublicKey=old_key, fixturePublicKey=public, fixtureKeyNotice='Public deterministic test vector only; never trusted by the shipped pack.',
                                     originalSignerFunctions=signer, originalSignerSha256=hashlib.sha256(signer.encode()).hexdigest(), approvalCases=cases), indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    record(args.source.resolve(), args.output)
