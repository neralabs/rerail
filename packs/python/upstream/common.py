"""Shared by the upstream/<tool>/generate.py recorders: the pack's corpora and the files each scores, the files Python
cannot parse, and the recording format `rerail/conformance/languages/pack.ts` reads (`recordingSchema`).

Run under CPython 3.14: the parse check below is CPython's own parser, the way `php -l` is PHP's."""

from __future__ import annotations

import ast
import fnmatch
import json
import pathlib
import re
import subprocess
import sys
import warnings

HERE = pathlib.Path(__file__).resolve().parent
PACK = HERE.parent / "conformance.json"

# The directories a repository audit never enters (`IGNORED_DIRECTORIES`, rerail/src/code-rule-enforcer.ts).
IGNORED_DIRECTORIES = {".git", ".hg", ".svn", ".rerail", ".venv", "__pycache__", "node_modules", "dist", "build",
                       ".next", ".turbo", "coverage", "target", "vendor"}
# The extensions the host routes to the python dialect (`CODE_RULE_DIALECTS.python`, rerail/src/code-rule-languages.ts).
EXTENSIONS = (".py", ".pyi")
# Ruff's CLI default target version, and the engine's default `pythonVersion`: the ports keep it.
TARGET_VERSION = "py310"


def require_python() -> None:
    if sys.version_info[:2] != (3, 14):
        raise SystemExit(f"run under CPython 3.14 (the parse check is its parser), not {sys.version.split()[0]}")


def tool_version(executable: str, expected: str) -> str:
    """The tool's version from `--version`, refused unless it is the pinned one."""
    text = subprocess.run([executable, "--version"], capture_output=True, text=True, check=True).stdout
    version = text.split()[1]
    if version != expected:
        raise SystemExit(f"{executable} is {version}; the pack pins {expected}")
    return version


def corpus(name: str) -> dict:
    """The pack's declaration of corpus `name`: repository, commit, paths, exclude."""
    for entry in json.loads(PACK.read_text())["corpora"]:
        if entry["name"] == name:
            return entry
    raise SystemExit(f"{name} is not a corpus of {PACK}")


def _walk(root: pathlib.Path, directory: pathlib.Path, files: list[str]) -> None:
    if not directory.is_dir():
        files.append(directory.relative_to(root).as_posix())
        return
    for entry in sorted(directory.iterdir()):
        if entry.is_dir() and not entry.is_symlink():
            if entry.name not in IGNORED_DIRECTORIES:
                _walk(root, entry, files)
        elif entry.is_file() and not entry.is_symlink():
            files.append(entry.relative_to(root).as_posix())


def corpus_files(root: pathlib.Path, name: str) -> list[str]:
    """The corpus-relative Python files the runner scores: under the pack's paths, outside the directories a scan
    ignores, not excluded, checked out at the pack's commit."""
    declared = corpus(name)
    head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != declared["commit"]:
        raise SystemExit(f"{root} is at {head}, the pack pins {declared['commit']}")
    candidates: list[str] = []
    for path in declared["paths"]:
        _walk(root, root / path, candidates)
    return sorted({file for file in candidates if file.endswith(EXTENSIONS)
                   and not any(fnmatch.fnmatchcase(file, pattern) for pattern in declared["exclude"])})


def unparsable(root: pathlib.Path, files: list[str]) -> list[str]:
    """The files CPython 3.14 does not parse (or that are not UTF-8): no tool analyses them, and the runner never
    scores them."""
    rejected = []
    for file in files:
        data = (root / file).read_bytes()
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                ast.parse(data.decode("utf-8"), filename=file, type_comments=False)
        except (SyntaxError, UnicodeDecodeError, ValueError):
            rejected.append(file)
    return rejected


def corpus_source(name: str, rules: list[str], skipped: list[str]) -> dict:
    declared = corpus(name)
    return {"kind": "corpus", "name": name, "repository": declared["repository"], "commit": declared["commit"],
            "rules": sorted(rules), "skipped": sorted(skipped)}


def source_lines(path: pathlib.Path) -> list[str]:
    """The file's lines as CPython's tokenizer breaks them (`\\r\\n`, `\\r`, `\\n`), a leading byte-order mark dropped: the
    lines Python's `ast` positions count."""
    text = path.read_bytes().decode("utf-8")
    return re.split(r"\r\n|\r|\n", text.removeprefix("﻿"))


def code_point_column(lines: list[str], line: int, byte_offset: int) -> int:
    """The 1-based code-point column of a 0-based UTF-8 byte offset on a 1-based line (Python's `ast` `col_offset` and
    `end_col_offset`). Refused when the offset falls inside a character or past the line's end."""
    encoded = lines[line - 1].encode("utf-8")
    if byte_offset > len(encoded):
        raise SystemExit(f"byte offset {byte_offset} is past the end of line {line}")
    return len(encoded[:byte_offset].decode("utf-8")) + 1


def write(path: pathlib.Path, tool: str, version: str, source: dict, findings: list[dict]) -> None:
    findings = sorted(findings, key=lambda f: (f["rule"], f["path"], f["line"], f.get("column", 0), f.get("endLine", 0),
                                               f.get("endColumn", 0)))
    path.write_text(json.dumps({"tool": tool, "version": version, "source": source, "findings": findings},
                               indent=1, ensure_ascii=False) + "\n")
