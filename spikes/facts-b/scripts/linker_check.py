#!/usr/bin/env python3
"""Runs the reference-graph linker stages (`spikes/facts-b/linker/reference_edges.sql`) over the
cgn export of a member set and compares `reference_edges` with native's dump.

Inputs the stages read (all values text, as the export spells them):
  member(path, hash)                     the workspace's files
  cgn_<rel>(hash, variant, stub, ...)    per-content `as_references` relations, every variant
The stages are run in order, each as `CREATE TABLE <name> AS <statement>`. A stage's
`-- index: a,b` lines only add SQLite indexes on its table (speed, not semantics).

    python3 spikes/facts-b/scripts/linker_check.py --label baseline
"""
import argparse
import datetime
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
from collections import Counter

BENCH = os.path.expanduser("~/rerail-bench/facts/phase-2")
HERE = os.path.dirname(os.path.abspath(__file__))
LINKS_KEY = "547566c618e1c9e3"
# The export's relations the stages read (as_references of the dead-code clause's links).
RELATIONS = {
    "cgn_module": ["name"],
    "cgn_def": ["idx", "name", "start", "end", "flags"],
    "cgn_edge": ["def", "name"],
    "cgn_alias": ["key", "target"],
    "cgn_reference_alias": ["key", "target"],
    "cgn_opaque": ["name"],
    "cgn_star": ["name"],
    "cgn_super": ["key", "target", "kind"],
    "cgn_open": ["open", "open_sites"],
}


def rust_to_python(pattern):
    """The stages' Rust regexes in Python's `re`: they use only the shared subset (classes,
    groups, anchors, `(?:…)`); `\\z` is Rust's end of text."""
    return pattern.replace("\\z", "\\Z")


_compiled = {}


def rx(pattern):
    compiled = _compiled.get(pattern)
    if compiled is None:
        compiled = _compiled[pattern] = re.compile(rust_to_python(pattern), re.S)
    return compiled


def rx_matches(text, pattern):
    if text is None or pattern is None:
        return None
    return 1 if rx(pattern).search(text) else 0


def rx_extract(text, pattern, group):
    if text is None or pattern is None:
        return None
    found = rx(pattern).search(text)
    return None if found is None else found.group(group)


def rx_replace(text, pattern, by):
    if text is None or pattern is None:
        return None
    # Rust's `Regex::replace_all` with `$1`-style groups.
    return rx(pattern).sub(re.sub(r"\$(\d+)", r"\\\1", by), text)


def stages(path):
    """`-- stage <name>` blocks, each one statement; `-- index: cols` lines are index hints."""
    out, current = [], None
    for line in open(path, encoding="utf-8"):
        found = re.match(r"--\s*stage\s+(\w+)\s*$", line)
        if found:
            current = {"name": found.group(1), "sql": [], "indexes": []}
            out.append(current)
            continue
        if current is None:
            continue
        hint = re.match(r"--\s*index:\s*(.+?)\s*$", line)
        if hint:
            current["indexes"].append([c.strip() for c in hint.group(1).split(",")])
        current["sql"].append(line)
    for stage in out:
        stage["sql"] = "".join(stage["sql"]).strip().rstrip(";")
    return out


def glob_re(glob):
    """globset with literal_separator (compile/mod.rs:251): `**/` any directories, `/**` anything
    below, `*` within one segment."""
    out = glob.replace(".", "\\.").replace("**/", "%DIRS%").replace("/**", "%BELOW%").replace("*", "[^/]*")
    return re.compile("^" + out.replace("%DIRS%", "(?:.*/)?").replace("%BELOW%", "/.*") + "$")


def in_scope(rule_path):
    scope = json.load(open(rule_path, encoding="utf-8"))["scope"]
    include = [glob_re(g) for g in scope.get("include", [])]
    exclude = [glob_re(g) for g in scope.get("exclude", [])]
    return lambda p: any(r.search(p) for r in include) and not any(r.search(p) for r in exclude)


