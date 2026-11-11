"""Transfer type-safety APIs to typed .rr decisions and passive DTO storage."""
import os
from pathlib import Path


def supplied_files(suite, check, entry, sources, options=None):
    files = [dict(path=f'input/{i}/source.py', source=source) for i, (_, _, source) in enumerate(sources)]
    options = dict(options or {}, apiInputDirs=['input'], apiBuffers=[f['path'] for f in files],
                   apiSourcePaths=[p for p, _, _ in sources], apiSourceNames=[p.name for _, p, _ in sources])
    rows = suite.native(check, entry, files, options)
    originals = {file['path']: original for file, (_, original, _) in zip(files, sources, strict=True)}
    return rows, originals


def scan(suite, entry, module, args):
    check = 'type_safety_patterns'
    if entry == 'normalize_path':
        rows = suite.native(check, entry, [], dict(filePath=Path(args['file_path']).as_posix(), basePath=Path(args['base_dir']).as_posix()))
        if len(rows) != 1 or rows[0]['code'] != 'TSP_NORMALIZED':
            raise AssertionError('Native relative-path API must return exactly one value')
        return rows[0]['parameters']['result']
    if entry == 'is_allowlisted':
        rows = suite.native(check, entry, [], dict(category=args['category'].name,
            filePath=Path(args['file_path']).as_posix(), basePath=Path(args['base_dir']).as_posix()))
        if len(rows) > 1 or any(r['code'] != 'TSP_ALLOWED' for r in rows):
            raise AssertionError('Invalid native allowlist predicate')
        return bool(rows)
    if entry == 'find_python_files':
        root = Path(args['directory'])
        # Enumerate without the source scanner's pruning. SQL owns the complete filter.
        paths = [Path(parent) / name for parent, _, names in os.walk(root) for name in names]
        rows, originals = supplied_files(suite, check, entry,
            [(path.relative_to(root).as_posix(), path, '') for path in paths])
        if any(r['code'] != 'TSP_DISCOVERED' for r in rows):
            raise AssertionError('Invalid native discovery output')
        selected = {r['path'] for r in rows}
        return iter(path for name, path in originals.items() if name in selected)
    if entry == '_dedupe_issues':
        issues = args['issues']
        rows = suite.native(check, entry, [], dict(issuePaths=[str(v.file_path) for v in issues],
            issueLines=[v.line_number for v in issues], issueCategories=[v.category.value for v in issues]))
        if any(r['code'] != 'TSP_DEDUPE' for r in rows):
            raise AssertionError('Invalid native issue deduplication')
        return [issues[r['parameters']['ordinal']] for r in sorted(rows, key=lambda r: r['parameters']['ordinal'])]
    if entry == '_issues_for_files':
        paths = list(args['files'])
    else:
        paths = [Path(args['file_path'])]
    root = Path(args['base_dir'])
    sources = []
    for path in paths:
        try:
            content = (args['content'] if entry == 'check_file_content_for_patterns' else path.read_text())
        except (OSError, UnicodeDecodeError):
            continue
        name = (path.relative_to(root).as_posix() if path.is_relative_to(root)
                else '__outside_root__/' + path.as_posix().lstrip('/'))
        sources.append((name, path, content))
    options = dict(callSource=args['content']) if entry == 'check_function_signature_content' else {}
    rows, originals = supplied_files(suite, check, entry, sources, options)
    if any(r['code'] != 'TSP001' for r in rows):
        raise AssertionError('Unexpected native type-safety diagnostic')
    order = {path: i for i, path in enumerate(originals)}
    def key(row):
        ordinal = row['parameters']['ordinal']
        return (order[row['path']], 0, row['line'], ordinal) if ordinal < 5 else (order[row['path']], 1, ordinal, 0)
    return [module.TypeSafetyIssue(category=module.IssueCategory[r['parameters']['category']],
        file_path=originals[r['path']], line_number=r['line'], column=r['parameters']['column'],
        code_snippet=r['parameters']['snippet'], suggestion=r['parameters']['suggestion']) for r in sorted(rows, key=key)]
