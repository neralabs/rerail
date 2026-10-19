#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Observe third-batch fact boundaries and AST compatibility facades."""
from __future__ import annotations

import ast
from dataclasses import asdict, is_dataclass
from functools import wraps
import importlib
import inspect
from pathlib import Path
import re
import sys
import time
from weakref import WeakKeyDictionary

BATCH_THREE = (
    "plan_scope_side_effects", "plan_scope_ownership", "transaction_closures",
    "transaction_callback_wrappers", "transaction_patterns", "lazy_result_usage",
    "swallowed_exceptions", "error_handling", "module_level_caches", "connection_caches",
)
TAGGED = {"transaction_callback_wrappers", "swallowed_exceptions", "error_handling", "connection_caches"}


def relative(path, recorder, declared=None):
    path = Path(path)
    if declared is not None and not Path(declared).is_absolute():
        return Path(declared).as_posix()
    if not path.is_absolute():
        return path.as_posix()
    fixture = recorder.item.funcargs.get("tmp_path")
    if fixture is not None and path.is_relative_to(fixture):
        return path.relative_to(fixture).as_posix()
    for index, part in enumerate(path.parts):
        if part in ("api", "impl", "conform", "data_layer_testing") or re.match(r"^l\d{4}_", part):
            return Path(*path.parts[index:]).as_posix()
    return path.name


def category_config(check):
    if check != "transaction_patterns":
        return {}
    frame = inspect.currentframe()
    try:
        while frame is not None:
            value = frame.f_locals.get("categorized_functions")
            if isinstance(value, dict):
                return {"categories": {key: sorted(names) for key, names in value.items()}}
            frame = frame.f_back
    finally:
        del frame
    return {}


def findings(check, facts, path, recorder):
    output = []
    for item in facts:
        if not isinstance(item, dict):
            item = asdict(item) if is_dataclass(item) else vars(item)
        message = item.get("message", "")
        tag = None
        code = item.get("code")
        if check == "transaction_closures":
            code = re.search(r"TXN\d{3}", message).group()
        elif check == "transaction_callback_wrappers":
            code = "transaction_callback_wrapper"
            tag = "inline-reentrant" if "inline transaction" in message else "callback-helper"
        elif check == "transaction_patterns":
            code = item["category"]
        elif check == "lazy_result_usage":
            code = "lazy_result_usage"
        elif check == "swallowed_exceptions":
            code = {1: "INFO", 2: "WARNING", 3: "CRITICAL"}[item["severity"]]
            tag = ("suppress-legacy" if item.get("handler_summary") == "contextlib.suppress" else "suppress-critical") if "`contextlib.suppress(" in message else "except-" + code.lower()
        elif check == "error_handling":
            code = "EH001"
            tag = "log-reraise" if message.startswith("log-then-reraise") else "admin-commit"
        elif check == "module_level_caches":
            code = item["pattern_type"]
        elif check == "connection_caches":
            module = importlib.import_module("scripts.lint_connection_caches")
            message = item["description"]
            code = "CC001"
            tag = "pattern-" + str(next(i for i, (_, label) in enumerate(module.ALL_PATTERNS) if label == message))
        actual_path = item.get("file_path", item.get("path", item.get("file", path)))
        output.append({"path": relative(actual_path, recorder), "line": item.get("line", item.get("line_number")),
                       "column": item.get("col"), "code": code, "message": message, "tag": tag,
                       "severity": item.get("severity")})
    return output


def append(recorder, check, entry, files, facts, elapsed, config=None, errors=None):
    event = {"test": recorder.item.nodeid, "call": 1 + sum(e["test"] == recorder.item.nodeid for e in recorder.events),
             "check": check, "entry": entry, "files": files, "scope": {"include": ["**/*.py"], "exclude": []},
             "oracle_ms": elapsed, "compare_clause": check in TAGGED,
             "oracle_config": {"source_gate": entry == "FactEngine.materialize", **(config or {})}, "findings": []}
    if errors:
        event["retained_reason"] = "Original fact framework reports extraction errors: " + str(errors)
    elif not files:
        event["retained_reason"] = "No readable source files; retain discovery or I/O assertions upstream"
    else:
        for path, raw in facts:
            event["findings"].extend(findings(check, raw, path, recorder))
    recorder.events.append(event)