def load_inputs(db, members_path, export_path, rule_path):
    """The stages' inputs as the per-file producer exports them: `member(__path)` for every
    content, `cgn_<rel>(__path, variant, ...)` for those the rule's scope admits, in the reading
    the path selects, and `rule_scope(kind, glob)` from the rule."""
    # `path \t hash [\t ...]`: the corpus jobs TSV (oracle-b6-jobs.tsv) or a recording's file list.
    admit = in_scope(rule_path)
    members = [line.rstrip("\n").split("\t")[:2] for line in open(members_path, encoding="utf-8") if line.strip()]
    db.execute("CREATE TABLE member(__path TEXT)")
    db.executemany("INSERT INTO member VALUES (?)", [(p,) for p, _ in members])
    scope = json.load(open(rule_path, encoding="utf-8"))["scope"]
    db.execute("CREATE TABLE rule_scope(kind TEXT, glob TEXT)")
    db.executemany("INSERT INTO rule_scope VALUES (?, ?)", [(k, g) for k in ("include", "exclude") for g in scope.get(k, [])])
    by_reading = {}
    for p, h in members:
        if admit(p):
            by_reading.setdefault((h, p.endswith(".pyi")), []).append(p)
    hashes = {h for h, _ in by_reading}
    for rel, cols in RELATIONS.items():
        quoted = ", ".join(f'"{c}"' for c in cols)
        db.execute(f"CREATE TABLE {rel}(__path TEXT, variant TEXT, {quoted})")
    rows = {rel: [] for rel in RELATIONS}
    seen = set()
    proc = subprocess.Popen(["zstd", "-dc", export_path], stdout=subprocess.PIPE)
    for raw in proc.stdout:
        at = raw.find(b'"hash":"')
        if at < 0 or raw[at + 8:at + 72].decode() not in hashes:
            continue
        line = json.loads(raw)
        paths = by_reading.get((line["hash"], bool(line["stub"])), [])
        if paths:
            seen.add(line["hash"])
        refs = line["as_references"][LINKS_KEY]
        for rel in RELATIONS:
            for row in refs.get(rel, []):
                for p in paths:
                    rows[rel].append((p, line["variant"], *row))
    if proc.wait() != 0:
        raise SystemExit("zstd failed")
    for rel, cols in RELATIONS.items():
        db.executemany(f"INSERT INTO {rel} VALUES ({', '.join('?' * (len(cols) + 2))})", rows[rel])
        db.execute(f"CREATE INDEX {rel}_pv ON {rel}(__path, variant)")
    db.commit()
    return {"members": len(members), "distinct_hashes": len(hashes), "hashes_exported": len(seen),
            "rows": {rel: len(r) for rel, r in rows.items()}}


