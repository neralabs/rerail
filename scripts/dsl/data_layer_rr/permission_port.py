"""PermissionError I/O and passive DTO transport; native rules own decisions."""
from pathlib import Path
import json


def scan_file(suite, module, entry, file_path, source=None, tree=None):
    path = Path(file_path)
    try: disk = path.read_text(encoding='utf-8')
    except (OSError, UnicodeDecodeError): return []
    options = dict(permissionSuppliedSource=source) if entry == 'check_file_content' else {}
    rows = suite.native('permission_error', entry, [dict(path=path.name, source=disk)], options)
    rows.sort(key=lambda r: (r['parameters']['collector_order0'], r['parameters']['collector_order1'], r['parameters']['collector_order2']))
    return [module.Violation(path, r['line'], r['message']) for r in rows]


def install_detectors(suite, module):
    suite.replace(module, 'check_file', lambda file_path: scan_file(suite, module, 'check_file', file_path))
    suite.replace(module, 'check_file_content', lambda file_path, source, tree=None:
                  scan_file(suite, module, 'check_file_content', file_path, source, tree))


def install_fact_helpers(suite, module):
    def convert(entry, values):
        rows = suite.native('permission_error', entry, [dict(path='permission-facts.json',
                            source=json.dumps(dict(rows=values), ensure_ascii=False))])
        if any(r['code'] != 'PERMISSION_FACT' for r in rows): raise AssertionError('Unexpected native fact conversion')
        rows.sort(key=lambda r: r['parameters']['ordinal'])
        if [r['parameters']['ordinal'] for r in rows] != list(range(len(values))):
            raise AssertionError('Native fact conversion lost an input')
        return [json.loads(r['parameters']['value_json']) for r in rows]
    def from_fact(raw_fact, root):
        value = convert('_permission_from_fact', [dict(raw=raw_fact, root=str(root))])[0]
        if value is None: return None
        if type(value['line_number']) is not int: raise AssertionError('Native integer conversion is missing')
        return module.Violation(Path(value['file_path']), value['line_number'], value['message'])
    def to_fact(violation, root):
        return convert('_permission_to_fact', [dict(root=str(root), raw=dict(file_path=str(violation.file_path),
                       line_number=violation.line_number, message=violation.message))])[0]
    suite.replace(module, '_violation_from_fact', from_fact)
    suite.replace(module, '_violation_to_fact', to_fact)


def install(suite, module, detectors=False):
    import argparse
    from text_cli_port import columns, emit
    check = 'permission_error'
    rows = suite.native(check, '_cli_configuration', [])
    if not rows or any(r['code'] != 'TEXT_CLI_CONFIG' for r in rows):
        raise AssertionError('Missing native PermissionError configuration')
    def values(key):
        return [p['value'] for p in sorted((r['parameters'] for r in rows if r['parameters']['key'] == key), key=lambda p: p['ordinal'])]
    def find_python_files(root, exclude_dirs):
        paths = list(root.rglob('*.py'))
        found = suite.native(check, '_permission_files', [], dict(candidateFiles=[p.relative_to(root).as_posix() for p in paths], excludeDirs=list(exclude_dirs)))
        if any(r['code'] != 'PERMISSION_FILE' for r in found): raise AssertionError('Invalid native discovery rows')
        return [root/r['parameters']['value'] for r in sorted(found, key=lambda r: r['parameters']['ordinal'])]
    def violations_for_files(root, files):
        batch, owners = [], {}
        for index, path in enumerate(files):
            try: disk = path.read_text(encoding='utf-8')
            except (OSError, UnicodeDecodeError): continue
            key = f'file-{index}/{path.name}'; owners[key] = (index, path)
            batch.append(dict(path=key, source=disk))
        findings = suite.native(check, 'check_file', batch)
        findings.sort(key=lambda r: (owners[r['path']][0], r['parameters']['collector_order0'], r['parameters']['collector_order1'], r['parameters']['collector_order2']))
        return [module.Violation(owners[r['path']][1], r['line'], r['message']) for r in findings]
    def cli_main():
        parser = argparse.ArgumentParser(description=values('description')[0])
        parser.add_argument('--check', action='store_true', help=values('check_help')[0])
        args, _unknown = parser.parse_known_args()
        root = module.get_project_root()
        files = module.find_python_files(root, set(values('exclude')))
        violations = module._violations_for_files(root, tuple(files))
        options = columns([(v.file_path, v.line_number, v.message) for v in violations], ['File', 'Line', 'Message'])
        options.update(sourceRoot=str(root), checkMode=str(int(args.check)))
        return emit(suite, check, options)
    suite.replace(module, 'get_project_root', lambda: Path(module.__file__).parent.parent)
    suite.replace(module, 'find_python_files', find_python_files)
    suite.replace(module, '_violations_for_files', violations_for_files)
    suite.replace(module, 'cli_main', cli_main)
    install_fact_helpers(suite, module)
    if detectors: install_detectors(suite, module)
