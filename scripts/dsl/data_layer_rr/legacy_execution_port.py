"""Transport original legacy execution scanner inputs and typed finding values."""
import ast
import hashlib
from pathlib import Path

CHECK = 'legacy_execution_surfaces'
BUFFER = 'l2050_transaction/legacy-input.py'


def repository_files(root):
    files = []
    for path in sorted(p for p in root.rglob('*') if p.is_file()):
        raw = path.read_bytes()
        logical = path.relative_to(root).as_posix()
        try:
            files.append(dict(path=logical, source=raw.decode('utf-8')))
        except UnicodeDecodeError:
            files.append(dict(path=logical, bytesHex=raw.hex()))
    return files


def snapshot(module, entry, arguments):
    scope = dict(include=['**/*'], exclude=[])
    if entry == 'scan_repo':
        root = Path(arguments['root']).resolve()
        return repository_files(root), scope, root
    path = Path(arguments['rel_path'])
    root = Path(module.__file__).resolve().parents[1]
    if path.is_absolute():
        path = path.resolve()
        if path.is_relative_to(root):
            path = path.relative_to(root)
        else:
            root = None
            path = path.relative_to('/')
    else:
        root = None
    return [dict(path=path.as_posix(), source=arguments['source'])], scope, root


def findings(module, rows, entry):
    def order(row):
        p = row['parameters']
        if entry == 'check_exact_file_inventory':
            return p['family'], p['path']
        return p['path'], row['line'], row['code'], p['family'], p['label'], p['order']

    return [module.Violation(r['code'], r['parameters']['path'], r['line'], r['message'])
            for r in sorted(rows, key=order)]


def scan(backend, entry, module, arguments):
    if entry == '_get_allowlists_for_signing':
        rows = backend.native(CHECK, entry, [])
        parts = sorted((r['parameters'] for r in rows if r['code'] == 'LEG_SIGN_PART'),
                       key=lambda p: p['order'])
        entries = {p['part']: [] for p in parts}
        for row in rows:
            if row['code'] == 'LEG_SIGN_ENTRY':
                p = row['parameters']
                entries[p['part']].append(p['value'])
            elif row['code'] != 'LEG_SIGN_PART':
                raise ValueError('Invalid native signed allowlist')
        return {part: sorted(values) for part, values in entries.items()}
    if entry in {'_receiver_tokens', '_looks_like_cursor_receiver', '_is_graph_fallback_guard', '_is_under'}:
        files, options = [], {}
        if entry == '_is_under':
            options = dict(path=arguments['rel_path'], scope=arguments['scope'])
        else:
            source = ast.unparse(arguments['node']) + '\n'
            if entry != '_is_graph_fallback_guard':
                source = '_legacy_input = ' + source
            files = [dict(path=BUFFER, source=source)]
        rows = backend.native(CHECK, entry, files, options)
        if entry == '_receiver_tokens':
            if any(r['code'] != 'LEG_TOKEN' for r in rows):
                raise ValueError('Invalid native receiver token')
            return [r['parameters']['value'] for r in sorted(rows, key=lambda r: -r['parameters']['depth'])]
        if len(rows) > 1 or any(r['code'] != 'LEG_HELPER_TRUE' for r in rows):
            raise ValueError('Invalid native helper result')
        return bool(rows)
    if entry in {'scan_repo', '_iter_python_files'}:
        root = Path(arguments['root'])
        mode = 'repo' if entry == 'scan_repo' else 'python-inventory'
        options = dict(env=['data-layer.legacy-mode=' + mode])
        if entry == 'scan_repo' and not Path.read_text.__module__.startswith('pathlib'):
            # Original pytest I/O mocks run in the harness process. Transport
            # their failures as inputs to the native reporter.
            errors = {}
            for path in root.rglob('*'):
                if path.is_file() and path.suffix in {'.py', '.md', '.rst'}:
                    try:
                        path.read_text(encoding='utf-8')
                    except (OSError, UnicodeDecodeError) as error:
                        errors[path.relative_to(root).as_posix()] = str(error)
            options.update(readFailurePaths=list(errors), readFailureMessages=list(errors.values()))
        rows = backend.native(CHECK, entry, [], options, root=root)
        if entry == '_iter_python_files':
            if any(r['code'] != 'LEG_PYTHON_FILE' for r in rows):
                raise ValueError('Invalid native Python inventory')
            return [root/r['parameters']['path'] for r in sorted(rows, key=lambda r: r['parameters']['order'])]
        return findings(module, rows, entry)
    if entry in {'check_exact_file_inventory', 'check_removed_sql_read_leaf_inventory'}:
        rows = backend.native(CHECK, entry, [], dict(inventoryPaths=sorted(arguments['rel_paths'])))
        return findings(module, rows, entry)
    path, source = arguments['rel_path'], arguments['source']
    rows = backend.native(CHECK, entry, [dict(path=BUFFER, source=source)], dict(inputPath=path))
    if len(rows) == 1 and rows[0]['code'] == 'LEG_SYNTAX':
        row = rows[0]
        detail = row['parameters']['detail']
        if detail != 'source code string cannot contain null bytes':
            key = hashlib.sha256(f'{path}\0{source}'.encode()).hexdigest()
            detail = f'{detail} ({key}.py, line {row["line"]})'
        raise SyntaxError(detail)
    if any(r['code'] == 'LEG_SYNTAX' for r in rows):
        raise ValueError('Unexpected mixed legacy execution result')
    return findings(module, rows, entry)


