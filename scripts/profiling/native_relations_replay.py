#!/usr/bin/env python3
"""Compare native relation experiments against captured, complete linter rows.

This measures SQL replay including native loading, not parsing or extraction.
All decisions and seed selection remain in the captured SQL policy.
"""
import argparse
import hashlib
import json
import platform
from pathlib import Path
import resource
import sqlite3
import sys
import time

from native_sql_plan import expose, materialize as narrow, plan as demand_plan, quote
from reachability_replay import normalized, resolve, sqlite_row
from reachability_sql import connect, materialize, requested_roots


def infer_ir_names(db):
    """Propagate the actual SQL stage's IR-name seeds up reversed syntax edges."""
    started = time.process_time()
    db.execute("""CREATE TEMP TABLE inference_seeds AS
        SELECT __path,id,text AS name FROM py_names WHERE text LIKE '%IR'
        UNION SELECT __path,id,attr FROM attributes WHERE attr LIKE '%IR'""")
    db.execute("CREATE TEMP TABLE inference_labels AS SELECT name,row_number() OVER(ORDER BY name) AS label FROM (SELECT DISTINCT name FROM inference_seeds)")
    nodes = [r[0] for r in db.execute("SELECT compact FROM reach_keys")]
    edges = list(db.execute("SELECT dst,src FROM reach_edges"))
    seeds = list(db.execute("SELECT k.compact,l.label FROM inference_seeds s JOIN reach_keys k ON k.__path=s.__path AND k.key=s.id JOIN inference_labels l ON l.name=s.name"))
    prepared = time.process_time()
    arguments = ("inference", json.dumps(nodes), json.dumps(edges), json.dumps(seeds), "[]", 200_000_000, 90_000, True)
    serialized = time.process_time()
    db.execute("SELECT flow_load(?,?,?,?,?,?,?,?)", arguments)
    solved = time.process_time()
    db.execute("""CREATE TEMP TABLE ir_names AS SELECT DISTINCT k.__path,k.key AS root,l.name
        FROM inference f JOIN reach_keys k ON k.compact=f.node
        JOIN reach_requests r ON r.root=f.node JOIN inference_labels l ON l.label=f.label""")
    return {**json.loads(db.execute("SELECT flow_stats('inference')").fetchone()[0]),
            "inputNodes":len(nodes), "inputEdges":len(edges), "inputSeeds":len(seeds),
            "prepareCpuMs":(prepared-started)*1000, "serializeCpuMs":(serialized-prepared)*1000,
            "loadSolveCpuMs":(solved-serialized)*1000, "resultCpuMs":(time.process_time()-solved)*1000}


