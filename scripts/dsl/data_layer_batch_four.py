#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Observe the fourth batch's public analyzer boundaries without changing tests."""
from __future__ import annotations

from dataclasses import asdict, is_dataclass
from functools import wraps
import importlib
from pathlib import Path
import time

BATCH_FOUR = (
    "render_fstrings", "render_raw_strings", "regex_sql_parsing", "sql_grammar_escape_hatches",
    "grammar_render_methods", "grammar_str_params", "grammar_callable_constructors",
    "grammar_fragment_usage", "grammar_allowlist_burndown", "rust_sql_grammar_boundary",
)


def normalize(check, raw, root, files):
    if hasattr(raw, "violations"):
        raw = raw.violations
    output = []
    for item in raw:
        value = asdict(item) if is_dataclass(item) else item
        path = Path(value.get("file_path", value.get("file", value.get("path", files[0]["path"]))))
        if path.is_absolute():
            path = path.relative_to(root) if path.is_relative_to(root) else Path(path.name)
        code = value.get("code", value.get("violation_type"))
        if check == "render_fstrings": code = "render_fstring"
        if check == "regex_sql_parsing": code = value["category"]
        if check == "sql_grammar_escape_hatches": code = "escape_hatch"
        if check == "grammar_fragment_usage": code = "python_parse_error" if value.get("class_name")=="<parse>" else "sqlfragment_usage"
        if check == "rust_sql_grammar_boundary": code = "rust_sql_grammar_boundary"
        tag = None
        if check == "rust_sql_grammar_boundary":
            module = importlib.import_module("scripts.lint_rust_sql_grammar_boundary")
            tag = "pattern-" + str(next(i for i, (pattern, _) in enumerate(module.FORBIDDEN_PATTERNS) if pattern.pattern == value["rule"]))
        output.append({"tag": tag, "path": path.as_posix(), "line": value.get("line", value.get("line_number")),
                       "column": None, "code": code,
                       "message": value.get("message", value.get("reason", value.get("detail", value.get("preview", ""))))})
    return output


def configuration(module, check, kwargs):
    names = {
        "grammar_render_methods": ("ALLOWLISTED_STATIC_CLASSES", "LEGACY_ALLOWLISTED_CLASSES"),
        "grammar_str_params": ("ALLOWLISTED_STR_CLASSES",),
        "sql_grammar_escape_hatches": ("ALLOWLISTED_CLASSES", "DATA_VALUE_PARAMS", "OPERATOR_PARAMS", "SCHEMA_METADATA_CLASSES", "CALLSITE_PROTECTED_CLASSES"),
    }.get(check, ())
    options = {name: sorted(getattr(module, name)) for name in names}
    for key in ("grammar_class_names", "safe_param_types"):
        if kwargs.get(key) is not None: options[key] = sorted(kwargs[key])
    return {"options": options}


