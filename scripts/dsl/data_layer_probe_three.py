#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run third-batch source probes and preserve separate reporter contracts."""
from __future__ import annotations

import ast
from contextlib import ExitStack
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from data_layer_batch_three import TAGGED, findings


def analyzer(module, check, root, path, source, config):
    file = root / path
    if check == "plan_scope_side_effects":
        return module.scan_repo(root)
    if check == "plan_scope_ownership":
        return module._lint_file(root, file)
    if check == "transaction_closures":
        return module.lint_source(file, source, tree=ast.parse(source))
    if check == "transaction_callback_wrappers":
        return module.scan_source(file, source, root=root)
    if check == "transaction_patterns":
        categories = config.get("categories")
        if categories is None:
            categories = module._get_categorized_functions(module._load_baseline(root))
        config["categories"] = {key: sorted(names) for key, names in categories.items()}
        return module.analyze_file(file, {key: set(names) for key, names in categories.items()})
    if check == "lazy_result_usage":
        return module._scan_source(file, source, root)
    if check == "swallowed_exceptions":
        return module.scan_file_content(path, str(file), source)
    if check == "error_handling":
        return [{"path": file, "line": line, "message": message}
                for _, line, message in module.scan_file_content(file, root, source)]
    if check == "module_level_caches":
        return module.scan_file_content(path, file, source)
    return [{"file_path": file, "line": line, "description": label}
            for _, line, _, label in module.scan_file_content(file, root, source)]


def category(module, check, message):
    if check == "connection_caches":
        return "pattern-" + str(next(i for i, (_, label) in enumerate(module.ALL_PATTERNS) if label == message))
    if check == "error_handling":
        return "log-reraise" if message.startswith("log-then-reraise") else "admin-commit"
    if check == "transaction_callback_wrappers":
        return "inline-reentrant" if "inline transaction" in message else "callback-helper"
    return None


def record(module, event, source_root, root):
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore

    root = root.resolve()
    check = event["check"]
    file_input = event["files"][0]
    path, source = file_input["path"], file_input["source"]
    # Copy public, already signed oracle artifacts unchanged. No signing operation occurs.
    for name in (".transaction-patterns-baseline.json", ".swallowed-exceptions-baseline.json"):
        original = source_root / name
        if original.exists():
            (root / name).write_bytes(original.read_bytes())
    with ExitStack() as stack:
        # Model project-local files under the temporary project root.
        for name in ("ROOT", "REPO_ROOT"):
            if hasattr(module, name):
                stack.enter_context(patch.object(module, name, root))
        config = {"source_gate": check not in ("connection_caches", "plan_scope_ownership", "lazy_result_usage"),
                  **event.get("oracle_config", {})}
        raw = analyzer(module, check, root, path, source, config)
        recorder = SimpleNamespace(item=SimpleNamespace(funcargs={"tmp_path": root}))
        normalized = findings(check, raw, path, recorder)
        first = {**event, "oracle_config": config, "compare_clause": check in TAGGED, "findings": normalized}
        extractor_name = {"transaction_closures": "transaction_closure", "transaction_patterns": "transaction_pattern",
                          "transaction_callback_wrappers": "transaction_callback_wrapper"}.get(check, check)
        extractors = getattr(module, extractor_name + "_fact_extractors")(root)
        engine = FactEngine(root=root, fact_store=JsonFactStore(root / "facts"),
                            extractors=extractors, enable_fact_cache=False)
        result = engine.run_plugins([getattr(module, check + "_plugin")(root)], max_workers=1)[0]
        projected = []
        for violation in result.violations:
            rel = violation.path.relative_to(root) if violation.path.is_absolute() else violation.path
            projected.append({"path": rel.as_posix(), "line": violation.line, "column": None,
                              "code": violation.code, "message": violation.message,
                              "tag": category(module, check, violation.message)})
        second = {**event, "test": event["test"].removesuffix(":analyzer") + ":plugin",
                  "production_scope": True, "oracle_config": {},
                  "compare_clause": check in TAGGED, "findings": projected}
        if check == "swallowed_exceptions":
            second["retained_reason"] = "Signed aggregate baseline reporter retained upstream; native replay covers source detections"
        return [first, second]
