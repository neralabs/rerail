"""Writes ruff-rules.expected.json: the lines where Ruff's own rules report on small file trees, per case, for the ports
the engine tests pin (T100 over relative imports, B905 by target version, INP001, N999 and A005 by module path, N999's ignored names and
migration directories). Each case's files are written to a scratch directory and checked
there, so package detection and module paths are Ruff's. Run offline, not in CI:
python3 generate_ruff_rules.py <path to a ruff executable>."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
from typing import TypedDict


class Case(TypedDict, total=False):
    rule: str
    target: str
    files: dict[str, str]

T100_IMPORTS: str = """\
import pdb
import pdb.foo
import os, pdb
import IPython.terminal.embed as e
from IPython.terminal import embed
from pdb import foo
from pdb import (set_trace as st)
from pdb import set_trace, foo as bar
from .pdb import set_trace
from . import pdb
from .. import ipdb
from .x import pdb
from celery.contrib import rdb
from celery.contrib.rdb import set_trace
from sys import breakpointhook
from ..celery.contrib.rdb import set_trace
from .. import (ipdb as i, os)
"""

CASES: list[Case] = [
    # T100 names relative imports by their module text, ignoring the dots (`from .pdb import set_trace` is pdb.set_trace,
    # `from . import pdb` is pdb), inside a package and out of one.
    {"rule": "T100", "files": {"t100.py": T100_IMPORTS}},
    {"rule": "T100", "files": {"pkg/__init__.py": "", "pkg/t100.py": T100_IMPORTS}},
    # B905 applies from Python 3.10 (`strict=` is new there): a port gates its query on the target version.
    *(
        Case(rule="B905", target=target, files={"b905.py": "zip(a, b)\nzip(a, b, strict=True)\nzip(a, strict=False)\n"})
        for target in ["py39", "py310", "py313"]
    ),
    # File-level rules read the module path: INP001 whether a file is in a package, N999 the module's name (a
    # package's for its __init__.py), A005 whether a top-level module shadows the standard library.
    {"rule": "INP001", "files": {"pkg/__init__.py": "", "pkg/a.py": "x = 1\n", "pkg/sub/c.py": "x = 1\n", "loose/b.py": "x = 1\n"}},
    {
        "rule": "N999",
        "files": {
            "pkg/__init__.py": "",
            "pkg/Bad.py": "",
            "pkg/good.py": "",
            "pkg/0001_initial.py": "",
            "Mod/__init__.py": "",
            "Mod/fine.py": "",
            "loose/Bad.py": "",
        },
    },
    # N999's ignore-names and migration directories, which ports express with `notRegex`.
    {
        "rule": "N999",
        "files": {
            "pkg/__init__.py": "",
            "pkg/setUp.py": "",
            "pkg/maxDiff.py": "",
            "pkg/class.py": "",
            "pkg/migrations/__init__.py": "",
            "pkg/migrations/0001_initial.py": "",
            "pkg/migrations/Bad.py": "",
            "pkg/versions/__init__.py": "",
            "pkg/versions/1a.py": "",
            "pkg/other/__init__.py": "",
            "pkg/other/0001.py": "",
        },
    },
    {
        "rule": "A005",
        "files": {"logging/__init__.py": "", "utils/__init__.py": "", "utils/logging.py": "", "string.py": "", "fine.py": ""},
    },
    # Dotted directory and file names: N999 and A005 read them as on disk (`json.d`, `my.mod`), where `to_module_path`
    # names a directory by its stem (`json`). The host sends such a module as a list of names.
    {
        "rule": "N999",
        "files": {
            "json.d/__init__.py": "",
            "json.d/m.py": "",
            "Ok.x/__init__.py": "",
            "good/__init__.py": "",
            "good/my.mod.py": "",
            "good/fine.py": "",
            "my.mod.py": "",
        },
    },
    {
        "rule": "A005",
        "files": {"json.d/__init__.py": "", "json.d/logging.py": "", "logging.d/__init__.py": "", "my.logging.py": "", "json.py": ""},
    },
    # Ruff's `seen_module`: an import of the module visited before the node, in Ruff's order (the module body, then
    # function bodies level by level, then lambdas). SIM112 reads `os.environ[...]` by its text once `os` is seen.
    {
        "rule": "SIM112",
        "files": {
            "module_import.py": 'os.environ["before_the_import"]\n\n\ndef reads_later():\n    os.environ["in_a_body"]\n\n\n'
            'import os\n\nos.environ["after_the_import"]\n',
            "body_import.py": 'def first():\n    os.environ["before_the_body_import"]\n\n\ndef second():\n    import os\n\n'
            '    os.environ["after_it"]\n\n\ndef third():\n    os.environ["in_a_later_body"]\n\n    def nested():\n'
            '        os.environ["in_a_nested_body"]\n\n\nlater = lambda: os.environ["in_a_lambda"]\n'
            'os.environ["in_the_module"]\n',
        },
    },
    # ASYNC109 at 3.10 reports only where anyio or trio is seen (asyncio.timeout is 3.11's), at the `def`.
    {
        "rule": "ASYNC109",
        "target": "py310",
        "files": {
            "module_import.py": "async def before(timeout): ...\n\n\nimport trio\n\n\nasync def after(timeout): ...\n\n\n"
            "def outer():\n    async def nested(timeout): ...\n",
            "body_import.py": "def outer():\n    import anyio\n\n\nasync def in_the_module(timeout): ...\n\n\n"
            "def later():\n    async def nested(timeout): ...\n\n\nfrom .relative import trio\n",
        },
    },
]


def module_path(path: pathlib.Path) -> str | list[str] | None:
    """Ruff's `detect_package_root`, climbing while each directory holds an `__init__.py`, and the names from the root's
    parent as on disk, the file's by its stem: dotted, or a list when a name keeps a dot."""
    root: pathlib.Path | None = None
    directory: pathlib.Path = path.parent
    while (directory / "__init__.py").is_file():
        root = directory
        directory = directory.parent
    if root is None:
        return None
    parts: tuple[str, ...] = path.relative_to(root.parent).parts
    names: list[str] = [*parts[:-1], pathlib.PurePath(parts[-1]).stem]
    return names if any("." in name for name in names) else ".".join(names)


