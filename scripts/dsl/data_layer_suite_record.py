#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Record selected batches of plugins together, using the upstream shared fact engine."""
from __future__ import annotations

import argparse
import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from data_layer_record import BATCH_ONE, blob_id
from data_layer_batch_two import BATCH_TWO
from data_layer_batch_three import BATCH_THREE
from data_layer_batch_four import BATCH_FOUR


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--revision", default="2b1504adc4291daaf080b2e932962b134da1a69a")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--batches", default="1", help="Comma-separated batch numbers, currently 1, 2, 3, and 4.")
    args = parser.parse_args()
    root = args.source.resolve()
    sys.path.insert(0, str(root))
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    from scripts.lint_lib.serverplan_facts import serverplan_fact_extractors

    plugins, extractors = [], serverplan_fact_extractors(root)
    batch_ids = [int(value) for value in args.batches.split(",")]
    if any(value not in (1, 2, 3, 4) for value in batch_ids) or len(set(batch_ids)) != len(batch_ids):
        parser.error("--batches requires distinct batch numbers from 1,2,3,4")
    batches = {1: BATCH_ONE, 2: BATCH_TWO, 3: BATCH_THREE, 4: BATCH_FOUR}
    names = [name for value in batch_ids for name in batches[value]]
    for name in names:
        module = importlib.import_module("scripts.lint_" + name)
        if name == "rust_sql_grammar_boundary":
            continue
        factory = getattr(module, name + "_plugin")
        plugins.append(factory() if name == "serverplan_grammar_surface" else factory(root))
        extractor_name = {"transaction_patterns": "transaction_pattern", "transaction_callback_wrappers": "transaction_callback_wrapper",
                          "transaction_closures": "transaction_closure"}.get(name, name)
        factory = getattr(module, extractor_name + "_fact_extractors", None)
        if factory is not None:
            extractors.update(factory(root))
    engine = FactEngine(root=root, fact_store=JsonFactStore(Path(tempfile.mkdtemp(prefix="rerail-dl-facts-"))),
                        extractors=extractors, enable_fact_cache=False)
    plans = {plugin.name: engine.plan([plugin]) for plugin in plugins}
    missing = {spec.extractor_id for plan in plans.values() for spec in plan.required_facts
               if spec.extractor_id.split(".")[0] in (*BATCH_THREE, *BATCH_FOUR) and spec.extractor_id not in extractors}
    if missing:
        raise ValueError(f"Missing oracle extractors: {sorted(missing)}")
    selected = sorted({path for plan in plans.values() for path in plan.files})
    if "rust_sql_grammar_boundary" in names:
        module = importlib.import_module("scripts.lint_rust_sql_grammar_boundary")
        selected = sorted(set(selected) | set(module._rust_files()))
    # Callable-constructor facts discover their grammar inputs outside the plan's sentinel file.
    if "grammar_callable_constructors" in names:
        module = importlib.import_module("scripts.lint_grammar_callable_constructors")
        selected = sorted(set(selected) | set(module._iter_files(root / module.GRAMMAR_ROOT)))
    files = [{"path": path.relative_to(root).as_posix(), "source": path.read_text()} for path in selected]
    tree = subprocess.check_output(["git", "-C", str(args.repo), "ls-tree", "-rz", args.revision])
    expected = {}
    for record in tree.split(b"\0"):
        if record:
            metadata, path = record.split(b"\t", 1)
            expected[path.decode()] = metadata.split()[2].decode()
    mismatches = [file["path"] for file in files if blob_id((root / file["path"]).read_bytes()) != expected.get(file["path"])]
    if mismatches:
        raise ValueError(f"Source files differ from pinned revision: {mismatches[:10]}")
    # Observe the shared view once. The signed swallowed-exception reporter remains
    # separately visible; this migration compares its source detections to native detections.
    captured = []
    materialize = engine.materialize
    def observe(*arguments, **keywords):
        view = materialize(*arguments, **keywords)
        captured.append(view)
        return view
    engine.materialize = observe
    started = time.perf_counter()
    results = engine.run_plugins(plugins, max_workers=args.workers)
    oracle_ms = (time.perf_counter() - started) * 1000
    events = []
    reporter_results = []
    for result in results:
        if result.name == "swallowed_exceptions":
            reporter_results.append({"check": result.name, "contract": "signed aggregate baseline reporter; retained upstream",
                                     "findings": [{"path": v.path.as_posix(), "line": v.line, "code": v.code, "message": v.message} for v in result.violations]})
            view = captured[0]
            findings = []
            for path, bundle in view.files.items():
                for fact in bundle.facts.get("swallowed_exceptions.violations", []):
                    findings.append({"path": str(fact["path"]), "line": fact["line"], "column": fact.get("col"),
                                     "code": {1: "INFO", 2: "WARNING", 3: "CRITICAL"}[fact["severity"]], "message": fact["message"]})
            events.append({"test": "suite:" + result.name, "call": 1, "check": result.name, "entry": "fact-extractor",
                           "upstream_passed": True, "input_mode": "suite", "production_scope": True,
                           "scope": {"include": ["**/*.py"], "exclude": []}, "file_set": "suite", "findings": findings})
            continue
        events.append({"test": "suite:" + result.name, "call": 1, "check": result.name, "entry": "plugin.reporter",
                       "upstream_passed": True, "input_mode": "suite", "production_scope": True,
                       "scope": {"include": ["**/*.py"], "exclude": []}, "file_set": "suite",
                       "findings": [{"path": v.path.relative_to(root).as_posix(), "line": v.line, "column": getattr(v, "column", None),
                                     "code": v.code, "message": v.message, "severity": v.severity} for v in result.violations]})
    if "sql_grammar_escape_hatches" in names:
        module = importlib.import_module("scripts.lint_sql_grammar_escape_hatches")
        original_event = next(event for event in events if event["check"] == "sql_grammar_escape_hatches")
        reporter_results.append({"check": original_event["check"], "contract": "signed aggregate ceiling reporter; retained upstream", "findings": original_event["findings"]})
        original_event["entry"] = "source detections before aggregate ceiling"
        original_event["findings"] = [{"path": Path(v.file_path).relative_to(root).as_posix(), "line": v.line_number, "column": None,
                                       "code": "escape_hatch", "message": v.reason} for v in module._violations_from_view(captured[0])]
    if "rust_sql_grammar_boundary" in names:
        module = importlib.import_module("scripts.lint_rust_sql_grammar_boundary")
        findings = module.check_all()
        events.append({"test": "suite:rust_sql_grammar_boundary", "call": 1, "check": "rust_sql_grammar_boundary", "entry": "check_all",
                       "upstream_passed": True, "input_mode": "suite", "production_scope": True,
                       "scope": {"include": ["**/*.rs"], "exclude": []}, "file_set": "suite",
                       "findings": [{"path": v.path.as_posix(), "line": v.line, "column": None,
                                     "code": "rust_sql_grammar_boundary", "message": v.message} for v in findings]})
    output = {"format": "rerail.data-layer-recording/v1", "commit": args.revision, "source_mismatches": mismatches,
              "checks": names, "batches": batch_ids, "retained_reporter_contracts": reporter_results,
              "pytest_exit_code": 0, "collected_tests": 0, "uncaptured_tests": [], "events": events,
              "oracle_ms": (time.perf_counter() - started) * 1000, "oracle_profile": engine.last_profile_metrics,
              "files": files, "file_count": len(files), "bytes": sum(len(file["source"].encode()) for file in files), "workers": args.workers}
    args.output.write_text(json.dumps(output) + "\n")
    print(json.dumps({key: value for key, value in output.items() if key not in ("events", "oracle_profile", "files")}, indent=2))


if __name__ == "__main__":
    main()
