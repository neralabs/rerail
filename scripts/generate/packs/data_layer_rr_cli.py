"""Emit pinned CLI metadata; report logic lives in the shared native library."""
import json


def api_scope(text):
    """Admit supplied API documents; production predicates select their own scope."""
    return '\n'.join('.scope include "**/*.py"' if line.startswith('.scope include ') else line
                     for line in text.splitlines() if not line.startswith('.scope exclude ')) + '\n'


def logical_scope(text, name):
    """Move an existing policy's glob filters behind its production entry point."""
    patterns = {}
    for kind in ['include', 'exclude']:
        lines = [line.split(' ', 2)[2] for line in text.splitlines() if line.startswith('.scope ' + kind + ' ')]
        patterns[kind] = [p for line in lines for p in json.loads('[' + line + ']')]
    result = api_scope(text) + f'.decl {name}(d: number)\n'
    result += f'{name}(d) :- doc(d,_,_), option("entryPoint",_).\n'
    result += f'.decl {name}_included(d: number)\n'
    for pattern in patterns['include']:
        result += f'{name}_included(d) :- doc(d,p,_), glob({json.dumps(pattern)},p).\n'
    result += f'.decl {name}_excluded(d: number)\n'
    for pattern in patterns['exclude']:
        result += f'{name}_excluded(d) :- doc(d,p,_), glob({json.dumps(pattern)},p).\n'
    result += f'{name}(d) :- {name}_included(d), !{name}_excluded(d), !option("entryPoint",_).\n'
    return result


def cli_suffix(source, check):
    from data_layer_rr_text import module, character_class
    original = module(source, check)
    kind = {'backend_type_strings': 'backend', 'no_source_inspection': 'source', 'hasattr_async': 'async',
            'serverplan_grammar_surface': 'surface'}[check]
    descriptions = {'backend': 'Lint for backend type string comparisons',
        'source': 'Lint test files for forbidden source inspection patterns',
        'async': 'Lint for hasattr/getattr async method detection anti-pattern'}
    config = {'description': [descriptions[kind]]} if kind != 'surface' else {}
    if kind == 'backend':
        config.update(epilog=[original.__doc__],
            check_help=['Exit with error code if violations found (for CI)'],
            paths_help=['Paths to check (default: api/ impl/ data_layer/ lXXXX_*/)'])
    elif kind == 'source':
        config.update(check_help=['Exit with error code 1 if violations found'], directory=list(original.TEST_DIRECTORIES))
        config['EXCLUDED_PATHS'] = list(original.EXCLUDED_PATHS)
        config['FORBIDDEN_PATTERNS.pattern'] = [p.pattern for p, _ in original.FORBIDDEN_PATTERNS]
        config['FORBIDDEN_PATTERNS.flags'] = [str(p.flags) for p, _ in original.FORBIDDEN_PATTERNS]
        config['FORBIDDEN_PATTERNS.label'] = [label for _, label in original.FORBIDDEN_PATTERNS]
    elif kind == 'async':
        config['directory'] = [p.rstrip('/') for p in original.DIRECTORIES_TO_CHECK]
        config['layer_regex'] = ['l' + character_class(str.isdecimal) + r'{4}_.+(?s:.*)']
        for name in ['ALLOWED_PATTERNS', 'EXCLUDE_PATTERNS']:
            config[name] = list(getattr(original, name))
        for name in ['HASATTR_ASYNC_PATTERN', 'GETATTR_ASYNC_PATTERN']:
            pattern = getattr(original, name)
            config[name + '.pattern'] = [pattern.pattern]
            config[name + '.flags'] = [str(pattern.flags)]
    else:
        from scripts.lint_lib.architectural_rules import linter_guidance_lines
        config['guidance'] = linter_guidance_lines(check)
    text = '\n.decl tcli_kind(value: symbol)\ntcli_kind(' + json.dumps(kind) + ').\n'
    text += '.decl tcli_config(key: symbol, ordinal: number, value: symbol)\n'
    for key, values in config.items():
        for i, value in enumerate(values):
            text += f'tcli_config({json.dumps(key)},{i},{json.dumps(value)}).\n'
    return text + '#include "data-layer/text-cli.rr"\n'