def run(ruff: str, case: Case) -> dict[str, object]:
    files: dict[str, str] = case["files"]
    target: str = case.get("target", "py313")
    with tempfile.TemporaryDirectory() as scratch:
        root: pathlib.Path = pathlib.Path(scratch).resolve()
        for relative, source in files.items():
            path: pathlib.Path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source)
        command: list[str] = [ruff, "check", "--no-cache", "--isolated", "--select", case["rule"], "--target-version", target]
        report = subprocess.run(
            [*command, "--output-format", "json", *files], capture_output=True, text=True, check=False, cwd=root
        )
        diagnostics: list[dict[str, str | dict[str, int]]] = json.loads(report.stdout)
        out: list[dict[str, object]] = []
        for relative, source in files.items():
            path = root / relative
            rows: list[int] = []
            for item in diagnostics:
                location = item["location"]
                assert isinstance(location, dict)
                if pathlib.Path(str(item["filename"])).resolve() == path and item["code"] == case["rule"]:
                    rows.append(location["row"])
            rows.sort()
            out.append({"path": relative, "module": module_path(path), "source": source, "rows": rows})
    return {"rule": case["rule"], "target": target, "files": out}


def main() -> None:
    ruff: str = sys.argv[1]
    version: str = subprocess.run([ruff, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    expected: dict[str, object] = {"oracle": f"{version} --isolated", "cases": [run(ruff, case) for case in CASES]}
    (pathlib.Path(__file__).parent / "ruff-rules.expected.json").write_text(json.dumps(expected, indent=1) + "\n")


if __name__ == "__main__":
    main()