def signature(backend, value):
    rows = backend.native(CHECK, '_legacy_signature_contract', [], dict(legacySignature=value))
    if len(rows) != 1 or rows[0]['code'] != 'LEG_SIGNATURE':
        raise ValueError('Invalid native legacy signature response')
    return rows[0]['parameters']


def report(backend, violations, *, verbose=False, check=False, verified=True):
    rows = backend.native(CHECK, '_legacy_report', [], dict(
        reportPaths=[v.file_path for v in violations],
        reportLines=[str(v.line_number) for v in violations],
        reportCodes=[v.code for v in violations],
        reportMessages=[v.message for v in violations],
        verbose=str(verbose).lower(), check=str(check).lower(), verified=str(verified).lower()))
    output = sorted((r['parameters'] for r in rows if r['code'] == 'LEG_CLI_OUTPUT'),
                    key=lambda p: p['order'])
    exits = [r['parameters']['status'] for r in rows if r['code'] == 'LEG_CLI_EXIT']
    if len(exits) != 1 or bool(output) != verified or len(output) + len(exits) != len(rows):
        raise ValueError('Invalid native legacy report')
    return ''.join(p['text'] for p in output), exits[0]


def install_configuration(backend, module):
    call_index = len(backend.calls)
    rows = backend.native(CHECK, '_legacy_config', [])
    by_code = {}
    for row in rows:
        by_code.setdefault(row['code'], []).append(row['parameters'])
    values = {}
    for row in by_code['LEG_CONFIG']:
        values.setdefault(row['key'], []).append(row['value'])
    singles = {'__doc__': 'docstring', 'LINTER_NAME': 'signed-linter',
               'LEGACY_WRAPPERS_MODULE': 'canonical-wrapper-module',
               'LEGACY_DELEGATE_CANONICAL_DEF_FILE': 'canonical-wrapper',
               '_ALLOWLISTS_SIGNATURE': 'signed-signature'}
    sets = {'DIRECT_LEGACY_SURFACE_DECORATORS': 'direct-decorator', 'LEGACY_WRAPPER_DECORATORS': 'boundary-decorator',
            'ADMIN_EAGER_TERMINAL_ALLOWLIST': 'eager-terminal-allow', 'LEGACY_WRAPPER_IMPORT_ALLOWLIST': 'wrapper-allow',
            'LEGACY_DELEGATE_ALLOWLIST': 'stub-allow', 'SKIP_DIRS': 'repository-skip',
            'RAW_TXN_METHODS': 'raw-transaction', 'RAW_CURSOR_METHODS': 'raw-cursor',
            'VERSIONING_COMPAT_WRAPPERS': 'versioning-helper'}
    bindings = {name: values[key][0] for name, key in singles.items()}
    bindings.update({name: frozenset(values[key]) for name, key in sets.items()})
    scopes = sorted(by_code['LEG_CONFIG_SCOPE'], key=lambda row: row['order'])
    allow = by_code['LEG_CONFIG_SCOPE_PATH']
    bindings['GRAPH_FALLBACK_SCOPE'] = next(p['scope'] for p in scopes if p['rule_code'] == 'LEG-060')
    for name, code in [('RAW_TXN_SCOPE_ALLOWLISTS', 'LEG-040'), ('RAW_CURSOR_SCOPE_ALLOWLISTS', 'LEG-050')]:
        bindings[name] = {p['scope']: frozenset(row['path'] for row in allow
                                              if (row['rule_code'], row['scope']) == (code, p['scope']))
                          for p in scopes if p['rule_code'] == code}
    inventory = sorted(by_code['LEG_CONFIG_INVENTORY'], key=lambda row: row['order'])
    bindings['EXACT_FILE_NAME_ALLOWLISTS'] = {
        p['name']: frozenset(row['path'] for row in by_code['LEG_CONFIG_INVENTORY_PATH'] if row['name'] == p['name'])
        for p in inventory}
    for name, value in bindings.items():
        if getattr(module, name) != value:
            raise AssertionError(f'Native legacy execution configuration differs for {name}')
        backend.replace(module, name, value)
    backend.configuration_bindings[CHECK] = dict(nativeCall=call_index, names=list(bindings))


