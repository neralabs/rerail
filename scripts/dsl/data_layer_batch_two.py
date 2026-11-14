#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Input and result adapters for unchanged second-batch data-layer tests."""
from __future__ import annotations

import inspect
import importlib
from pathlib import Path

BATCH_TWO = (
    "dynamic_module_loading", "forwardref", "unresolved_forward_refs", "classvar_check",
    "type_safety_patterns", "conn_or_txn_union", "async_executor", "async_patterns",
    "async_sync_boundary", "admin_async_purity",
)
ADAPTERS = (
    ("dynamic_module_loading", "_scan_file", "batch_two"),
    ("dynamic_module_loading", "find_violations", "batch_two"),
    ("forwardref", "lint_file", "batch_two"),
    ("unresolved_forward_refs", "analyze_file", "batch_two"),
    ("classvar_check", "lint_file", "batch_two"),
    ("type_safety_patterns", "check_file_for_patterns", "batch_two"),
    ("type_safety_patterns", "check_function_signatures", "batch_two"),
    ("conn_or_txn_union", "analyze_file", "batch_two"),
    ("async_executor", "check_file", "batch_two"),
    ("async_executor", "check_source", "batch_two"),
    ("async_patterns", "lint_file", "batch_two"),
    ("async_patterns", "analyze_source", "batch_two"),
    ("async_sync_boundary", "scan_tree", "batch_two"),
    ("admin_async_purity", "_collect_violations", "batch_two"),
)


def snapshot(module, entry, args, kwargs):
    values = inspect.signature(getattr(module, entry)).bind(*args, **kwargs).arguments
    path = values.get("file_path", values.get("filepath"))
    root = values.get("root", values.get("base_dir", values.get("project_root")))
    scope = {"include": ["**/*.py"], "exclude": []}
    if path is not None:
        path = Path(path)
        if not path.is_absolute():
            root = Path.cwd()
            path = root / path
        if root is None or not path.is_relative_to(Path(root)):
            root = path.parent.parent if path.parent.name in ("api", "impl", "conform", "public") else path.parent
        root = Path(root)
        paths = [path]
        if entry == "_scan_file" and path.relative_to(root).as_posix() in values["allowlisted"]:
            scope = {"include": [], "exclude": []}
    elif entry == "check_source":
        return [{"path": "input.py", "source": values["source"]}], scope, None
    else:
        root = Path(root)
        if entry == "find_violations":
            paths = module.discover_python_files(root, values.get("directories"), include_tests=False)
            scope["exclude"] = list(module.ALLOWLISTED_FILES)
        elif entry == "_collect_violations":
            paths = []
            for target in module.SCAN_DIRS:
                target = root / target
                paths.extend([target] if target.is_file() else sorted(target.rglob("*.py")))
            paths = [p for p in paths if not module._should_skip(p.relative_to(root).as_posix())
                     and not module._is_allowlisted(p.relative_to(root).as_posix())]
        elif entry == "scan_tree":
            paths = []
            for target in values.get("paths") or ("api", "impl", "conform"):
                target = root / target
                paths.extend([target] if target.is_file() else sorted(target.rglob("*.py")))
        else:
            raise ValueError(f"Unknown batch-two entry: {entry}")
    files = []
    for path in paths:
        try:
            source = values["source"] if "source" in values else path.read_bytes().decode("utf-8")
        except (OSError, UnicodeError):
            continue
        files.append({"path": path.relative_to(root).as_posix(),
                      "source": source.replace("\r\n", "\n").replace("\r", "\n")})
    return files, scope, root


def normalize(check, result, root, files):
    if check == "async_sync_boundary":
        result = result.violations
    findings = []
    for item in result:
        column = None
        tag = None
        if check == "dynamic_module_loading":
            path, line, _snippet, message = item
            code = "DML001"
            module = importlib.import_module("scripts.lint_dynamic_module_loading")
            tag = "pattern-" + str(next(i for i, (_, label) in enumerate((*module._PATTERNS, *module._IMPORT_MODULE_PATTERNS)) if label == message))
        elif check == "async_executor":
            line, message = item
            path, code = files[0]["path"], "run_in_executor"
        elif isinstance(item, dict):
            path, line, message = item["file"], item["line"], item["problem"]
            code = "unresolved_forward_ref"
        else:
            path = next(getattr(item, key) for key in ("file_path", "file") if hasattr(item, key))
            line = getattr(item, "line_number", getattr(item, "line", None))
            code = getattr(item, "code", None)
            if check == "type_safety_patterns":
                code, message = "TSP001", f"{item.category.value}: {item.suggestion}"
                tag = ("signature-" + item.category.name.removeprefix("STR_IN_").lower()
                       if item.category.name.startswith("STR_IN_") else "text-" + item.category.name.lower())
            elif check == "conn_or_txn_union":
                code, message = item.violation_type, item.details
            elif check == "async_sync_boundary":
                code, message = item.category, item.call_text
            elif check == "admin_async_purity":
                code, message = "AAP001", f"{item.pattern}: {item.text}"
                module = importlib.import_module("scripts.lint_admin_async_purity")
                tag = "pattern-" + str(next(i for i, (_, label) in enumerate(module.FORBIDDEN_PATTERNS) if label == item.pattern))
            else:
                message = item.message
            column = getattr(item, "column", getattr(item, "col", None))
        path = Path(path)
        if path.is_absolute():
            path = path.relative_to(root)
        findings.append({"path": path.as_posix(), "line": int(line), "column": column,
                         "code": code, "message": message,
                         "tag": tag,
                         "severity": getattr(item, "severity", None),
                         "is_fallback": getattr(item, "is_fallback", False)})
    return findings
