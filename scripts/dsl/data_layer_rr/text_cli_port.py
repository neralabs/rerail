"""Transport CLI arguments and callback rows; native relations own reports."""
from pathlib import Path


def emit(suite, check, options, entry='_cli_report', require_exit=True):
    import sys
    rows = suite.native(check, entry, [], options)
    if any(r['code'] not in {'TEXT_CLI_OUTPUT', 'TEXT_CLI_EXIT', 'TEXT_CLI_PATH_ERROR'} for r in rows):
        raise AssertionError('Unexpected native CLI response')
    for p in sorted((r['parameters'] for r in rows if r['code'] == 'TEXT_CLI_OUTPUT'),
                    key=lambda p: (p['phase'], p['group'], p['ordinal'])):
        sys.stdout.write(p['stdout']); sys.stderr.write(p['stderr'])
    errors = [r['parameters'] for r in rows if r['code'] == 'TEXT_CLI_PATH_ERROR']
    if errors:
        # Native rows select the first failure. Path supplies its public exception DTO.
        Path(min(errors, key=lambda p: p['ordinal'])['path']).relative_to(Path(options['sourceRoot']))
        raise AssertionError('Native path error unexpectedly succeeded')
    exits = [r['parameters']['value'] for r in rows if r['code'] == 'TEXT_CLI_EXIT']
    if not require_exit:
        if exits: raise AssertionError('Unexpected status in a CLI prelude')
        return None
    if len(exits) != 1:
        raise AssertionError('One native CLI exit is required')
    return exits[0]


def columns(rows, names):
    return {'cli' + name: [str(row[i]) for row in rows] for i, name in enumerate(names)}


def install(suite, module, check):
    import argparse
    settings = suite.native(check, '_cli_configuration', [])
    if not settings or any(r['code'] != 'TEXT_CLI_CONFIG' for r in settings):
        raise AssertionError('Missing native CLI configuration')
    def values(key):
        return [p['value'] for p in sorted((r['parameters'] for r in settings), key=lambda p: p['ordinal']) if p['key'] == key]
    names = []
    if check == 'no_source_inspection':
        if values('directory') != module.TEST_DIRECTORIES:
            raise AssertionError('Native source-inspection directories differ')
        if values('EXCLUDED_PATHS') != module.EXCLUDED_PATHS:
            raise AssertionError('Native source-inspection exclusions differ')
        for key, actual in [('pattern', [p.pattern for p, _ in module.FORBIDDEN_PATTERNS]),
                            ('flags', [str(p.flags) for p, _ in module.FORBIDDEN_PATTERNS]),
                            ('label', [label for _, label in module.FORBIDDEN_PATTERNS])]:
            if values('FORBIDDEN_PATTERNS.' + key) != actual:
                raise AssertionError('Native source-inspection pattern metadata differs: ' + key)
        names = ['TEST_DIRECTORIES', 'EXCLUDED_PATHS', 'FORBIDDEN_PATTERNS']
    if check == 'hasattr_async':
        if values('directory') != [p.rstrip('/') for p in module.DIRECTORIES_TO_CHECK]:
            raise AssertionError('Native async directories differ')
        names = ['DIRECTORIES_TO_CHECK', 'ALLOWED_PATTERNS', 'EXCLUDE_PATTERNS',
                 'HASATTR_ASYNC_PATTERN', 'GETATTR_ASYNC_PATTERN']
        for name in ['ALLOWED_PATTERNS', 'EXCLUDE_PATTERNS']:
            if values(name) != getattr(module, name):
                raise AssertionError('Native async configuration differs: ' + name)
        for name in ['HASATTR_ASYNC_PATTERN', 'GETATTR_ASYNC_PATTERN']:
            pattern = getattr(module, name)
            if values(name + '.pattern') != [pattern.pattern] or values(name + '.flags') != [str(pattern.flags)]:
                raise AssertionError('Native async pattern metadata differs: ' + name)
        def default_directories(root):
            selected = suite.native(check, '_cli_directories', [], dict(topDirs=[p.name for p in root.iterdir() if p.is_dir()]))
            if any(r['code'] != 'TEXT_CLI_DIRECTORY' for r in selected):
                raise AssertionError('Unexpected native CLI directories')
            return tuple(Path(r['parameters']['value']) for r in sorted(selected, key=lambda r: r['parameters']['ordinal']))
        suite.replace(module, '_default_directories', default_directories)
    if hasattr(suite, 'configuration_bindings'):
        suite.configuration_bindings.setdefault(check, dict(nativeCall=len(suite.calls)-1, names=[]))['names'].extend(names)
    def parser():
        if check == 'backend_type_strings':
            p = argparse.ArgumentParser(description=values('description')[0],
                formatter_class=argparse.RawDescriptionHelpFormatter, epilog=values('epilog')[0])
            p.add_argument('--check', action='store_true', help=values('check_help')[0])
            p.add_argument('paths', nargs='*', type=Path, default=None, help=values('paths_help')[0])
        else:
            p = argparse.ArgumentParser(description=values('description')[0])
            if check == 'no_source_inspection':
                p.add_argument('--check', action='store_true', help=values('check_help')[0])
            module.add_discovery_args(p)
        return p
    def cli_main():
        args = parser().parse_args()
        if check == 'backend_type_strings':
            # argparse supplies [] for an absent nargs='*' positional, as upstream does.
            files = module.find_python_files(args.paths)
            rows = [v for path in files for v in module.check_file(path)]
            options = columns([(v.file, v.line, v.column, v.code, v.message, v.snippet) for v in rows],
                              ['File', 'Line', 'Column', 'Code', 'Message', 'Snippet'])
            options['checkMode'] = str(int(args.check))
        elif check == 'no_source_inspection':
            root, dirs = module.resolve_root_and_dirs(args, default_dirs=values('directory'))
            emit(suite, check, {}, entry='_cli_begin', require_exit=False)
            rows = module.find_violations(root, dirs)
            options = columns([(p, line, content, name) for p, line, content, name in rows],
                              ['File', 'Line', 'Snippet', 'Pattern'])
            options['checkMode'] = str(int(args.check))
            options['sourceRoot'] = str(root)
        else:
            root, dirs = module.resolve_root_and_dirs(args, default_dirs=None)
            if args.dirs is None:
                dirs = module._default_directories(root)
            rows = []
            for directory in dirs:
                path = root / directory
                if path.exists():
                    for item in path.rglob('*.py'):
                        if not module.should_exclude(item):
                            rows.extend((item.relative_to(root), *issue) for issue in module.check_file(item))
            options = columns(rows, ['File', 'Line', 'Pattern', 'Method'])
        return emit(suite, check, options)
    suite.replace(module, 'cli_main', cli_main)