def install(recorder):
    modules = {name: importlib.import_module("scripts.lint_" + name) for name in BATCH_FOUR}
    adapters = (
        ("render_fstrings", "analyze_file", "file"), ("render_fstrings", "analyze_source", "source"),
        ("render_raw_strings", "analyze_file", "file"), ("render_raw_strings", "analyze_source", "source"),
        ("sql_grammar_escape_hatches", "scan_file", "file"), ("sql_grammar_escape_hatches", "scan_source", "source"),
        ("sql_grammar_escape_hatches", "scan_usages", "file"), ("sql_grammar_escape_hatches", "scan_usages_source", "source"),
        ("grammar_render_methods", "scan_file", "file"), ("grammar_render_methods", "scan_source", "source"),
        ("grammar_str_params", "lint_file", "file"), ("grammar_str_params", "lint_source", "source"),
        ("grammar_callable_constructors", "collect_violations", "root"),
        ("grammar_fragment_usage", "check_file", "file"),
        ("rust_sql_grammar_boundary", "check_file", "file"),
    )
    def wrap(original, module, check, entry, kind):
        @wraps(original)
        def observed(*args, **kwargs):
            if recorder.item is None or recorder.depth: return original(*args, **kwargs)
            path = Path(args[0])
            root = path.resolve() if kind == "root" else path.resolve().parent
            files = []
            if kind == "root":
                paths = module._iter_files(path)
            else: paths = [path]
            for file in paths:
                try:
                    source = args[1] if kind == "source" else file.read_text()
                    files.append({"path": file.resolve().relative_to(root).as_posix(), "source": source})
                except (OSError, UnicodeError): pass
            event = {"test": recorder.item.nodeid, "call": 1 + sum(e["test"] == recorder.item.nodeid for e in recorder.events),
                     "check": check, "entry": entry, "files": files, "scope": {"include": ["**/*"], "exclude": []},
                     "oracle_config": configuration(module, check, kwargs), "compare_clause": check == "rust_sql_grammar_boundary"}
            recorder.depth += 1
            started = time.perf_counter()
            try: result = original(*args, **kwargs)
            finally: recorder.depth -= 1
            event["oracle_ms"] = (time.perf_counter() - started) * 1000
            if not files: event["retained_reason"] = "No readable files; retain discovery and I/O assertions upstream"
            else: event["findings"] = normalize(check, result, root, files)
            recorder.events.append(event)
            return result
        return observed
    for check, entry, kind in adapters:
        module = modules[check]; original = getattr(module, entry)
        recorder.originals.append((module, entry, original))
        setattr(module, entry, wrap(original, module, check, entry, kind))

    cls = modules["regex_sql_parsing"].RegexSqlParsingChecker
    original = cls.check_file
    @wraps(original)
    def regex(owner, tree, path, source):
        if recorder.item is None or recorder.depth: return original(owner, tree, path, source)
        recorder.depth += 1
        started = time.perf_counter()
        try: result = original(owner, tree, path, source)
        finally: recorder.depth -= 1
        files = [{"path": Path(path).name, "source": source}]
        recorder.events.append({"test": recorder.item.nodeid, "call": 1 + sum(e["test"] == recorder.item.nodeid for e in recorder.events),
            "check": "regex_sql_parsing", "entry": "RegexSqlParsingChecker.check_file", "files": files,
            "scope": {"include": ["**/*.py"], "exclude": []}, "oracle_config": {"options": {"sourceGate": False}},
            "oracle_ms": (time.perf_counter() - started) * 1000,
            "findings": normalize("regex_sql_parsing", result, Path(path).parent, files)})
        return result
    recorder.originals.append((cls, "check_file", original)); cls.check_file = regex

    # Classifier tests supply literal pattern/method pairs. Turn those inputs into
    # source calls mechanically; the original classifier supplies the expected code.
    module = modules['regex_sql_parsing']
    classifier = module._classify_pattern
    @wraps(classifier)
    def classify(pattern, method):
        if recorder.item is None or recorder.depth:
            return classifier(pattern, method)
        started = time.perf_counter()
        result = classifier(pattern, method)
        source = 'import re\nvalue = re.' + method + '(' + repr(pattern) + ')\n'
        event = {'test':recorder.item.nodeid,'call':1+sum(e['test']==recorder.item.nodeid for e in recorder.events),
                 'check':'regex_sql_parsing','entry':'_classify_pattern source translation','input_mode':'mechanically generated literal call',
                 'files':[{'path':'input.py','source':source}],'scope':{'include':['**/*.py'],'exclude':[]},
                 'oracle_config':{'options':{'sourceGate':False}},'oracle_ms':(time.perf_counter()-started)*1000,
                 'findings':[] if result is None else [{'path':'input.py','line':2,'column':None,'code':result,'message':'Original pattern classifier result'}]}
        if method not in ('search','sub','match','findall','fullmatch','split','compile'):
            event['retained_reason']='Classifier helper accepts a method outside the actual analyzer call surface'
        recorder.events.append(event)
        return result
    recorder.originals.append((module,'_classify_pattern',classifier));module._classify_pattern=classify
