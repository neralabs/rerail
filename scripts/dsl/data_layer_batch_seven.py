# SPDX-License-Identifier: Apache-2.0
"""Observe the unchanged seventh-batch analyzers; never translate expected results."""
from __future__ import annotations

import ast
from dataclasses import asdict, is_dataclass
from functools import wraps
import importlib
import inspect
from pathlib import Path
import re
import time
from weakref import WeakKeyDictionary

BATCH_SEVEN = (
    "post_op_side_effects", "post_op_no_internal_dispatch", "executor_publication_repairs",
    "history_serverplan_projection", "revision_ancestry_invariants", "versioned_merged_identity",
    "publication_registry_writes", "declared_fact_interrogation", "grammar_mutation_invalidation",
    "unknown_contract_defaults",
)
MODULES = {name: "scripts.lint_" + name for name in BATCH_SEVEN}
MODULES["versioned_merged_identity"] = "scripts.check_versioned_merged_identity"
TESTS = {name: "scripts/tests/test_" + module.rsplit(".", 1)[1] + ".py" for name, module in MODULES.items()}
CHECKERS = {
    "publication_registry_writes": "PublicationRegistryWritesChecker",
    "declared_fact_interrogation": "DeclaredFactInterrogationChecker",
    "grammar_mutation_invalidation": "GrammarMutationInvalidationChecker",
    "unknown_contract_defaults": "UnknownContractDefaultsChecker",
}


def normalize(check, result, files, logical):
    if result is None:
        return []
    if isinstance(result, dict):
        result = result.get("violations", [])
    output = []
    for item in result:
        if isinstance(item, str):
            match = re.fullmatch(r"(.+?):(?:(\d+):)?\s*(.*)", item)
            if match is None:
                raise ValueError("Unsupported textual finding: " + item)
            path, line, message = match.groups()
            code = "executor_publication_repair" if check == "executor_publication_repairs" else (
                "forbidden_descriptor" if "forbidden ancestry descriptor" in message else
                "removed_surface" if "removed stmt_ancestry_chain" in message else "stored_revision_text_cast")
            value = {"path": path, "line": int(line) if line else 1, "message": message, "code": code}
        else:
            value = asdict(item) if is_dataclass(item) else item if isinstance(item, dict) else vars(item)
        supplied = value.get("path", value.get("file", value.get("file_path")))
        if supplied is None:
            if len(files) != 1:
                raise ValueError("Finding without a file requires an explicit diagnostic-domain adapter")
            supplied = files[0]["path"]
        path = logical(supplied)
        if not any(f["path"] == path for f in files) and len(files) == 1 and Path(path).name == Path(files[0]["path"]).name:
            path = files[0]["path"]
        code = value.get("code") or value.get("category")
        if not code:
            raise ValueError("Finding has no diagnostic identity")
        output.append({"path": path, "line": value.get("line", 1), "column": value.get("col"),
                       "code": code, "message": value.get("message", value.get("detail", ""))})
    return output


