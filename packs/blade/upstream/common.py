"""Shared by the upstream/<tool>/generate.py generators: the expected-findings format `rerail/conformance/languages/pack.ts`
reads (`expectedFindingsSchema`)."""

from __future__ import annotations

import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2] / "scripts" / "s3"))
import pack_fixtures  # noqa: E402


def write(path: pathlib.Path, tool: str, version: str, source: dict, findings: list[dict]) -> None:
    findings = sorted(findings, key=lambda f: (f["rule"], f["path"], f["line"], f.get("column", 0)))
    path.write_text(json.dumps({"tool": tool, "version": version, "source": source, "findings": findings},
                               indent=1, ensure_ascii=False) + "\n")
    pack_fixtures.expected_findings_written(path, source)
