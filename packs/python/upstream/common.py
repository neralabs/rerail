"""Shared by the upstream/<tool>/generate.py recorders: the recording format `rerail/conformance/languages/pack.ts`
reads (`recordingSchema`), and the pins every recorder refuses any other than.

Run under CPython 3.14."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
PACK = HERE.parent / "conformance.json"
sys.path.insert(0, str(HERE.parents[2] / "scripts" / "s3"))
import pack_fixtures  # noqa: E402

# Ruff's CLI default target version, and the engine's default `pythonVersion`: the ports keep it.
TARGET_VERSION = "py310"


def require_python() -> None:
    if sys.version_info[:2] != (3, 14):
        raise SystemExit(f"run under CPython 3.14, not {sys.version.split()[0]}")


def tool_version(executable: str, expected: str) -> str:
    """The tool's version from `--version`, refused unless it is the pinned one."""
    text = subprocess.run([executable, "--version"], capture_output=True, text=True, check=True).stdout
    version = text.split()[1]
    if version != expected:
        raise SystemExit(f"{executable} is {version}; the pack pins {expected}")
    return version



def write(path: pathlib.Path, tool: str, version: str, source: dict, findings: list[dict]) -> None:
    findings = sorted(findings, key=lambda f: (f["rule"], f["path"], f["line"], f.get("column", 0)))
    path.write_text(json.dumps({"tool": tool, "version": version, "source": source, "findings": findings},
                               indent=1, ensure_ascii=False) + "\n")
    pack_fixtures.recording_written(path, source)


def origins(files_by_rule: dict[str, list[str]]) -> dict[str, dict]:
    """`source.units`: each copied upstream file, by path, with the rules whose upstream tests it came from."""
    rules_by_file: dict[str, set[str]] = {}
    for rule, files in files_by_rule.items():
        for file in files:
            rules_by_file.setdefault(file, set()).add(rule)
    return {file: {"rules": sorted(rules)} for file, rules in sorted(rules_by_file.items())}


def shipped(tool: str) -> set[str]:
    """The upstream rules of the pack's shipped ports (conformance.json policies under rules/) for `tool`."""
    return {rule["upstream"]["rule"] for rule in json.loads(PACK.read_text())["rules"]
            if rule["upstream"]["tool"] == tool and rule["policy"].startswith("rules/")}


def suppression_cases(cases: dict[str, str], comments: list[str], tool: str,
                      names: dict[str, str] | None = None) -> dict[str, str]:
    """One file per rule: its case (`{c}` where the reported line ends) written once per comment, the first being none.
    `{code}`, `{name}` and `{other}` in a comment are the rule's code, its name in `names` and another rule's code."""
    missing = sorted(shipped(tool) - set(cases))
    if missing:
        raise SystemExit(f"no suppression case for the shipped {tool} rules {', '.join(missing)}")
    files = {}
    for code, case in sorted(cases.items()):
        blocks = []
        for comment in comments:
            other = "B102" if code == "B101" else "B101"
            text = comment.format(code=code, name=(names or {}).get(code, ""), other=other)
            blocks.append(case.replace("{c}", text).replace("{{", "{").replace("}}", "}"))
        files[f"{code}.py"] = "\n".join(blocks)
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
