"""Shared by the upstream/<tool>/generate.py recorders: the pack's corpora and the files each scores, the files Python
cannot parse, and the recording format `rerail/conformance/languages/pack.ts` reads (`recordingSchema`).

Run under CPython 3.14: the parse check below is CPython's own parser, the way `php -l` is PHP's."""

from __future__ import annotations

import ast
import fnmatch
import json
import pathlib
import subprocess
import sys
import warnings

HERE = pathlib.Path(__file__).resolve().parent
PACK = HERE.parent / "conformance.json"
sys.path.insert(0, str(HERE.parents[2] / "scripts" / "s3"))
import pack_fixtures  # noqa: E402

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


def write(path: pathlib.Path, tool: str, version: str, source: dict, findings: list[dict]) -> None:
    findings = sorted(findings, key=lambda f: (f["rule"], f["path"], f["line"], f.get("column", 0)))
    path.write_text(json.dumps({"tool": tool, "version": version, "source": source, "findings": findings},
                               indent=1, ensure_ascii=False) + "\n")
    pack_fixtures.recording_written(path, source)


def shipped(tool: str) -> set[str]:
    """The upstream rules of the pack's shipped ports (conformance.json policies under rules/) for `tool`."""
    return {rule["upstream"]["rule"] for rule in json.loads(PACK.read_text())["rules"]
            if rule["upstream"]["tool"] == tool and rule["policy"].startswith("rules/")}


def suppression_cases(cases: dict[str, str], comments: list[str], tool: str,
                      names: dict[str, str] | None = None, paths: dict[str, str] | None = None,
                      exempt: dict[str, str] | None = None) -> dict[str, str]:
    """One file per rule: its case (`{c}` where the reported line ends) written once per comment, the first being none.
    `{code}`, `{name}` and `{other}` in a comment are the rule's code, its name in `names` and another rule's code.
    The file is `<code>.py` unless `paths` names another (a stub for a rule the tool reports only in stubs). A shipped
    rule in `exempt` (with the reason) needs no case."""
    missing = sorted(shipped(tool) - set(cases) - set(exempt or {}))
    if missing:
        raise SystemExit(f"no suppression case for the shipped {tool} rules {', '.join(missing)}")
    files = {}
    for code, case in sorted(cases.items()):
        blocks = []
        for comment in comments:
            other = "B102" if code == "B101" else "B101"
            text = comment.format(code=code, name=(names or {}).get(code, ""), other=other)
            blocks.append(case.replace("{c}", text).replace("{{", "{").replace("}}", "}"))
        files[(paths or {}).get(code, f"{code}.py")] = "\n".join(blocks)
    return files


def write_authored(target: pathlib.Path, cases: dict[str, str]) -> list[str]:
    """Writes Rerail's own cases under `target` (paths relative to it) and returns their paths."""
    for path, text in cases.items():
        (target / path).parent.mkdir(parents=True, exist_ok=True)
        (target / path).write_text(text)
    return sorted(cases)


def write_record(tool: str, title: str, files: list[str], holders: str) -> None:
    """upstream/<tool>/UPSTREAM.md: the fixtures Rerail wrote, which the tool's recording scores."""
    rows = "".join(f"| `upstream/{tool}/fixtures/{file}` | written by Rerail |\n" for file in files)
    (HERE / tool / "UPSTREAM.md").write_text(
        f"# {title}: cases Rerail wrote\n\n"
        f"The fixtures listed below were written by Rerail for the pinned tool to score, beside the tests copied from "
        f"upstream (`licenses/{tool}/UPSTREAM.md`): a case per shipped rule on lines carrying the tool's suppression "
        f"comments, and inputs the ports once misread. `{tool}/generate.py` holds them ({holders}) and records the "
        f"tool's findings on them in `fixtures.expected.json`. They are Rerail's (Apache-2.0, the pack's licence), so "
        f"nothing goes under `licenses/`. Paths are relative to the pack directory.\n\n"
        f"| File | Origin |\n|---|---|\n{rows}")
