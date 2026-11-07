"""Collect original cases without adapters; read assertion sites from the unchanged module AST."""
import ast
import hashlib
import inspect
import json
import os
import sys
from pathlib import Path

import pytest
from migration import ROOT

sys.path.insert(0, str(ROOT / 'scripts/dsl/data_layer_ten/contracts'))
from inventory_collect import Collection as OriginalCollection


class Collection(OriginalCollection):
    def pytest_collection_finish(self, session):
        parsed = {}
        for item in session.items:
            function = inspect.unwrap(item.obj)
            path = Path(inspect.getsourcefile(function)).resolve()
            if not path.is_relative_to(session.config.rootpath):
                raise ValueError('Original test definition is outside the pinned source: ' + str(path))
            if path not in parsed:
                parsed[path] = ast.parse(path.read_text())
            lines, first = inspect.getsourcelines(function)
            matches = [node for node in ast.walk(parsed[path]) if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
                       and node.name == function.__name__
                       and min([node.lineno, *[d.lineno for d in node.decorator_list]]) == first]
            if len(matches) != 1:
                raise ValueError(f'Cannot locate the complete original definition: {item.nodeid}')
            assertions = [dict(line=node.lineno - first + 1, expression=ast.unparse(node.test))
                          for node in ast.walk(matches[0]) if isinstance(node, ast.Assert)]
            self.cases[item.nodeid] = dict(file=item.path.relative_to(session.config.rootpath).as_posix(),
                function=getattr(item, 'originalname', None) or item.name,
                definitionSha256=hashlib.sha256(''.join(lines).encode()).hexdigest(), assertSites=assertions,
                conditionalMarkers=[m.name for m in item.iter_markers() if m.name in {'skip', 'skipif', 'xfail'}])


if __name__ == '__main__':
    root, filename, output = Path(sys.argv[1]).resolve(), sys.argv[2], Path(sys.argv[3]).resolve()
    os.chdir(root)
    sys.path[:0] = [str(root), str(root / 'scripts')]
    collector = Collection()
    code = pytest.main(['--collect-only', '--rootdir', str(root), '-c', str(root / 'pyproject.toml'), '-q', filename], plugins=[collector])
    output.write_text(json.dumps(dict(exitCode=int(code), before=collector.before, cases=collector.cases,
        deselected=collector.deselected, errors=collector.errors), indent=2) + '\n')
    if code or collector.errors or collector.deselected or set(collector.before) != set(collector.cases):
        raise SystemExit(1)
