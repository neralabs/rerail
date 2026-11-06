# SPDX-License-Identifier: Apache-2.0
"""Collect a complete original test module without the port's pytest adapters."""
import ast
import hashlib
import inspect
import json
import os
from pathlib import Path
import sys
import textwrap
import pytest


class Collection:
    def __init__(self):
        self.before = []; self.cases = {}; self.deselected = []; self.errors = []

    @pytest.hookimpl(wrapper=True, tryfirst=True)
    def pytest_collection_modifyitems(self, items):
        self.before = [item.nodeid for item in items]
        yield

    def pytest_deselected(self, items):
        self.deselected.extend(item.nodeid for item in items)

    def pytest_collectreport(self, report):
        if report.failed or report.skipped:
            self.errors.append(dict(nodeid=report.nodeid, outcome=report.outcome, detail=str(report.longrepr)))

    def pytest_collection_finish(self, session):
        for item in session.items:
            text = inspect.getsource(item.obj)
            tree = ast.parse(textwrap.dedent(text))
            assertions = [dict(line=node.lineno, expression=ast.unparse(node.test))
                          for node in ast.walk(tree) if isinstance(node, ast.Assert)]
            self.cases[item.nodeid] = dict(file=item.path.relative_to(session.config.rootpath).as_posix(),
                function=getattr(item, 'originalname', None) or item.name, definitionSha256=hashlib.sha256(text.encode()).hexdigest(),
                assertSites=assertions, conditionalMarkers=[mark.name for mark in item.iter_markers() if mark.name in {'skip', 'skipif', 'xfail'}])


if __name__ == '__main__':
    root, filename, output = Path(sys.argv[1]).resolve(), sys.argv[2], Path(sys.argv[3]).resolve()
    os.chdir(root); sys.path[:0] = [str(root), str(root / 'scripts')]
    collector = Collection()
    code = pytest.main(['--collect-only', '--rootdir', str(root), '-c', str(root / 'pyproject.toml'), '-q', filename], plugins=[collector])
    output.write_text(json.dumps(dict(exitCode=int(code), before=collector.before, cases=collector.cases,
                                    deselected=collector.deselected, errors=collector.errors), indent=2) + '\n')
    if code or collector.errors or collector.deselected or set(collector.before) != set(collector.cases):
        raise SystemExit(1)