def rerail_refuse(construct):
    raise ValueError(f"refused: {construct}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--label", default="run")
    parser.add_argument("--members", default=f"{BENCH}/s3-crossfile/exports/oracle-b6-jobs.tsv",
                        help="path<TAB>hash[<TAB>...] per worktree file (default: the 2b1504adc corpus)")
    parser.add_argument("--native", default=f"{BENCH}/s3-crossfile/linker/native-ground-truth/reference_edges.tsv")
    parser.add_argument("--export", default=f"{BENCH}/s4-schema/cgn-export.jsonl.zst")
    parser.add_argument("--rule", default=os.path.join(HERE, "../../../packs/data-layer/candidates/rules/data-layer-internal-dead-code.json"),
                        help="the rule whose scope selects the members")
    parser.add_argument("--sql", nargs="+", default=[os.path.join(HERE, "../../../facts/src/front", f) for f in ("module_path.sql", "reference_edges.sql")],
                        help="view files, run in order (front/rule.rs compiles them the same way)")
    parser.add_argument("--out-root", default=f"{BENCH}/s3-crossfile/linker")
    parser.add_argument("--inputs-db", help="cache of the loaded inputs (created when missing)")
    parser.add_argument("--keep-db", action="store_true", help="keep the run's SQLite database in the run dir")
    args = parser.parse_args()

    stamp = datetime.datetime.now().strftime("%Y%m%dT%H%M%S")
    run_dir = os.path.join(args.out_root, f"{stamp}-{args.label}")
    os.makedirs(run_dir, exist_ok=True)
    db_path = os.path.join(run_dir, "linker.sqlite") if args.keep_db else ":memory:"
    db = sqlite3.connect(db_path)
    db.execute("PRAGMA temp_store = MEMORY")
    db.execute("PRAGMA cache_size = -4000000")
    db.create_function("rx_matches", 2, rx_matches, deterministic=True)
    db.create_function("rx_extract", 3, rx_extract, deterministic=True)
    db.create_function("rx_replace", 3, rx_replace, deterministic=True)
    db.create_function("rerail_refuse", 1, rerail_refuse, deterministic=True)

    started = time.time()
    if args.inputs_db and os.path.exists(args.inputs_db):
        source = sqlite3.connect(args.inputs_db)
        source.backup(db)
        source.close()
        loaded = json.loads(db.execute("SELECT value FROM linker_inputs_meta").fetchone()[0])
    else:
        loaded = load_inputs(db, args.members, args.export, args.rule)
        db.execute("CREATE TABLE linker_inputs_meta(value TEXT)")
        db.execute("INSERT INTO linker_inputs_meta VALUES (?)", (json.dumps(loaded),))
        db.commit()
        if args.inputs_db:
            target = sqlite3.connect(args.inputs_db)
            db.backup(target)
            target.close()
    load_s = time.time() - started
    print(json.dumps({"inputs": loaded, "load_s": round(load_s, 1)}), flush=True)

    timings = []
    for stage in [s for path in args.sql for s in stages(path)]:
        t = time.time()
        db.execute(f"DROP TABLE IF EXISTS {stage['name']}")
        db.execute(f"CREATE TABLE {stage['name']} AS {stage['sql']}")
        for i, cols in enumerate(stage["indexes"]):
            db.execute(f"CREATE INDEX {stage['name']}_i{i} ON {stage['name']}({', '.join(cols)})")
        db.commit()
        n = db.execute(f"SELECT count(*) FROM {stage['name']}").fetchone()[0]
        timings.append({"stage": stage["name"], "rows": n, "s": round(time.time() - t, 2)})
        print(f"  {stage['name']:<28} {n:>10} rows {time.time() - t:8.2f}s", flush=True)

    native = set()
    for line in open(args.native, encoding="utf-8"):
        path, start, target_path, target_start = line.rstrip("\n").split("\t")[:4]
        native.add((path, int(start), target_path, int(target_start)))
    sql = {(p, int(s), tp, int(ts)) for p, s, tp, ts in db.execute(
        "SELECT __path, __start, target_path, target_start FROM reference_edges")}
    incomplete = db.execute("SELECT count(*) FROM reference_graph_incomplete").fetchone()[0]
    refusals = [r for r in db.execute("SELECT path, construct FROM linker_refusal")]
    by_construct = Counter(c for _, c in refusals)
    refused_paths = {p for p, _ in refusals}
    only_native, only_sql = sorted(native - sql), sorted(sql - native)

    def attributed(edge):
        return edge[0] in refused_paths or edge[2] in refused_paths

    unattributed = [e for e in only_native + only_sql if not attributed(e)]
    names = {}
    for path, start, name in db.execute("SELECT path, start, name FROM s_def"):
        names[(path, int(start))] = name

    def show(edge):
        return f"{edge[0]}#{edge[1]} {names.get((edge[0], edge[1]), '?')} -> {edge[2]}#{edge[3]} {names.get((edge[2], edge[3]), '?')}"

    result = {
        "label": args.label, "run_dir": run_dir, "members": args.members, "native_dump": args.native,
        "export": args.export, "sql": [os.path.abspath(p) for p in args.sql], "inputs": loaded, "load_s": round(load_s, 1),
        "stages": timings, "native_edges": len(native), "sql_edges": len(sql),
        "only_native": len(only_native), "only_sql": len(only_sql),
        "differences": len(only_native) + len(only_sql),
        "unattributed_differences": len(unattributed),
        "reference_graph_incomplete": incomplete,
        "refusals_by_construct": {c: {"rows": n, "paths": len({p for p, k in refusals if k == c})}
                                  for c, n in sorted(by_construct.items())},
        "first_only_native": [show(e) for e in only_native[:30]],
        "first_only_sql": [show(e) for e in only_sql[:30]],
    }
    with open(os.path.join(run_dir, "results.json"), "w") as f:
        json.dump(result, f, indent=1)
    with open(os.path.join(run_dir, "differences.tsv"), "w") as f:
        for side, edges in (("native", only_native), ("sql", only_sql)):
            for e in edges:
                f.write(f"only_{side}\t{e[0]}\t{e[1]}\t{e[2]}\t{e[3]}\t{'refused' if attributed(e) else ''}\n")
    print(f"native edges N={len(native)}  sql edges M={len(sql)}  differences D={len(only_native) + len(only_sql)}"
          f" (only native {len(only_native)}, only sql {len(only_sql)}, unattributed {len(unattributed)})")
    print(f"reference_graph_incomplete rows: {incomplete}")
    print("refusals by construct:", json.dumps(result["refusals_by_construct"]))
    for title, edges in (("only native", only_native), ("only sql", only_sql)):
        for e in edges[:30]:
            print(f"  {title}: {show(e)}")
    print(f"results: {run_dir}/results.json")
    return 0 if not unattributed else 1


if __name__ == "__main__":
    sys.exit(main())
