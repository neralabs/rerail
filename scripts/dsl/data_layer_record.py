#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Record data-layer tests without translating or weakening their assertions.

This records original tests. It does not run Rerail or certify port parity.
The intended native replay adapter belongs under conformance/.
Run with data-layer's Python environment. Full repository snapshots stay outside Git.
"""

from __future__ import annotations

import argparse
from functools import wraps
import hashlib
import importlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

import pytest
import data_layer_batch_two as batch_two
import data_layer_batch_three as batch_three
import data_layer_batch_four as batch_four


BATCH_ONE = (
    "removed_compat_surfaces", "versioned_lifecycle_legacy_surfaces", "no_api_root_imports",
    "backend_type_strings", "hasattr_async", "no_source_inspection", "serverplan_name_inference",
    "no_blanket_grammar_bypass", "serverplan_grammar_surface", "permission_error",
)
ADAPTERS = (
    ("removed_compat_surfaces", "scan_file", "file"),
    ("versioned_lifecycle_legacy_surfaces", "scan_file", "file"),
    ("no_api_root_imports", "check_file", "file"),
    ("backend_type_strings", "check_file", "file"),
    ("hasattr_async", "check_file", "file"),
    ("hasattr_async", "check_source", "source"),
    ("no_source_inspection", "find_violations", "inspection"),
    ("serverplan_name_inference", "violations", "files"),
    ("no_blanket_grammar_bypass", "check_no_bypass_references", "bypass"),
    ("no_blanket_grammar_bypass", "check_no_public_grammar_methods", "public"),
    ("serverplan_grammar_surface", "scan_file", "file"),
    ("permission_error", "check_file", "file"),
    ("plan_scope_side_effects", "scan_repo", "repository"),
)


def snapshot(module, kind, args, kwargs):
    supplied = args[0] if args else next(iter(kwargs.values()))
    if kind == "source":
        return [{"path": "input.py", "source": supplied}], {"include": ["**/*.py"], "exclude": []}, None
    if kind == "file":
        paths = [Path(supplied)]
        root = paths[0].parent
    elif kind == "files":
        paths = list(map(Path, supplied))
        root = Path(os.path.commonpath([str(path.parent) for path in paths])) if paths else Path("/")
    else:
        root = Path(supplied)
        paths = sorted(root.rglob("*.py"))
    scope = {"include": ["**/*.py"], "exclude": []}
    if kind == "inspection":
        directories = kwargs.get("directories", args[1] if len(args) > 1 else None)
        directories = module.TEST_DIRECTORIES if directories is None else directories
        scope = {"include": [f"{directory}/**/*.py" for directory in directories],
                 "exclude": [f"**/{directory}/**" for directory in module.EXCLUDED_PATHS]}
    if kind == "public":
        scope["include"] = [module.SQL_CONNECTION_FILE]
    if kind == "bypass":
        scope["exclude"] = [f"**/{part}/**" for part in ("__pycache__", ".venv", ".git", "tmp")]
    files = []
    for path in paths:
        try:
            source = path.read_bytes().decode("utf-8")
        except (OSError, UnicodeError):
            continue
        # Match Python's universal-newline file reader.
        source = source.replace("\r\n", "\n").replace("\r", "\n")
        files.append({"path": path.relative_to(root).as_posix(), "source": source})
    return files, scope, root


def normalize(check, entry, result, root, files):
    findings = []
    def relative(path):
        path = Path(path)
        return path.relative_to(root).as_posix() if path.is_absolute() else path.as_posix()
    for finding in result:
        column = None
        if check == "hasattr_async":
            line, code, method = finding
            path, message = files[0]["path"], method
        elif check == "no_source_inspection":
            path, line, snippet, label = finding
            path, code, message = relative(path), "NSI001", f"Source inspection forbidden: {label}"
        elif check == "no_blanket_grammar_bypass":
            match = re.fullmatch(r"  (.+?):(\d+): (.*)", finding)
            if match is None:
                raise ValueError("CLI infrastructure diagnostic: " + finding)
            path, line, message = match.groups()
            code = "NGB001"
        elif check == "serverplan_grammar_surface":
            match = re.fullmatch(r"(.+?):(\d+): (.*)", finding)
            if match is None:
                match = re.fullmatch(r"(.+?): (syntax error while scanning.*)", finding)
                if match is None:
                    raise ValueError("Unsupported grammar result: " + finding)
                path, message = match.groups()
                line = 1
            else:
                path, line, message = match.groups()
            path = relative(path)
            kinds = (("forbidden helper definition ", "helper_definition"),
                     ("forbidden direct grammar helper definition ", "direct_helper_definition"),
                     ("forbidden helper attribute ", "helper_attribute"),
                     ("forbidden direct grammar helper attribute ", "direct_helper_attribute"),
                     ("forbidden DDL escape import ", "ddl_import"),
                     ("forbidden DDL escape hatch ", "ddl_escape"),
                     ("syntax error while scanning", "SYNTAX"))
            code = "SPG-" + next(kind for prefix, kind in kinds if message.startswith(prefix))
        elif isinstance(finding, str):
            match = re.fullmatch(r"(.+?):(\d+): ([A-Z0-9_-]+) (.*)", finding)
            if match is None:
                raise ValueError("Unsupported oracle result: " + finding)
            path, line, code, message = match.groups()
        else:
            path = next(getattr(finding, key) for key in ("file_path", "path", "file") if hasattr(finding, key))
            path = relative(path)
            line = getattr(finding, "line_number", getattr(finding, "line", None))
            code = getattr(finding, "code", {"versioned_lifecycle_legacy_surfaces": "VLL001", "no_api_root_imports": "NARI001", "permission_error": "builtin_permission_error"}.get(check))
            message = getattr(finding, "message", None)
            if message is None:
                message = finding.render().split(": ", maxsplit=2)[-1]
            column = getattr(finding, "column", None)
        findings.append({"path": path, "line": int(line), "column": column, "code": code, "message": message,
                         "severity": getattr(finding, "severity", None)})
    return findings



def blob_id(data: bytes) -> str:
    return hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()


class Recorder:
    def __init__(self, root: Path, batch: int = 1) -> None:
        self.root = root
        self.batch = batch
        self.item = None
        self.events: list[dict] = []
        self.reports: dict[str, list[dict]] = {}
        self.collected: list[str] = []
        self.originals = []
        self.depth = 0

    def pytest_configure(self) -> None:
        for check, entry, kind in (*ADAPTERS, *batch_two.ADAPTERS):
            module = importlib.import_module("scripts.lint_" + check)
            original = getattr(module, entry)

            def wrap(original, check, entry, kind, module):
                @wraps(original)
                def observed(*args, **kwargs):
                    if self.item is None or self.depth:
                        return original(*args, **kwargs)
                    event = {"test": self.item.nodeid, "check": check, "entry": entry,
                             "call": 1 + sum(event["test"] == self.item.nodeid for event in self.events)}
                    try:
                        files, scope, root = (batch_two.snapshot(module, entry, args, kwargs)
                                              if kind == "batch_two" else snapshot(module, kind, args, kwargs))
                        event.update(files=files, scope=scope, input_mode=kind)
                        if check in ("dynamic_module_loading", "type_safety_patterns", "admin_async_purity"):
                            event["compare_clause"] = True
                        # Preserve path skipping, including the upstream no_skip fixture.
                        if check == "backend_type_strings" and files:
                            if module.should_skip_file(Path(args[0])):
                                event["scope"] = {"include": [], "exclude": []}
                            event["path_skip_override"] = "no_skip" in self.item.funcargs
                    except Exception as error:
                        event["retained_reason"] = f"Input adapter: {type(error).__name__}: {error}"
                    if not Path.read_text.__module__.startswith("pathlib"):
                        event["retained_reason"] = "Test replaces the Python source reader; retain its I/O assertions upstream"
                    self.depth += 1
                    started = time.perf_counter_ns()
                    try:
                        result = original(*args, **kwargs)
                    finally:
                        event["oracle_ms"] = (time.perf_counter_ns() - started) / 1_000_000
                        self.depth -= 1
                    if "retained_reason" not in event:
                        try:
                            event["findings"] = (batch_two.normalize(check, result, root, files)
                                                  if kind == "batch_two" else normalize(check, entry, result, root, files))
                        except Exception as error:
                            event["retained_reason"] = f"Output adapter: {type(error).__name__}: {error}"
                    # Read-failure tests exercise the Python file reader, not rule semantics.
                    if not event.get("files") and "retained_reason" not in event:
                        event["retained_reason"] = "No readable source files (file discovery or I/O contract)"
                    self.events.append(event)
                    return result
                return observed

            self.originals.append((module, entry, original))
            setattr(module, entry, wrap(original, check, entry, kind, module))
        if self.batch == 3:
            batch_three.install(self)
        if self.batch == 4:
            batch_four.install(self)

    def pytest_collection_finish(self, session) -> None:
        self.collected = [item.nodeid for item in session.items]

    def pytest_runtest_call(self, item) -> None:
        self.item = item

    def pytest_runtest_logreport(self, report) -> None:
        self.reports.setdefault(report.nodeid, []).append({"phase": report.when, "outcome": report.outcome})
        if report.when == "teardown":
            self.item = None

    def pytest_unconfigure(self) -> None:
        for module, entry, original in reversed(self.originals):
            setattr(module, entry, original)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--revision", default="2b1504adc4291daaf080b2e932962b134da1a69a")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--batch", type=int, choices=(1, 2, 3, 4), default=1)
    parser.add_argument("tests", nargs="*", help="Defaults to the selected batch's unchanged test modules.")
    args = parser.parse_args()
    if not args.tests:
        batches = {1: BATCH_ONE, 2: batch_two.BATCH_TWO, 3: batch_three.BATCH_THREE, 4: batch_four.BATCH_FOUR}
        args.tests = [f"scripts/tests/test_lint_{check}.py" for check in batches[args.batch]
                      if (args.source / f"scripts/tests/test_lint_{check}.py").exists()]
    root = args.source.resolve()
    commit = subprocess.check_output(["git", "-C", str(args.repo), "rev-parse", f"{args.revision}^{{commit}}"], text=True).strip()
    tree = subprocess.check_output(["git", "-C", str(args.repo), "ls-tree", "-rz", commit])
    expected = {}
    for record in tree.split(b"\0"):
        if record:
            metadata, path = record.split(b"\t", 1)
            expected[path.decode()] = metadata.split()[2].decode()

    sys.path.insert(0, str(root))
    recorder = Recorder(root, args.batch)
    # Keep the source tree's pytest configuration and available conftest files.
    exit_code = pytest.main(["--rootdir", str(root), "-c", str(root / "pyproject.toml"), "-q", *[str(root / path) for path in args.tests]], plugins=[recorder])
    loaded = set()
    for module in tuple(sys.modules.values()):
        path = getattr(module, "__file__", None)
        if path and Path(path).suffix == ".py" and Path(path).resolve().is_relative_to(root):
            loaded.add(Path(path).resolve())
    loaded.update(root / path for path in args.tests)
    mismatches = [path.relative_to(root).as_posix() for path in sorted(loaded) if blob_id(path.read_bytes()) != expected.get(path.relative_to(root).as_posix())]
    for event in recorder.events:
        reports = recorder.reports.get(event["test"], [])
        event["upstream_passed"] = len(reports) == 3 and all(report["outcome"] == "passed" for report in reports)
    captured = {event["test"] for event in recorder.events}
    eligible = [event for event in recorder.events if event["upstream_passed"] and "retained_reason" not in event]
    output = {
        "format": "rerail.data-layer-recording/v1",
        "commit": commit,
        "missing_test_modules": [f"scripts/tests/test_lint_{check}.py" for check in batch_four.BATCH_FOUR if not (root / f"scripts/tests/test_lint_{check}.py").exists()] if args.batch == 4 else [],
        "checks": list(batch_four.BATCH_FOUR) if args.batch == 4 else sorted({check for check in (*BATCH_ONE, *batch_two.BATCH_TWO, *batch_three.BATCH_THREE, *batch_four.BATCH_FOUR) if any(f"test_lint_{check}.py" in test for test in args.tests)}),
        "python": sys.version,
        "pytest": pytest.__version__,
        "root_conftest_present": (root / "conftest.py").exists(),
        "loaded_source_files_checked": len(loaded),
        "source_mismatches": mismatches,
        "pytest_exit_code": int(exit_code),
        "collected_tests": len(recorder.collected),
        "uncaptured_tests": [nodeid for nodeid in recorder.collected if nodeid not in captured],
        "eligible_calls": len(eligible),
        "positive_calls": sum(bool(event["findings"]) for event in eligible),
        "clean_calls": sum(not event["findings"] for event in eligible),
        "test_outcomes": recorder.reports,
        "events": recorder.events,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n")
    print(f"Recorded {len(recorder.events)} calls: {output['positive_calls']} positive, {output['clean_calls']} clean; {len(output['uncaptured_tests'])} uncaptured tests.")
    raise SystemExit(1 if mismatches or not eligible else int(exit_code))


if __name__ == "__main__":
    main()
