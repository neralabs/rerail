"""Writes python.expected.json: for the Python workspace in project/, what each call ran.

`runtime`: CPython 3.13.14 running project/main.py with every call of the workspace's code traced
(trace.py, through sys.monitoring): per file, each call expression's byte range and line, and the
definitions (file, `def` line) it dispatched to, or a lambda, a class without a workspace
`__init__`, or a callable outside the workspace. main.py runs every call site of the workspace.

Paths are relative to project/. Run offline, not in CI, under the pinned interpreter:
    python3.13 generate.py"""

from __future__ import annotations

import json
import pathlib
import platform
import subprocess
import sys
import tempfile

PYTHON = "3.13.14"
HERE = pathlib.Path(__file__).parent


def main() -> None:
    if platform.python_version() != PYTHON:
        sys.exit(f"generate.py runs under CPython {PYTHON}, not {platform.python_version()}")
    with tempfile.TemporaryDirectory() as directory:
        output = pathlib.Path(directory) / "runtime.json"
        subprocess.run(
            [sys.executable, "-X", "frozen_modules=on", str(HERE / "trace.py"), str(HERE / "project"), str(output)],
            check=True, stdout=subprocess.DEVNULL, env={"PYTHONDONTWRITEBYTECODE": "1", "PYTHONHASHSEED": "0"},
        )
        runtime = json.loads(output.read_text())
    expected = {"python": platform.python_version(), "runtime": runtime}
    (HERE / "python.expected.json").write_text(json.dumps(expected, indent=1) + "\n")


if __name__ == "__main__":
    main()