def install(recorder):
    trees = WeakKeyDictionary()
    original_parse = ast.parse

    @wraps(original_parse)
    def parsed(source, *args, **kwargs):
        tree = original_parse(source, *args, **kwargs)
        if recorder.item is not None and isinstance(source, (str, bytes)):
            text = source.decode("utf-8") if isinstance(source, bytes) else source
            for node in ast.walk(tree):
                trees[node] = text
        return tree

    recorder.originals.append((ast, "parse", original_parse))
    ast.parse = parsed
    modules = {name: importlib.import_module(module) for name, module in MODULES.items()}

    def wrap(original, check, entry, module):
        signature = inspect.signature(original)

        @wraps(original)
        def observed(*args, **kwargs):
            if recorder.item is None or recorder.depth:
                return original(*args, **kwargs)
            arguments = dict(signature.bind_partial(*args, **kwargs).arguments)
            event = {"test": recorder.item.nodeid, "check": check, "entry": entry,
                     "call": 1 + sum(e["test"] == recorder.item.nodeid for e in recorder.events),
                     "scope": {"include": ["**/*.py"], "exclude": []}, "production_scope": False}
            roots = [arguments.get("root"), recorder.item.funcargs.get("tmp_path"),
                     getattr(module, "ROOT", getattr(module, "REPO_ROOT", recorder.root))]
            roots = [Path(p).resolve() for p in roots if p is not None]

            def logical(path):
                path = Path(path)
                if path.is_absolute():
                    base = next((root for root in roots if path.is_relative_to(root)), None)
                    return path.relative_to(base).as_posix() if base else path.name
                return path.as_posix()

            files = []
            try:
                source = arguments.get("source")
                tree = next((arguments[k] for k in ("tree", "node") if isinstance(arguments.get(k), ast.AST)), None)
                if source is None and tree is not None:
                    source = trees.get(tree)
                path = arguments.get("path", arguments.get("file_path", "input.py"))
                if source is not None:
                    files = [{"path": logical(path), "source": source}]
                elif check == "executor_publication_repairs":
                    path = Path(arguments.get("path", module.EXECUTOR))
                    files = [{"path": logical(path), "source": path.read_text()}]
                elif check == "revision_ancestry_invariants":
                    paths = arguments.get("paths", module.SCAN_ROOTS)
                    paths = [p for root in paths for p in ([Path(root)] if Path(root).is_file() else sorted(Path(root).rglob("*.py")))]
                    files = [{"path": logical(p), "source": p.read_text()} for p in paths]
                elif check == "versioned_merged_identity":
                    root = Path(arguments.get("root", module.ROOT))
                    paths = sorted((root / "impl/common/l2070_server_plan/steps").rglob("*.py"))
                    files = [{"path": logical(p), "source": p.read_text()} for p in paths]
                    event["oracle_config"] = {"options": {"expectedCallers": sorted(arguments.get("expected_callers", module.EXPECTED_CALLERS))}}
                else:
                    raise ValueError("No original source-bearing analyzer input")
                if not files:
                    raise ValueError("Empty source domain needs an absent-owner diagnostic adapter")
                event["files"] = files
                if check in CHECKERS:
                    event["oracle_config"] = {"options": {"sourceAnalyzer": True}}
                if not Path.read_text.__module__.startswith("pathlib"):
                    raise ValueError("Test replaces the source reader; retain its original I/O assertions")
            except Exception as error:
                event["retained_reason"] = "Input adapter: " + type(error).__name__ + ": " + str(error)
            recorder.depth += 1
            started = time.perf_counter()
            try:
                result = original(*args, **kwargs)
            finally:
                event["oracle_ms"] = (time.perf_counter() - started) * 1000
                recorder.depth -= 1
            if "retained_reason" not in event:
                try:
                    raw = args[0].violations if entry == "_RuntimeOpBehaviorVisitor.visit" else result
                    event["findings"] = normalize(check, raw, files, logical)
                except Exception as error:
                    event["retained_reason"] = "Output adapter: " + type(error).__name__ + ": " + str(error)
            recorder.events.append(event)
            return result
        return observed

    entries = {
        "executor_publication_repairs": ["violations", "_extract_executor_publication_facts"],
        "revision_ancestry_invariants": ["violations", "_extract_revision_ancestry_facts"],
        "history_serverplan_projection": ["check_source"],
        "post_op_no_internal_dispatch": ["collect_violations_for_source"],
        "versioned_merged_identity": ["collect_violations"],
        "post_op_side_effects": ["_collect_runtime_allowlist_bypass_violations_from_tree", "_collect_violations"],
    }
    for check, names in entries.items():
        module = modules[check]
        for entry in names:
            original = getattr(module, entry)
            recorder.originals.append((module, entry, original))
            setattr(module, entry, wrap(original, check, entry, module))
    for check, name in CHECKERS.items():
        cls = getattr(modules[check], name)
        original = cls.check_file
        recorder.originals.append((cls, "check_file", original))
        cls.check_file = wrap(original, check, "check_file", modules[check])
    cls = modules["post_op_side_effects"]._RuntimeOpBehaviorVisitor
    original = cls.visit
    recorder.originals.append((cls, "visit", original))
    cls.visit = wrap(original, "post_op_side_effects", "_RuntimeOpBehaviorVisitor.visit", modules["post_op_side_effects"])