def install(backend, module):
    import argparse
    import sys

    install_configuration(backend, module)

    def verify_allowlists():
        from lint_lib import signing

        value = module._ALLOWLISTS_SIGNATURE
        result = signature(backend, value)
        verified = bool(result['passed'])
        verifier = signing.verify_multi_allowlist
        if verifier.__module__ != signing.__name__:
            # The original tests install a verifier double. Check its callback
            # protocol while requiring the real native verifier to agree.
            observed = verifier(result['linter'], module._get_allowlists_for_signing(), value,
                                warn_only=bool(result['warn_only']))
            if bool(observed) != verified:
                raise AssertionError('Verifier double disagrees with the native verdict')
        else:
            sys.stderr.write(result['stderr'])
        return verified

    def print_report(violations, *, verbose):
        output, _ = report(backend, violations, verbose=verbose)
        sys.stdout.write(output)

    def cli_main():
        parser = argparse.ArgumentParser(description='Audit legacy/internal execution surfaces during purge prep.')
        parser.add_argument('--check', action='store_true', help='Exit 1 when violations are found.')
        parser.add_argument('--verbose', action='store_true', help='Print additional context for each violation.')
        parser.add_argument('--sign-allowlist', action='store_true', help='Sign the allowlists with NERA_SIGNING_KEY.')
        parser.add_argument('--approved-eager-processing-boundary', action='append', default=[], metavar='ENTRY',
                            help='Approved explicit eager-processing boundary entry. Repeat once per new '
                                 'EXPLICIT_EAGER_PROCESSING_BOUNDARY_ALLOWLIST entry.')
        parser.add_argument('--approved-legacy-wrapper-import', action='append', default=[], metavar='PATH',
                            help='Approved legacy wrapper import allowlist path. Repeat once per new '
                                 'LEGACY_WRAPPER_IMPORT_ALLOWLIST entry.')
        args = parser.parse_args()
        if args.sign_allowlist:
            raise NotImplementedError('Legacy allowlist signing administration is pending')
        if not module._verify_allowlists():
            return report(backend, [], verified=False)[1]
        violations = module.scan_repo(Path(module.__file__).resolve().parents[1])
        output, status = report(backend, violations, verbose=args.verbose, check=args.check)
        sys.stdout.write(output)
        return status

    backend.replace(module, '_verify_allowlists', verify_allowlists)
    backend.replace(module, '_print_report', print_report)
    backend.replace(module, 'cli_main', cli_main)
