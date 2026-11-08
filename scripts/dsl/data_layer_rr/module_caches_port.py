"""Transport cache scan metadata, selected buffers and typed native results without detector decisions."""
import ast
from pathlib import Path
from ast_transport import visit_columns

ALLOWLIST = 'scripts/allowlists/lint_module_level_caches_allowlist.json'


def ordered(rows):
    return sorted(rows, key=lambda r:(r['path'],r['parameters']['visit'],r['parameters']['suborder']))


def violation(module, row):
    p = row['parameters']
    result = module.Violation(path=row['path'],line=row['line'],col=p['column'],severity=module.Severity(p['severity']),
        message=p['value'],pattern_type=p['pattern_type'],name=p['name'])
    object.__setattr__(result,'_native_key',p['key'])
    object.__setattr__(result,'_native_format',p['formatted'])
    return result


def raw(rows):
    return [dict(path=r['path'],line=r['line'],col=r['parameters']['column'],severity=r['parameters']['severity'],
        message=r['parameters']['value'],pattern_type=r['parameters']['pattern_type'],name=r['parameters']['name'])
        for r in ordered(rows) if r['code']=='MC_FACT']


def metadata(root, dirs=None):
    directory_names = [p.name for p in root.iterdir() if p.is_dir()]
    groups = dirs if dirs is not None else directory_names
    paths, actual, indexes, owners, memberships = [], [], {}, [], []
    part_owners, parts = [], []
    for directory in groups:
        target = root/directory
        if not target.exists():
            continue
        for path in sorted(target.rglob('*.py')):
            relative = path.relative_to(root)
            key = relative.as_posix()
            if key not in indexes:
                indexes[key] = len(paths)
                paths.append(key)
                actual.append(path)
                for part in relative.parts:
                    part_owners.append(str(indexes[key]))
                    parts.append(part)
            owners.append(str(indexes[key]))
            memberships.append(directory)
    return dict(directoryNames=directory_names,candidatePaths=paths,candidatePartOwners=part_owners,
        candidateParts=parts,candidateGroupOwners=owners,candidateGroups=memberships,
        dirsProvided=str(dirs is not None).lower(),scanDirs=dirs or []),actual


def select(suite, root, dirs, include_tests):
    options, actual = metadata(root, dirs)
    options['includeTests'] = str(include_tests).lower()
    rows = suite.native('module_level_caches','_collect_files',[],options)
    files = sorted([r for r in rows if r['code']=='MC_FILE'],key=lambda r:(r['parameters']['group'],r['parameters']['i']))
    pairs = [(actual[r['parameters']['i']],r['path']) for r in files]
    count = next(r['parameters']['value'] for r in rows if r['code']=='MC_FILE_COUNT')
    assert len(pairs)==count
    return pairs


def for_files(suite, module, root, file_pairs):
    # Source rel_by_path is keyed by resolved files and retains the last supplied label.
    labels = {p.resolve():rel for p,rel in file_pairs}
    files, scopes, seen = [], [], set()
    for path, _ in file_pairs:
        path = path if path.is_absolute() else root/path
        if path in seen or path.suffix!='.py' or not path.is_file():
            continue
        seen.add(path)
        try:
            text = path.read_text(encoding='utf-8')
        except (OSError,UnicodeDecodeError):
            continue
        files.append(dict(path=f'l0000_transport/file-{len(files)}.py',source=text))
        scopes.append(labels[path.resolve()])
    rows = suite.native('module_level_caches','_facts_for_files',files,
        dict(documentPaths=[f['path'] for f in files],scopePaths=scopes))
    return rows


def supplied(suite, rel_path, source):
    physical = 'l0000_transport/file.py'
    return suite.native('module_level_caches','scan_file_content',[dict(path=physical,source=source)],
        dict(documentPaths=[physical],scopePaths=[rel_path]))


def graph(suite, rel_path, tree):
    options = dict(visit_columns(tree),originalPath=rel_path)
    return suite.native('module_level_caches','collector',[dict(path='l0000_transport/ast.py',source='pass\n')],options)


def install(suite, module):
    def scan_codebase(root, *, include_tests=False, dirs=None):
        root = Path(root)
        pairs = select(suite,root,dirs,include_tests)
        rows = for_files(suite,module,root,pairs)
        violations = [violation(module,r) for r in ordered(rows) if r['code']=='MC_FACT']
        violations.sort(key=lambda v:(v.path,v.line,v.col,v.name))
        return module.ScanResults(violations=violations,files_scanned=len(pairs))
    def scan_dirs(root=module.ROOT):
        names = [p.name for p in Path(root).iterdir() if p.is_dir()]
        rows = suite.native('module_level_caches','_scan_dirs',[],dict(directoryNames=names))
        return [r['parameters']['value'] for r in sorted(rows,key=lambda r:r['parameters']['i'])]
    def load():
        files = []
        if module.ALLOWLIST_FILE.exists():
            files.append(dict(path=ALLOWLIST,source=module.ALLOWLIST_FILE.read_text(encoding='utf-8')))
        rows = suite.native('module_level_caches','_load_allowlist_entries',files,
            dict(allowlistDisplayPath=str(module.ALLOWLIST_FILE)))
        errors = [r for r in rows if r['code']=='MC_ALLOW_ERROR']
        if errors:
            p = errors[0]['parameters']
            if p['kind']=='KeyError':
                raise KeyError(p['value'])
            raise ValueError(p['value'])
        entries = []
        for r in rows:
            if r['code']!='MC_ALLOW_ENTRY':
                continue
            p = r['parameters']
            entry = module.AllowlistEntry(path=p['path'],pattern_type=p['pattern_type'],name=p['name'],
                classification=p['classification'],reason=p['reason'])
            object.__setattr__(entry,'_native_key',p['key'])
            object.__setattr__(entry,'_native_signature',p['signature'])
            entries.append(entry)
        entries.sort(key=lambda e:e._native_key)
        return entries
    def scan_file_content(rel_path, abs_path, source, tree=None):
        return [violation(module,r) for r in ordered(supplied(suite,rel_path,source)) if r['code']=='MC_FACT']
    def scan_file(abs_path, rel_path):
        try:
            source = Path(abs_path).read_text(encoding='utf-8')
        except (OSError,UnicodeDecodeError):
            return []
        return scan_file_content(rel_path,abs_path,source)
    suite.replace(module.Violation,'allowlist_key',property(lambda self:self._native_key))
    suite.replace(module.Violation,'format',lambda self:self._native_format)
    suite.replace(module.AllowlistEntry,'match_key',property(lambda self:self._native_key))
    suite.replace(module.AllowlistEntry,'signature_entry',property(lambda self:self._native_signature))
    for name, value in [('scan_codebase',scan_codebase),('_scan_dirs',scan_dirs),('_load_allowlist_entries',load),
            ('_load_allowlist_index',lambda:{e._native_key:e for e in load()}),
            ('_collect_files',lambda root,dirs,include_tests:select(suite,Path(root),dirs,include_tests)),
            ('_facts_for_files',lambda root,file_pairs:raw(for_files(suite,module,Path(root),file_pairs))),
            ('scan_file_content',scan_file_content),('_scan_file',scan_file)]:
        suite.replace(module,name,value)
