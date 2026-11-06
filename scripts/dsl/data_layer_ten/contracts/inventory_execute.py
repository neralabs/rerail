# SPDX-License-Identifier: Apache-2.0
"""Run original registry and workflow assertions without native detector adapters."""
import json
import os
from pathlib import Path
import subprocess
import sys
import pytest
from inventory_collect import Collection


class Execution(Collection):
    def __init__(self):
        super().__init__(); self.outcomes = {}

    def pytest_runtest_logreport(self, report):
        self.outcomes.setdefault(report.nodeid, []).append(dict(phase=report.when, outcome=report.outcome))


if __name__ == '__main__':
    root, output = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    files = sys.argv[3:]
    os.chdir(root); sys.path[:0] = [str(root), str(root / 'scripts')]
    collector = Execution()
    code = pytest.main(['--rootdir', str(root), '-c', str(root / 'pyproject.toml'), '-q', *files], plugins=[collector])
    output.write_text(json.dumps(dict(format='rerail.original-host-test-execution/v1', oracle=True,
        sourcePin=subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip(),
        exitCode=int(code), cases=collector.outcomes, collected=collector.cases,
        deselected=collector.deselected, errors=collector.errors), indent=2) + '\n')
    raise SystemExit(code)
