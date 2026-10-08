"""Shared by the rules/<tool>/generate.py recorders: the Blade pack's corpora, their templates, and the recording
format `rerail/conformance/languages/pack.ts` reads (`recordingSchema`)."""

from __future__ import annotations

import json
import pathlib
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
PACK = HERE.parents[2] / "conformance" / "languages" / "blade" / "pack.json"
# The directories the runner never walks (rerail/src/code-rule-enforcer.ts, IGNORED_DIRECTORIES).
IGNORED = {".git", ".hg", ".svn", ".rerail", ".venv", "__pycache__", "node_modules", "dist", "build", ".next",
           ".turbo", "coverage", "target", "vendor"}


def corpus(name: str) -> dict:
    """The pack's declaration of corpus `name`: repository, commit, paths, exclude."""
    for entry in json.loads(PACK.read_text())["corpora"]:
        if entry["name"] == name:
            return entry
    raise SystemExit(f"{name} is not a corpus of {PACK}")


def corpus_files(root: pathlib.Path, name: str) -> list[str]:
    """Corpus-relative `.blade.php` templates under the pack's paths, checked out at the pack's commit: the files the
    runner scores."""
    declared = corpus(name)
    head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != declared["commit"]:
        raise SystemExit(f"{root} is at {head}, the pack pins {declared['commit']}")
    files = set()
    for path in declared["paths"]:
        for file in (root / path).rglob("*.blade.php"):
            relative = file.relative_to(root)
            if file.is_file() and not IGNORED.intersection(relative.parts):
                files.add(relative.as_posix())
    return sorted(files)


def corpus_source(name: str, rules: list[str], skipped: list[str], unanalysed: list[dict] = ()) -> dict:
    """`unanalysed`: line ranges (`path`, `startLine`, `endLine`) of analysed files the tool did not analyse."""
    declared = corpus(name)
    return {"kind": "corpus", "name": name, "repository": declared["repository"], "commit": declared["commit"],
            "rules": sorted(rules), "skipped": sorted(skipped),
            **({"unanalysed": sorted(unanalysed, key=lambda region: (region["path"], region["startLine"],
                                                                       region["endLine"]))} if unanalysed else {})}


def write(path: pathlib.Path, tool: str, version: str, source: dict, findings: list[dict]) -> None:
    findings = sorted(findings, key=lambda f: (f["rule"], f["path"], f["line"], f.get("column", 0)))
    path.write_text(json.dumps({"tool": tool, "version": version, "source": source, "findings": findings},
                               indent=1, ensure_ascii=False) + "\n")
