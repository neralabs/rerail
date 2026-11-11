"""Generate the legacy execution detector from the pinned policy and allowlists."""
from contextlib import redirect_stdout
import io
import json
import re

from data_layer_rr_text import character_class, module


def repository_program(text):
    text = re.sub(r'^\.scope include .*$', '.scope include "**/*"', text, flags=re.M)
    return re.sub(r'^\.scope exclude .*$', '.scope exclude "**/*"', text, flags=re.M)


def generate(source, target, header):
    original = module(source, 'legacy_execution_surfaces')
    plugin = original.legacy_execution_surfaces_plugin(source)
    text = header('legacy_execution_surfaces', target)
    roots = [path.relative_to(source).as_posix() for path in plugin.file_scope.directories]
    skipped = ('.git', '.venv', 'venv', '__pycache__', 'tmp', 'test', 'tests')
    include = [root + '/**/*.py' for root in roots]
    exclude = [f'**/{part}/**' for part in skipped]
    for kind, patterns in [('include', include), ('exclude', exclude)]:
        text = re.sub(r'^\.scope ' + kind + r' .*$', lambda _: '.scope ' + kind + ' ' +
                      ', '.join(map(json.dumps, patterns)), text, flags=re.M)
    text = re.sub(r'^\.description .*$', lambda _: '.description ' + json.dumps(plugin.description),
                  text, flags=re.M)

    feedback = io.StringIO()
    with redirect_stdout(feedback):
        original.print_architectural_feedback_block(original.LINTER_NAME)
    lines = feedback.getvalue().splitlines(keepends=True)
    groups = {
        'wrapper-module': original.LEGACY_WRAPPERS_MODULE_ALIASES,
        'canonical-wrapper-module': [original.LEGACY_WRAPPERS_MODULE],
        'canonical-wrapper': [original.LEGACY_DELEGATE_CANONICAL_DEF_FILE],
        'boundary-decorator': original.LEGACY_WRAPPER_DECORATORS,
        'direct-decorator': original.DIRECT_LEGACY_SURFACE_DECORATORS,
        'stub-allow': original.LEGACY_DELEGATE_ALLOWLIST,
        'wrapper-allow': original.LEGACY_WRAPPER_IMPORT_ALLOWLIST,
        'versioning-helper': original.VERSIONING_COMPAT_WRAPPERS,
        'versioning-allow': original.VERSIONING_COMPAT_ALLOWLIST,
        'raw-transaction': original.RAW_TXN_METHODS,
        'raw-cursor': original.RAW_CURSOR_METHODS,
        'eager-method': original.EAGER_EXECUTION_METHODS,
        'eager-terminal-allow': original.ADMIN_EAGER_TERMINAL_ALLOWLIST,
        'boundary-call': original.LEGACY_EXECUTION_BOUNDARY_CALLS,
        'boundary-call-allow': original.EXPLICIT_EAGER_PROCESSING_BOUNDARY_ALLOWLIST,
        'graph-fallback-allow': original.GRAPH_FALLBACK_ALLOWLIST,
        'source-trigger': plugin.required_facts[0].source_triggers,
        'production-root': roots,
        'production-skip': skipped,
        'layer-path': ['l' + character_class(str.isdigit) + '(?s:.*)'],
        'removed-file': original.REMOVED_SQL_READ_LEAF_FILES,
        'repository-skip': original.SKIP_DIRS,
        'signed-linter': [original.LINTER_NAME],
        'signed-signature': [original._ALLOWLISTS_SIGNATURE],
        'signed-predicate': [original._EXPLICIT_LEGACY_SURFACE_SOURCE],
        'feedback-before': [''.join(lines[:-1])],
        'feedback-after': [lines[-1]],
        'report-separator': ['=' * 72],
        'docstring': [original.__doc__],
    }
    text += '.decl leg_config(key: symbol, value: symbol)\n'
    for key, values in groups.items():
        for value in sorted(values):
            text += f'leg_config({json.dumps(key)}, {json.dumps(value)}).\n'
    text += '.decl leg_raw_scope(code: symbol, scope: symbol, order: number)\n'
    text += '.decl leg_scope_allow(code: symbol, scope: symbol, path: symbol)\n'
    for code, scopes in [('LEG-040', original.RAW_TXN_SCOPE_ALLOWLISTS),
                         ('LEG-050', original.RAW_CURSOR_SCOPE_ALLOWLISTS),
                         ('LEG-060', {original.GRAPH_FALLBACK_SCOPE: original.GRAPH_FALLBACK_ALLOWLIST})]:
        for i, (scope, allow) in enumerate(scopes.items()):
            args = f'{json.dumps(code)}, {json.dumps(scope)}'
            text += f'leg_raw_scope({args}, {i}).\n'
            for path in sorted(allow):
                text += f'leg_scope_allow({args}, {json.dumps(path)}).\n'
    text += '.decl leg_text_scope(path: symbol)\n'
    for path in sorted(original.SQL_POST_FETCH_FILTER_SCOPES):
        text += f'leg_text_scope({json.dumps(path)}).\n'
    text += '.decl leg_filter_token(token: symbol, order: number)\n'
    for i, token in enumerate(sorted(original.SQL_POST_FETCH_FILTER_TOKENS)):
        text += f'leg_filter_token({json.dumps(token)}, {i}).\n'
    text += '.decl leg_inventory_name(name: symbol, order: number)\n.decl leg_inventory_allow(name: symbol, path: symbol)\n'
    for i, (name, paths) in enumerate(original.EXACT_FILE_NAME_ALLOWLISTS.items()):
        text += f'leg_inventory_name({json.dumps(name)}, {i}).\n'
        for path in sorted(paths):
            text += f'leg_inventory_allow({json.dumps(name)}, {json.dumps(path)}).\n'
    text += '.decl leg_signed_part(part: symbol, order: number)\n'
    for i, part in enumerate(original._get_allowlists_for_signing()):
        text += f'leg_signed_part({json.dumps(part)}, {i}).\n'
    return text + '#include "data-layer/legacy-execution-detector.rr"\n'