def install(recorder):
    modules = {name: importlib.import_module("scripts.lint_" + name) for name in BATCH_THREE}
    sources = WeakKeyDictionary()
    original_parse = ast.parse

    @wraps(original_parse)
    def parse(source, *args, **kwargs):
        tree = original_parse(source, *args, **kwargs)
        if recorder.item is not None:
            sources[tree] = source.decode() if isinstance(source, bytes) else source
        return tree

    recorder.originals.append((ast, "parse", original_parse))
    ast.parse = parse

    module = modules["connection_caches"]
    connection_scan = module.scan_file_content

    @wraps(connection_scan)
    def scan_connection(file_path, root, source):
        if recorder.item is None or recorder.depth:
            return connection_scan(file_path, root, source)
        started = time.perf_counter()
        result = connection_scan(file_path, root, source)
        raw = [{"file_path": path, "line": line, "description": label} for path, line, _, label in result]
        append(recorder, "connection_caches", "scan_file_content",
               [{"path": relative(file_path, recorder), "source": source}], [(file_path, raw)],
               (time.perf_counter() - started) * 1000)
        return result

    recorder.originals.append((module, "scan_file_content", connection_scan))
    module.scan_file_content = scan_connection

    def observe_engine(original):
        @wraps(original)
        def observed(engine, plugins, **kwargs):
            if recorder.item is None or recorder.depth:
                return original(engine, plugins, **kwargs)
            plugins = tuple(plugins)
            plan = engine.plan(plugins)
            checks = {spec.extractor_id.split(".")[0] for spec in plan.required_facts} & set(BATCH_THREE)
            if not checks:
                return original(engine, plugins, **kwargs)
            recorder.depth += 1
            started = time.perf_counter()
            try:
                result = original(engine, plugins, **kwargs)
            finally:
                recorder.depth -= 1
            for check in sorted(checks):
                files, raw, errors = [], [], []
                for path in plan.files:
                    ids = [s.extractor_id for s in plan.facts_by_file[path] if s.extractor_id.startswith(check + ".")]
                    if not ids:
                        continue
                    try:
                        files.append({"path": relative(path, recorder), "source": path.read_text()})
                    except (OSError, UnicodeError):
                        continue
                    bundle = result.files.get(path)
                    if bundle is not None:
                        raw.append((path, [item for key in ids for item in bundle.facts.get(key, [])]))
                        errors.extend(bundle.errors[key] for key in ids if key in bundle.errors)
                append(recorder, check, "FactEngine.materialize", files, raw,
                       (time.perf_counter() - started) * 1000,
                       {**category_config(check), "source_gate": any(s.source_triggers for s in plan.required_facts
                                                                  if s.extractor_id.startswith(check + "."))}, errors)
            return result
        return observed

    def observe_source(original):
        @wraps(original)
        def observed(**kwargs):
            checks = {s.extractor_id.split(".")[0] for s in kwargs["specs"]} & set(BATCH_THREE)
            if recorder.item is None or recorder.depth or not checks:
                return original(**kwargs)
            recorder.depth += 1
            started = time.perf_counter()
            try:
                result = original(**kwargs)
            finally:
                recorder.depth -= 1
            for check in sorted(checks):
                ids = [s.extractor_id for s in kwargs["specs"] if s.extractor_id.startswith(check + ".")]
                raw = [item for key in ids for item in result.facts.get(key, [])]
                declared = next((item.get("path", item.get("file_path")) for item in raw if isinstance(item, dict)), None)
                path = relative(kwargs["path"], recorder, declared)
                append(recorder, check, "materialize_source_facts", [{"path": path, "source": kwargs["source"]}],
                       [(path, raw)], (time.perf_counter() - started) * 1000, category_config(check), result.errors)
            return result
        return observed

    for name in ("scripts.lint_lib.fact_engine", "lint_lib.fact_engine"):
        module = sys.modules.get(name)
        if module is None:
            continue
        owner = module.FactEngine
        original = owner.materialize
        recorder.originals.append((owner, "materialize", original))
        owner.materialize = observe_engine(original)
        original = module.materialize_source_facts
        replacement = observe_source(original)
        recorder.originals.append((module, "materialize_source_facts", original))
        module.materialize_source_facts = replacement
        for target in modules.values():
            if getattr(target, "materialize_source_facts", None) is original:
                recorder.originals.append((target, "materialize_source_facts", original))
                target.materialize_source_facts = replacement

    def observe_visit(original, check, entry):
        @wraps(original)
        def observed(owner, tree):
            if recorder.item is None or recorder.depth:
                return original(owner, tree)
            recorder.depth += 1
            started = time.perf_counter()
            try:
                result = original(owner, tree)
            finally:
                recorder.depth -= 1
            path = relative(owner.file_path, recorder)
            source = sources.get(tree)
            config = {"categories": {key: sorted(names) for key, names in owner.categorized_functions.items()}} if check == "transaction_patterns" else {}
            raw = getattr(owner, "findings", getattr(owner, "violations", []))
            append(recorder, check, entry, [{"path": path, "source": source}] if source is not None else [],
                   [(path, raw)], (time.perf_counter() - started) * 1000, config)
            return result
        return observed

    for check, name in (("transaction_closures", "TransactionClosureLinter"),
                        ("transaction_closures", "ClosureDefinitionFinder"),
                        ("transaction_patterns", "ConnectionParameterFinder")):
        owner = getattr(modules[check], name)
        original = owner.visit
        recorder.originals.append((owner, "visit", original))
        owner.visit = observe_visit(original, check, name + ".visit")
