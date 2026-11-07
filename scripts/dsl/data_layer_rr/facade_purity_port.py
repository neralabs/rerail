"""Transport facade inputs and result DTOs; purity decisions and default targets are native."""
import os
from pathlib import Path


def paths_for(targets):
    paths = []
    for target in map(Path, targets):
        if target.is_file():
            paths.append(target)
        else:
            paths.extend(sorted(path for path in target.glob('*.py') if path.is_file()))
    return paths


def snapshot(module, entry, args, kwargs):
    targets = args[0] if args else kwargs['targets' if entry == 'lint_targets' else 'path']
    paths = paths_for(targets) if entry == 'lint_targets' else [Path(targets)]
    root = Path(os.path.commonpath([str(path.resolve().parent) for path in paths])) if paths else module.REPO_ROOT
    files = []
    for path in paths:
        if path.is_file():
            logical = path.resolve().relative_to(root).as_posix()
            files.append(dict(path=logical, source=path.read_text()))
    return files, dict(include=['**/*.py'], exclude=[]), root


class DefaultTargets:
    """The original constant assertion reads the rule's actual native configuration table."""
    def __init__(self, suite, module):
        self.suite, self.module = suite, module

    def __iter__(self):
        rows = self.suite.native('data_layer_facade_purity', 'default_targets', [])
        if any(row['code'] != 'FAC_CONFIG' for row in rows):
            raise AssertionError('Unexpected facade configuration output')
        return iter(self.module.REPO_ROOT / row['parameters']['path']
            for row in sorted(rows, key=lambda row: row['parameters']['ordinal']))


def scan(suite, entry, module, args):
    paths = paths_for(args['targets']) if entry == 'lint_targets' else [Path(args['path'])]
    originals, files = {}, []
    for index, path in enumerate(paths):
        if not path.is_file():
            continue
        logical = f'input-{index}/{path.name}'
        originals[logical] = path
        # The framework does not read explicit non-Python files.
        source = path.read_text() if path.suffix == '.py' else ''
        files.append(dict(path=logical, source=source))
    rows = suite.native('data_layer_facade_purity', entry, files)
    if any(row['code'] not in {'FAC001', 'FAC002', 'FAC003', 'FAC004'} for row in rows):
        raise AssertionError('Unexpected facade purity diagnostic')
    order = {file['path']: index for index, file in enumerate(files)}
    rows.sort(key=lambda row: (order[row['path']], row['parameters']['ordinal'], row['parameters']['witness']))
    violations = [module.Violation(originals[row['path']], row['line'], row['code'], row['message']) for row in rows]
    return module.LintResults(violations=violations, files_checked=len(paths)) if entry == 'lint_targets' else violations
