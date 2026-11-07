"""Shared by the upstream/<tool>/generate.py recorders: the pack's corpora, the files PHP cannot parse, and the
recording format `rerail/conformance/languages/pack.ts` reads (`recordingSchema`)."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = pathlib.Path(__file__).resolve().parent
PACK = HERE.parent / "conformance.json"
sys.path.insert(0, str(HERE.parents[2] / "scripts" / "s3"))
import pack_fixtures  # noqa: E402


def corpus(name: str) -> dict:
    """The pack's declaration of corpus `name`: repository, commit, paths, exclude."""
    for entry in json.loads(PACK.read_text())["corpora"]:
        if entry["name"] == name:
            return entry
    raise SystemExit(f"{name} is not a corpus of {PACK}")


def corpus_files(root: pathlib.Path, name: str) -> list[str]:
    """Corpus-relative `.php` files under the pack's paths, Blade templates excluded, checked out at the pack's
    commit."""
    declared = corpus(name)
    head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != declared["commit"]:
        raise SystemExit(f"{root} is at {head}, the pack pins {declared['commit']}")
    files = []
    for path in declared["paths"]:
        for file in sorted((root / path).rglob("*.php")):
            if file.is_file() and not file.name.endswith(".blade.php"):
                files.append(file.relative_to(root).as_posix())
    return files


def unparsable(root: pathlib.Path, files: list[str]) -> list[str]:
    """The files `php -l` rejects: no tool analyses them, and the runner never scores them."""
    def rejected(file: str) -> bool:
        return subprocess.run(["php", "-n", "-l", str(root / file)], capture_output=True).returncode != 0
    with ThreadPoolExecutor(max_workers=8) as pool:
        verdicts = list(pool.map(rejected, files))
    return [file for file, bad in zip(files, verdicts) if bad]


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
    pack_fixtures.recording_written(path, source)
