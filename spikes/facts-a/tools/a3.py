#!/usr/bin/env python3
"""a3.py <run results.json>... : Spike A3 breakdown per run, and the remaining-SQL sizing.

Per run (thread seconds unless noted):
  cpu, wall        process user+sys and real time, `/usr/bin/time -l` in the run's stderr.txt
  extraction       read + parse + extract (per-file.tsv sums, `thread_ns`)
  ast_inputs       AST evidence inputs still evaluated by the matcher (`input_ns`)
  views            substituted stages: every `view:*` stage, `view:dump` (the one-pass node
                   dumps) included
  sql              every other SQL stage and final query
  spill_load       repository partitions: reading the spill files back (`spill:load`)
  rss              peak (`maximum resident set size`), and current RSS at the end of the
                   per-file phase (`cur_rss_bytes.after_workers`) and at its peak
                   (`max_rss_bytes.after_workers`)
Runs without per-clause stage times (before A3) are split by stage name: the tree stages
(`nodes`, `edges`, `ownership`, `descendants`, `node_ordinals`, `syntax_rows_*`) in `tree_sql`.

`--sql <policies dir>` adds, for the median run, the remaining SQL per clause: its top stages
and whether each stage's text is shared with another clause, byte-identical (after whitespace
normalisation) or identical once the clause's input and stage names are replaced by
placeholders.
"""
import collections, json, os, re, sys

S = 1e9
TREE = re.compile(r"^(nodes|edges|ownership|descendants|node_ordinals|syntax_rows_\d+)$")
RULES = ["serverplan-bypass", "serverplan-steps", "serverplan-state-tripwires", "serverplan-dispatch-architecture", "serverplan-quality",
         "serverplan-target-architecture", "execution-authority-bypass", "impl-direct-execution", "raw-execute-regression", "legacy-execution-surfaces"]


def run_summary(p):
    res = json.load(open(p))
    d = os.path.dirname(p)
    err = open(os.path.join(d, "stderr.txt")).read()
    real, user, sys_ = map(float, re.search(r"([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys", err).groups())
    rss = int(re.search(r"(\d+)\s+maximum resident set size", err).group(1))
    t = res["thread_ns"]
    out = {"run": d, "cpu": user + sys_, "wall": real, "rss_peak": rss, "rss_file_phase_peak": res["max_rss_bytes"]["after_workers"],
           "rss_file_phase_end": res.get("cur_rss_bytes", {}).get("after_workers"), "rss_after_finish_cur": res.get("cur_rss_bytes", {}).get("after_finish"),
           "spill_bytes": res.get("spill_bytes"), "finish_s": res["phases_ns"]["finish"] / S,
           "extraction": (t["read"] + t["parse"] + t["extract"]) / S, "eval": t["eval"] / S, "ast_inputs": t["input_rows"] / S,
           "a3": res["a3_clause_file_ns"], "per_rule": {r["check"]: r["ns"] / S for r in res["rules"]}}
    views = sql = spill = tree = 0.0
    if "clause_stage_ns" in res:
        for x in res["clause_stage_ns"]:
            s = x["ns"] / S
            if x["stage"].startswith("view:"):
                views += s
            elif x["stage"] == "spill:load":
                spill += s
            else:
                sql += s
                if TREE.match(x["stage"]):
                    tree += s
    else:
        for k, (n, ns) in res["sql_stage_ns"].items():
            sql += ns / S
            if TREE.match(k):
                tree += ns / S
    out.update(views=views, sql=sql, tree_sql=tree, spill_load=spill)
    return out, res


def norm(q):
    return " ".join(q.split())


def sql_sizing(res, policies):
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from resolve import resolve
    texts = {}  # (check, clause, stage) -> (exact, templated)
    for rule in RULES:
        doc = resolve(json.load(open(os.path.join(policies, f"data-layer-{rule}.json"))))
        check = rule.replace("-", "_")
        for c in doc["clauses"]:
            rel = c.get("relational")
            if not rel:
                continue
            names = list(rel["inputs"].keys()) + [s["name"] for s in rel.get("stages", [])]
            pat = re.compile(r"\b(" + "|".join(re.escape(n) for n in sorted(names, key=len, reverse=True)) + r")\b") if names else None
            stmts = [(s["name"], s.get("query") or json.dumps(s.get("reachability"), sort_keys=True)) for s in rel.get("stages", [])] + [("<final>", rel["query"])]
            for name, q in stmts:
                exact = norm(q)
                templ = pat.sub(lambda m: "<" + ("in" if m.group(1) in rel["inputs"] else "st") + ">", exact) if pat else exact
                texts[(check, c["id"], name)] = (exact, templ)
    by_exact, by_templ = collections.defaultdict(set), collections.defaultdict(set)
    for (ch, cl, st), (e, t) in texts.items():
        by_exact[e].add((ch, cl))
        by_templ[t].add((ch, cl))
    per_clause = collections.defaultdict(list)
    for x in res["clause_stage_ns"]:
        if x["stage"].startswith("view:") or x["stage"] == "spill:load":
            continue
        key = (x["check"], x["clause"], x["stage"])
        e, t = texts.get(key, (None, None))
        shared_exact = sorted(f"{a}/{b}" for a, b in by_exact.get(e, ()) if (a, b) != key[:2]) if e else []
        shared_templ = sorted(f"{a}/{b}" for a, b in by_templ.get(t, ()) if (a, b) != key[:2]) if t else []
        per_clause[key[:2]].append({"stage": x["stage"], "s": x["ns"] / S, "shared_exact": shared_exact, "shared_templated": [s for s in shared_templ if s not in shared_exact]})
    out = []
    for k, v in per_clause.items():
        v.sort(key=lambda r: -r["s"])
        out.append({"check": k[0], "clause": k[1], "sql_s": sum(r["s"] for r in v), "top": v[:5]})
    out.sort(key=lambda r: -r["sql_s"])
    # Totals: SQL seconds in stages whose text another clause shares.
    tot = sum(r["s"] for k, v in per_clause.items() for r in v)
    ex = sum(r["s"] for k, v in per_clause.items() for r in v if r["shared_exact"])
    tm = sum(r["s"] for k, v in per_clause.items() for r in v if r["shared_exact"] or r["shared_templated"])
    return {"sql_total_s": tot, "shared_exact_s": ex, "shared_exact_or_templated_s": tm, "clauses": out}


def main(argv):
    policies = None
    if "--sql" in argv:
        i = argv.index("--sql")
        policies = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    runs = [run_summary(p) for p in argv]
    out = {"runs": [r for r, _ in runs]}
    med, med_res = sorted(runs, key=lambda r: r[0]["cpu"])[len(runs) // 2]
    out["median_run"] = med["run"]
    out["spread"] = {k: [r[k] for r, _ in runs] for k in ("cpu", "wall", "rss_peak", "rss_file_phase_end")}
    if policies and "clause_stage_ns" in med_res:
        out["remaining_sql"] = sql_sizing(med_res, policies)
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main(sys.argv[1:])
