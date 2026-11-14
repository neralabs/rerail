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
import resource
import hashlib
from dataclasses import replace

from data_layer_record import BATCH_ONE, blob_id
from data_layer_batch_two import BATCH_TWO
from data_layer_batch_three import BATCH_THREE
from data_layer_batch_four import BATCH_FOUR
from data_layer_batch_five import BATCH_FIVE, MODULES as BATCH_FIVE_MODULES
from data_layer_batch_six import BATCH_SIX


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--revision", default="2b1504adc4291daaf080b2e932962b134da1a69a")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--batches", default="1", help="Comma-separated batch numbers, currently 1 through 6.")
    parser.add_argument("--checks", help="Optional comma-separated subset of the selected batches")
    parser.add_argument("--selection", type=Path, help="Pinned profiling file/check selection; preserve project-fact metadata sentinels")
    parser.add_argument("--cpu-profile", type=Path, help="Optional cProfile output for oracle execution; adds measurement overhead")
    args = parser.parse_args()
    root = args.source.resolve()
    sys.path.insert(0, str(root))
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    from scripts.lint_lib.serverplan_facts import serverplan_fact_extractors

    plugins, extractors = [], serverplan_fact_extractors(root)
    batch_ids = [int(value) for value in args.batches.split(",")]
    if any(value not in (1, 2, 3, 4, 5, 6) for value in batch_ids) or len(set(batch_ids)) != len(batch_ids):
        parser.error("--batches requires distinct batch numbers from 1,2,3,4,5,6")
    batches = {1: BATCH_ONE, 2: BATCH_TWO, 3: BATCH_THREE, 4: BATCH_FOUR, 5:BATCH_FIVE, 6:BATCH_SIX}
    names = [name for value in batch_ids for name in batches[value]]
    if args.checks:
        selected_checks = args.checks.split(',')
        if len(set(selected_checks)) != len(selected_checks) or set(selected_checks)-set(names):parser.error('--checks must select distinct checks from --batches')
        names = [name for name in names if name in selected_checks]
    graph_checks={'public_api_triples','internal_dead_code','imperative_private_functions'}
    graph_module=None
    for name in names:
        if name in graph_checks:
            if graph_module is None:
                graph_module=importlib.import_module('scripts.lint_symbol_graph_plugins')
                extractors.update(graph_module.symbol_graph_fact_extractors(root))
            plugins.append(next(plugin for plugin in graph_module.symbol_graph_linter_plugins(root) if plugin.name==name))
            continue
        module = importlib.import_module(BATCH_FIVE_MODULES.get(name,"scripts.lint_" + name))
        if name == "rust_sql_grammar_boundary":
            continue
        plugin_name={"serverplan_target_architecture":"target_architecture","serverplan_dispatch_architecture":"dispatch_architecture"}.get(name,name)
        factory = getattr(module, plugin_name + "_plugin")
        plugins.append(factory() if name == "serverplan_grammar_surface" else factory(root))
        extractor_name = {"transaction_patterns": "transaction_pattern", "transaction_callback_wrappers": "transaction_callback_wrapper",
                          "transaction_closures": "transaction_closure","serverplan_target_architecture":"target_architecture","serverplan_dispatch_architecture":"dispatch"}.get(name, name)
        factory = getattr(module, extractor_name + "_fact_extractors", None)
        if factory is not None:
            extractors.update(factory(root))
    engine = FactEngine(root=root, fact_store=JsonFactStore(Path(tempfile.mkdtemp(prefix="rerail-dl-facts-"))),
                        extractors=extractors, enable_fact_cache=False)
    selection = json.loads(args.selection.read_text()) if args.selection else None
    metadata_files = set()
    if selection is not None:
        if any(name not in BATCH_SIX for name in names):
            parser.error('Restricted profiling plans currently support batch-six plugins only; other project extractors can read outside the plan')
        if selection['commit'] != args.revision or set(selection['checks']) != set(names):
            parser.error('Selection commit/checks must match the oracle invocation')
        selected_paths = {(root / item['path']).resolve() for item in selection['files']}
        if not selected_paths or len(selected_paths) != len(selection['files']) or any(not path.is_relative_to(root) for path in selected_paths):
            parser.error('Selection requires distinct files within the source root')
        for item in selection['files']:
            if hashlib.sha256((root / item['path']).read_bytes()).hexdigest() != item['sha256']:
                raise ValueError('Selection source hash mismatch: ' + item['path'])
        original_plan = engine.plan
        def selected_plan(selected_plugins):
            plan = original_plan(selected_plugins)
            sentinels = {path for path, specs in plan.facts_by_file.items() if any(spec.project_input_facts for spec in specs)}
            metadata_files.update(sentinels - selected_paths)
            paths = tuple(path for path in plan.files if path in selected_paths or path in sentinels)
            return replace(plan, files=paths, facts_by_file={path: plan.facts_by_file[path] for path in paths})
        engine.plan = selected_plan
    plans = {plugin.name: engine.plan([plugin]) for plugin in plugins}
    missing = {spec.extractor_id for plan in plans.values() for spec in plan.required_facts
               if spec.extractor_id.split(".")[0] in (*BATCH_THREE, *BATCH_FOUR) and spec.extractor_id not in extractors}
    if missing:
        raise ValueError(f"Missing oracle extractors: {sorted(missing)}")
    selected = sorted({path for plan in plans.values() for path in plan.files})
    if graph_module is not None:
        from scripts.lint_lib.discovery import discover_python_files, data_layer_production_dirs
        selected=sorted(set(selected)|set(discover_python_files(root,directories=['api',*data_layer_production_dirs(root)],include_tests=False)))
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
    profiler = None
    if args.cpu_profile:
        import cProfile
        profiler = cProfile.Profile()
        profiler.enable()
    started = time.perf_counter()
    cpu_started = time.process_time()
    children_started = resource.getrusage(resource.RUSAGE_CHILDREN)
    results = engine.run_plugins(plugins, max_workers=args.workers)
    oracle_ms = (time.perf_counter() - started) * 1000
    children_finished = resource.getrusage(resource.RUSAGE_CHILDREN)
    oracle_cpu_ms = (time.process_time() - cpu_started + children_finished.ru_utime + children_finished.ru_stime
                     - children_started.ru_utime - children_started.ru_stime) * 1000
    if profiler:
        profiler.disable()
        profiler.dump_stats(str(args.cpu_profile))
    events = []
    reporter_results = []
    for result in results:
        if selection is not None and result.name == 'serverplan_state_tripwires':
            # Generated-file freshness is an upstream integration contract, as documented in
            # docs/data-layer-batch-six.md. A restricted view necessarily changes that digest.
            # Keep the reporter observation and compare its unchanged source/allowlist path.
            module = importlib.import_module('scripts.lint_serverplan_state_tripwires')
            reporter_results.append({'check': result.name, 'contract': 'generated-artifact freshness and aggregate reporters remain upstream integration contracts',
                'findings': [{'path': v.path.as_posix(), 'line': v.line, 'code': v.code, 'message': v.message} for v in result.violations]})
            spec = module._serverplan_state_tripwire_spec(root)
            violations = [violation for bundle in captured[0].files.values()
                          for raw in bundle.facts.get(spec.extractor_id, [])
                          if (violation := module._violation_from_fact(raw)) is not None]
            violations = module._filter_signed_python_work_allowlist(violations)
            current, _stale = module._evaluate_against_allowlist(violations, module._load_allowlist())
            events.append({'test': 'suite:' + result.name, 'call': 1, 'check': result.name,
                           'entry': 'source-facts-and-upstream-signed-allowlists', 'input_mode': 'suite',
                           'findings': [{'path': v.file.relative_to(root).as_posix() if v.file.is_absolute() else v.file.as_posix(),
                                         'line': v.line, 'code': v.code, 'message': v.message} for v in current]})
            continue
        if result.name == 'public_api_triples':
            reporter_results.append({'check':result.name, 'contract':'signed total baseline reporter; retained upstream',
                'findings':[{'path':v.path.as_posix(),'line':v.line,'code':v.code,'message':v.message} for v in result.violations]})
            raw_items=[]
            for bundle in captured[0].files.values():
                survey=bundle.facts.get(graph_module.SYMBOL_GRAPH_SURVEY_EXTRACTOR.extractor_id)
                if isinstance(survey,dict):raw_items=survey.get('public_api_triples',[]);break
            events.append({'test':'suite:'+result.name,'call':1,'check':result.name,'entry':'graph-analyzer',
                'upstream_passed':True,'input_mode':'suite','production_scope':True,
                'scope':{'include':['**/*.py'],'exclude':[]},'file_set':'suite',
                'findings':[{'path':v['path'],'line':v['line'],'column':None,'code':v['issue'],'message':v['detail']} for v in raw_items]})
            continue
        if result.name in ("swallowed_exceptions", "async_sync_delegation"):
            reporter_results.append({"check": result.name, "contract": "signed aggregate baseline reporter; retained upstream",
                                     "findings": [{"path": v.path.as_posix(), "line": v.line, "code": v.code, "message": v.message} for v in result.violations]})
            view = captured[0]
            findings = []
            for path, bundle in view.files.items():
                for fact in bundle.facts.get(result.name+".violations", []):
                    if result.name=='async_sync_delegation':
                        findings.append({'path':fact['file'],'line':fact['line'],'column':None,'code':fact['category'],'message':fact['description']})
                    else:findings.append({"path": str(fact["path"]), "line": fact["line"], "column": fact.get("col"),
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
              "oracle_ms": oracle_ms, "oracle_cpu_ms": oracle_cpu_ms, "oracle_profile": engine.last_profile_metrics,
              "oracle_total_cpu_ms": (time.process_time() - cpu_started + resource.getrusage(resource.RUSAGE_CHILDREN).ru_utime
                                      + resource.getrusage(resource.RUSAGE_CHILDREN).ru_stime - children_started.ru_utime - children_started.ru_stime) * 1000,
              "peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if sys.platform == 'darwin' else 1024),
              "profiled": profiler is not None, "selection": selection,
              "metadata_files": sorted(path.relative_to(root).as_posix() for path in metadata_files),
              "measurement_note": "Oracle CPU covers run_plugins including waited-for child workers. Peak RSS covers this process only. Restricted plans retain project-fact metadata sentinels; process-tree sampling supplies concurrent worker RSS.",
              "files": files, "file_count": len(files), "bytes": sum(len(file["source"].encode()) for file in files), "workers": args.workers}
    args.output.write_text(json.dumps(output) + "\n")
    print(json.dumps({key: value for key, value in output.items() if key not in ("events", "oracle_profile", "files")}, indent=2))


if __name__ == "__main__":
    main()
