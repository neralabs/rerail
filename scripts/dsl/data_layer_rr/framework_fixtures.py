"""Build passive suite-registration fixtures before native detector guards start.

These are framework configuration assertions, not detector calls. No reporter,
extractor, scanner or subprocess is invoked while preparing their metadata.
The guards remain active for every assertion and every detector invocation.
"""
from dataclasses import replace
import importlib

GOVERNANCE_NAMES = frozenset({
    'markdown_docs', 'function_level_imports',
    'broad_type_surfaces', 'broad_type_surfaces_ratchet',
})


def prepare(backend, targets):
    if not any(target.split('::', 1)[0] == 'scripts/tests/test_lint_all.py' for target in targets):
        return
    suite = importlib.import_module('scripts.lint_all')
    standalone = importlib.import_module('scripts.lint_standalone_linter_entrypoints_plugin')
    # The upstream export is only a projection of this declarative command list.
    scripts = frozenset(script for script, _args in standalone.STANDALONE_LINTER_COMMANDS)
    backend.replace(standalone, 'registered_standalone_linter_scripts', lambda: set(scripts))
    registrations = []
    prepared = set()
    for registration in suite.DL_SUITE.linter_plugin_factories:
        selected = set(registration.names) & GOVERNANCE_NAMES
        if selected:
            # Construct descriptors only. Their deferred reporter is never run.
            descriptor = registration.factory()
            registration = replace(registration, factory=lambda value=descriptor: value)
            prepared.update(selected)
        registrations.append(registration)
    if prepared != GOVERNANCE_NAMES:
        raise AssertionError('Upstream governance metadata fixture changed')
    backend.replace(suite.DL_SUITE, 'linter_plugin_factories', registrations)
