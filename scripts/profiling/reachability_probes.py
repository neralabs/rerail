#!/usr/bin/env python3
"""Compare batched pair probes against shared SQL closure and the Rust scalar."""
import argparse
import hashlib
import json
from pathlib import Path
import random
import time

from reachability_replay import resolve
from reachability_sql import connect, materialize, quote, requested_roots


def prefix(capture, case, path, extension, work=200_000_000):
    clause = next(clause for clause in capture["policy"]["clauses"]
                  if any(s["name"] == "descendants" for s in clause.get("relational", {}).get("stages", [])))
    plan = clause["relational"]
    db = connect(extension, seconds=105, work=work)
    for name, columns in case["schemas"][clause["id"]].items():
        db.execute(f"CREATE TABLE {quote(name)} ({','.join(quote(column) for column in columns)})")
        db.executemany(f"INSERT INTO {quote(name)} VALUES ({','.join('?' for _ in columns)})",
                       [row for row in case["rows"].get(clause["id"], {}).get(name, []) if row[0] == path])
    for stage in plan["stages"]:
        if stage["name"] == "descendants":
            break
        db.execute(f"CREATE TEMP TABLE {quote(stage['name'])} AS " + resolve(stage["query"], case["options"]))
        for i, columns in enumerate(stage.get("indexes", [])):
            db.execute(f"CREATE INDEX {quote(stage['name']+'_'+str(i))} ON {quote(stage['name'])} ({','.join(quote(column) for column in columns)})")
        if stage.get("indexes"):
            db.execute("ANALYZE " + quote(stage["name"]))
    return db, plan


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True)
    parser.add_argument("--extension", required=True)
    parser.add_argument("--path", required=True)
    parser.add_argument("--pairs", type=int, default=20_000)
    parser.add_argument("--roots", type=int, default=128)
    parser.add_argument("--root-kind", action="append", help="Restrict the authored request set to these node kinds")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    capture = json.loads(Path(args.capture).read_text())
    case = capture["cases"][0]
    db, plan = prefix(capture, case, args.path, args.extension)
    randomizer = random.Random(739)
    if args.pairs < 1 or args.roots < 1: raise ValueError("pairs and roots must be positive")
    roots = requested_roots(plan)
    if args.root_kind:
        kinds = ",".join("'" + kind.replace("'", "''") + "'" for kind in args.root_kind)
        roots = "SELECT DISTINCT r.__path,r.id FROM (" + roots + ") r JOIN nodes n ON n.__path=r.__path AND n.id=r.id WHERE n.kind IN (" + kinds + ")"
    roots = "SELECT * FROM (" + roots + ") ORDER BY id LIMIT " + str(args.roots)
    # SQL creates and retains its materialized closure for all subsequent probes.
    sql = materialize(db, roots, "sql", batch_size=256)
    requests = [row[0] for row in db.execute("SELECT root FROM reach_requests")]
    fetch_started = time.process_time()
    nodes = [row[0] for row in db.execute("SELECT compact FROM reach_keys")]
    edges = list(db.execute("SELECT src,dst FROM reach_edges"))
    fetch_cpu_ms = (time.process_time()-fetch_started)*1000
    probes = []
    all_positives = db.execute("SELECT root,node FROM reach_pairs ORDER BY root,node").fetchall()
    positives = randomizer.sample(all_positives, min(len(all_positives),max(1,args.pairs//2)))
    probes.extend(positives)
    while len(probes) < args.pairs:
        probes.append((randomizer.choice(requests), randomizer.choice(nodes)))
    randomizer.shuffle(probes)
    db.execute("CREATE TABLE probes(id INTEGER PRIMARY KEY,root INTEGER,node INTEGER)")
    db.executemany("INSERT INTO probes VALUES (?,?,?)", [(i,*pair) for i,pair in enumerate(probes)])
    started = time.process_time()
    encoded_nodes, encoded_edges = json.dumps(nodes), json.dumps(edges)
    serialized = time.process_time()
    handle = db.execute("SELECT reach_load(?,?,?,?)", (encoded_nodes,encoded_edges,200_000_000,90_000)).fetchone()[0]
    loaded = time.process_time()
    measurements = []
    truth = None
    for iteration in range(3):
        for backend in ["sql", "scalar"]:
            before = time.process_time()
            query = ("SELECT id,EXISTS(SELECT 1 FROM reach_pairs r WHERE r.root=p.root AND r.node=p.node) FROM probes p"
                     if backend == "sql" else f"SELECT id,is_reachable({handle},root,node) FROM probes")
            observed = db.execute(query).fetchall()
            elapsed = (time.process_time()-before)*1000
            if truth is None: truth = observed
            if observed != truth: raise AssertionError("point-probe parity failed")
            measurements.append({"iteration":iteration, "backend":backend, "cpuMs":elapsed})
    report = {"format":"rerail.reachability-probes/v1", "path":args.path, "pairs":len(probes),
              "roots":len(requests), "rootKinds":args.root_kind, "positive":sum(row[1] for row in truth), "sqlBuild":sql,
              "rustFetchCpuMs":fetch_cpu_ms,
              "rustSerializeCpuMs":(serialized-started)*1000, "rustLoadCpuMs":(loaded-serialized)*1000,
              "measurements":measurements, "rust":json.loads(db.execute("SELECT reach_stats(?)", (handle,)).fetchone()[0]),
              "captureSha256":hashlib.sha256(Path(args.capture).read_bytes()).hexdigest(),
              "extensionSha256":hashlib.sha256(Path(args.extension).read_bytes()).hexdigest(),
              "note":"Same pair table and immutable graph. SQL retains its materialized closure; Rust builds a bounded per-root memo on demand. Not full-rule timing."}
    Path(args.output).write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    db.close()


if __name__ == "__main__":
    main()
