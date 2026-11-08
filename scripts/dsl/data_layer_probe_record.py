#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Regenerate source-probe expectations with the pinned Python oracle."""
from __future__ import annotations

import argparse
import ast
from contextlib import nullcontext
import hashlib
import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from data_layer_batch_two import BATCH_TWO, normalize
from data_layer_batch_three import BATCH_THREE
import data_layer_probe_three


def analyze(module, check, root, path, source):
    file = root / path
    if check == "dynamic_module_loading":
        return module._scan_file(file, root, frozenset())
    if check in ("forwardref", "classvar_check"):
        return module.lint_file(file)
    if check == "unresolved_forward_refs":
        return module.analyze_file(file)
    if check == "type_safety_patterns":
        return module._issues_for_files(root, (file,))
    if check == "conn_or_txn_union":
        return module.analyze_file(file, root=root)
    if check == "async_executor":
        return module.check_file(file)
    if check == "async_patterns":
        return module.analyze_source(Path(path), source, tree=ast.parse(source))
    if check == "async_sync_boundary":
        return module.scan_tree(root, paths=[path])
    return module._collect_violations(root)


def category(module, check, message):
    if check == "dynamic_module_loading":
        patterns = (*module._PATTERNS, *module._IMPORT_MODULE_PATTERNS)
        return "pattern-" + str(next(i for i, (_, label) in enumerate(patterns) if label == message))
    if check == "admin_async_purity":
        return "pattern-" + str(next(i for i, (_, label) in enumerate(module.FORBIDDEN_PATTERNS)
                                     if message.startswith(label + ":")))
    if check == "type_safety_patterns":
        name = next(c.name for c in module.IssueCategory if message.startswith(c.value + ":"))
        return "signature-" + name.removeprefix("STR_IN_").lower() if name.startswith("STR_IN_") else "text-" + name.lower()
    return None


def verify_sources(root, repo, revision):
    tree = subprocess.check_output(["git", "-C", str(repo), "ls-tree", "-rz", revision])
    expected = {}
    for entry in tree.split(b"\0"):
        if entry:
            metadata, path = entry.split(b"\t", 1)
            expected[path.decode()] = metadata.split()[2].decode()
    loaded = set()
    for module in tuple(sys.modules.values()):
        value = getattr(module, "__file__", None)
        if value and Path(value).suffix == ".py" and Path(value).resolve().is_relative_to(root):
            loaded.add(Path(value).resolve())
    for path in loaded:
        data = path.read_bytes()
        identity = hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()
        if identity != expected.get(path.relative_to(root).as_posix()):
            raise ValueError(f"Loaded source differs from pinned revision: {path}")
    return len(loaded)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--inputs", type=Path, default=Path("fixtures/host/data-layer-port/batch-02-probes.json"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source_root = args.source.resolve()
    inputs = json.loads(args.inputs.read_text())
    sys.path.insert(0, str(source_root))
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore

    events = []
    # Only source inputs and case names are reused. Findings come from the oracle.
    for event in inputs["events"]:
        if event["production_scope"]:
            continue
        check = event["check"]
        if check not in (*BATCH_TWO, *BATCH_THREE) or len(event["files"]) != 1:
            raise ValueError("Expected a single-file source probe from batch two or three")
        file_input = event["files"][0]
        path, source = file_input["path"], file_input["source"]
        with tempfile.TemporaryDirectory(prefix="rerail-dl-oracle-probe-") as directory:
            root = Path(directory)
            file = root / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(source)
            module = importlib.import_module("scripts.lint_" + check)
            if check in BATCH_THREE:
                events.extend(data_layer_probe_three.record(module, event, source_root, root))
                continue
            raw = analyze(module, check, root, path, source)
            findings = normalize(check, raw, root, [file_input])
            events.append({**event, "findings": findings})
            factory = getattr(module, check + "_plugin")
            extractors = getattr(module, check + "_fact_extractors")(root)
            with patch.object(module, "ROOT", root) if hasattr(module, "ROOT") else nullcontext():
                engine = FactEngine(root=root, fact_store=JsonFactStore(root / "facts"),
                                    extractors=extractors, enable_fact_cache=False)
                result = engine.run_plugins([factory(root)], max_workers=1)[0]
            projected = []
            for violation in result.violations:
                rel = violation.path.relative_to(root) if violation.path.is_absolute() else violation.path
                projected.append({"path": rel.as_posix(), "line": violation.line, "column": None,
                                  "code": violation.code, "message": violation.message,
                                  "tag": category(module, check, violation.message)})
            events.append({**event, "test": event["test"].removesuffix(":analyzer") + ":plugin",
                           "production_scope": True, "findings": projected})
    output = {**inputs, "events": events, "source_mismatches": [],
              "loaded_source_files_checked": verify_sources(source_root, args.repo, inputs["commit"])}
    args.output.write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps({"cases": len(events) // 2, "calls": len(events),
                      "loaded_source_files_checked": output["loaded_source_files_checked"]}))


if __name__ == "__main__":
    main()