def execute(capture, case, backend, extension, only_path=None, verify_intermediates=False):
    if only_path is not None and only_path not in {f["path"] for f in case["files"]}:
        raise ValueError(f"Unknown selected path: {only_path}")
    findings, metrics = [], []
    for clause in capture["policy"]["clauses"]:
        if "relational" not in clause:
            continue
        policy = clause["relational"]
        schemas = case["schemas"][clause["id"]]
        inputs = case["rows"].get(clause["id"], {})
        stages = [{**s, "query": resolve(s["query"], case["options"])} for s in policy.get("stages", [])]
        final = resolve(policy["query"], case["options"])
        restricted = {**policy, "stages": stages}
        has_graph = any(s["name"] == "descendants" for s in stages)
        roots = requested_roots(restricted) if has_graph else None
        compiled = None
        planning = time.process_time()
        if backend in ("demand", "combined"):
            planner = connect(extension)
            # The graph adapter reads these columns outside the original stage.
            additional = {"descendants": [roots, "SELECT __path,id FROM nodes", "SELECT __path,src,dst FROM edges"]} if has_graph else {}
            compiled = demand_plan(planner, schemas, stages, final, additional)
            planner.close()
        metrics.append({"clause": clause["id"], "phase": "plan", "cpuMs": (time.process_time()-planning)*1000,
                        "columns": None if compiled is None else {k: compiled[k] for k in ("logicalColumns", "physicalColumns")}})
        paths = sorted({r[0] for rows in inputs.values() for r in rows}) if policy.get("partitionBy") == "file" else [None]
        for path in paths:
            if only_path and path and path != only_path:
                continue
            db = connect(extension)
            released = set()
            try:
                started = time.process_time()
                for name, schema in schemas.items():
                    columns = compiled["columns"][name] if compiled else schema
                    positions = [schema.index(c) for c in columns]
                    rows = [[row[i] for i in positions] for row in inputs.get(name, [])
                            if (path is None or row[0] == path) and (only_path is None or row[0] == only_path)]
                    physical = "__physical_"+name if compiled else name
                    if backend in ("rows", "combined"):
                        db.execute("SELECT native_rows_load(?,?,?,?)", (physical, json.dumps(columns), json.dumps(rows), "[0]"))
                    else:
                        db.execute(f"CREATE TABLE {quote(physical)} ({','.join(quote(c) for c in columns)})")
                        db.executemany(f"INSERT INTO {quote(physical)} VALUES ({','.join('?' for _ in columns)})", rows)
                    if compiled:
                        expose(db, name, physical, schema, columns)
                metrics.append({"clause": clause["id"], "path": path, "phase": "inputs", "cpuMs": (time.process_time()-started)*1000})
                special = set()
                for index, stage in enumerate(stages):
                    name = stage["name"]
                    started = time.process_time()
                    if name == "descendants":
                        graph = materialize(db, roots, "tree" if backend in ("tree", "combined") else "sql")
                        special.add(name)
                        metrics.append({"clause": clause["id"], "path": path, "phase": "graph", **graph})
                    elif name == "ir_names" and backend in ("flow", "combined"):
                        truth = normalized(list(db.execute(stage["query"]))) if verify_intermediates else None
                        flow = infer_ir_names(db)
                        if truth is not None and truth != normalized(list(db.execute("SELECT * FROM ir_names"))):
                            raise ValueError("Native inference differs from complete SQL ir_names relation")
                        special.add(name)
                        metrics.append({"clause": clause["id"], "path": path, "phase": "flow", **flow})
                    elif compiled:
                        narrow(db, name, stage["query"], compiled["schemas"][name], compiled["columns"][name], stage.get("indexes", []))
                    else:
                        db.execute(f"CREATE TEMP TABLE {quote(name)} AS {stage['query']}")
                    if not compiled or name == "ir_names" and name in special:
                        for i, fields in enumerate(stage.get("indexes", [])):
                            if name == "descendants":
                                break
                            db.execute(f"CREATE INDEX {quote(name+'_'+str(i))} ON {quote(name)} ({','.join(quote(f) for f in fields)})")
                        if stage.get("indexes") and name != "descendants":
                            db.execute("ANALYZE " + quote(name))
                    metrics.append({"clause": clause["id"], "path": path, "phase": name, "cpuMs": (time.process_time()-started)*1000})
                    if compiled:
                        # The special graph/flow backing tables remain alive for their views.
                        for relation, last in compiled["lastUse"].items():
                            if last == index and relation not in special and relation not in released:
                                db.execute("DROP VIEW " + quote(relation))
                                db.execute("DROP TABLE " + quote("__physical_"+relation))
                                released.add(relation)
                db.row_factory = sqlite_row
                findings.extend(db.execute(final).fetchall())
            except Exception as error:
                raise RuntimeError(f"{case['name']}/{clause['id']}/{path}/{backend}: {error}") from error
            finally:
                db.close()
    return findings, metrics


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--capture", required=True)
    p.add_argument("--extension", required=True)
    p.add_argument("--backend", choices=("sql", "rows", "demand", "tree", "flow", "combined"), required=True)
    p.add_argument("--offset", type=int, default=0)
    p.add_argument("--limit", type=int, default=20)
    p.add_argument("--path")
    p.add_argument("--verify-intermediates", action="store_true", help="Check complete inference rows; excludes this run from performance comparisons")
    p.add_argument("--output", required=True)
    args = p.parse_args()
    capture_path = Path(args.capture)
    started = time.process_time()
    capture = json.loads(capture_path.read_text())
    rows = []
    for case in capture["cases"][args.offset:args.offset+args.limit]:
        # Removing files changes project-wide absence checks. A subset therefore
        # uses the SQL control on that same subset, rather than filtered findings.
        truth = execute(capture, case, "sql", args.extension, args.path)[0] if args.path else case["expected"]
        before = time.process_time()
        actual, metrics = execute(capture, case, args.backend, args.extension, args.path, args.verify_intermediates)
        expected = normalized(truth)
        observed = normalized(actual)
        rows.append({"name":case["name"], "positive":bool(expected), "parity":expected==observed,
                     "expected":len(expected), "actual":len(observed), "missing": sorted(set(expected)-set(observed)),
                     "extra":sorted(set(observed)-set(expected)), "cpuMs":(time.process_time()-before)*1000, "metrics":metrics})
    report = {"format":"rerail.native-relations-replay/v1", "backend":args.backend,
              "captureSha256":hashlib.sha256(capture_path.read_bytes()).hexdigest(),
              "extensionSha256":hashlib.sha256(Path(args.extension).read_bytes()).hexdigest(),
              "commit":capture["commit"], "sqliteVersion":sqlite3.sqlite_version, "platform":platform.platform(),
              "verifiedIntermediates":args.verify_intermediates,
              "cpuMs":(time.process_time()-started)*1000,
              "peakRssBytes":resource.getrusage(resource.RUSAGE_SELF).ru_maxrss*(1 if sys.platform=="darwin" else 1024),
              "cases":len(rows), "positive":sum(r["positive"] for r in rows), "rows":rows,
              "parityReference":"SQL control on selected file" if args.path else "Complete captured native findings",
              "note":"Captured-input replay, including loading and planning. Not an end-to-end engine benchmark."}
    Path(args.output).write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps({k:v for k,v in report.items() if k!="rows"}))
    if any(not r["parity"] for r in rows):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
