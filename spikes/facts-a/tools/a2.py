#!/usr/bin/env python3
"""a2.py <run results.json>... : A2 numbers (i) and (ii) and the per-rule native vs facts table.

Native apportioning (batch 6, `~/rerail-bench/dl60-split/`, the 393 s baseline):
  global(rule)  = stage-build seconds (`b6-stage-builds.csv`, column total_s) plus re-extraction
                  seconds (`b6-extraction-per-input.csv`, column seconds) of the rule's clauses;
  file(rule)    = 122.8 s file-phase matching x (rule's AST matcher inputs / all 263), counts from
                  the census (`census.json`, units whose `unit` is `input`);
  native(rule)  = global(rule) + file(rule). What these do not cover (session setup, input
                  loads, producer, result, teardown, process overhead) stays unattributed and
                  counts in the baseline only.
  A sensitivity figure, global_full, adds every global-phase probe record of the clause
  (`b6.jsonl.gz`, summed as `tables.py` does).
quality-contracts-1 (A2 ii): global(clause) + 122.8 x 7/263, removed from the 393 s.

Facts total: process user+sys from `/usr/bin/time -l` in the run's stderr.txt. Facts per rule:
the run's per-rule clause nanoseconds (thread time, evaluation only; parse and extraction are
shared and reported separately).
"""
import collections, csv, gzip, json, os, re, sys

HOME = os.path.expanduser("~")
SPLIT = f"{HOME}/rerail-bench/dl60-split"
CENSUS = f"{HOME}/rerail-bench/facts/spike-a/work/census.json"
BASELINE, FILE_PHASE = 393.0, 122.8
QC1 = ("data-layer-serverplan-quality", "quality-contracts-1")
S = 1e9


def csv_global():
    g = collections.Counter()
    for r in csv.DictReader(open(f"{SPLIT}/b6-stage-builds.csv")):
        g[(r["rule"], r["clause"])] += float(r["total_s"])
    for r in csv.DictReader(open(f"{SPLIT}/b6-extraction-per-input.csv")):
        g[(r["rule"], r["clause"])] += float(r["seconds"])
    return g


def probe_global():
    recs = collections.defaultdict(list)
    with gzip.open(f"{SPLIT}/b6.jsonl.gz", "rt") as fh:
        for line in fh:
            d = json.loads(line)
            recs[d["k"]].append(d)
    pc = collections.Counter()
    for d in recs["in"]:
        pc[(d["r"], d["c"])] += d["ns"] / S
    for d in recs["job"]:
        pc[(d["r"], d["c"])] += (d["doc"] + d["syn"]) / S
    for d in recs["sn"]:
        pc[(d["r"], d["c"])] += d["total"] / S
    for d in recs["st"]:
        pc[(d["r"], d["c"])] += (d["cols"] + d["rel"] + d["build"]) / S
    for d in recs["dm"]:
        pc[(d["r"], d["c"])] += (d["cols"] + d["ns"]) / S
    for d in recs["pub"]:
        pc[(d["r"], d["c"])] += (d["ns"] + d["rel"]) / S
    for d in recs["px"]:
        for x in d["d"]:
            pc[(x[0], x[1])] += d["ns"] / S / len(d["d"])
    for d in recs["fin"]:
        pc[(d["r"], d["c"])] += d["ns"] / S
    for d in recs["drop"]:
        pc[(d["r"], d["c"])] += d["ns"] / S
    return pc


def by_rule(pc):
    out = collections.Counter()
    for (r, _), s in pc.items():
        out[r.removeprefix("data-layer-")] += s
    return out


def main(paths):
    g, full = csv_global(), probe_global()
    census = json.load(open(CENSUS))
    inputs = collections.Counter()
    qc1_inputs = 0
    for u in census["units"]:
        if u["unit"] == "input":
            inputs[u["rule"]] += 1
            if ("data-layer-" + u["rule"], u["clause"]) == QC1:
                qc1_inputs += 1
    total_inputs = sum(inputs.values())
    g_rule, full_rule = by_rule(g), by_rule(full)
    nat = {}
    for r in sorted(set(g_rule) | set(inputs) | set(full_rule)):
        f = FILE_PHASE * inputs[r] / total_inputs
        nat[r] = {"global": g_rule[r], "global_full": full_rule[r], "file": f, "inputs": inputs[r], "native": g_rule[r] + f}
    qc1 = {"global": g[QC1], "global_full": full[QC1], "inputs": qc1_inputs, "file": FILE_PHASE * qc1_inputs / total_inputs}
    qc1["native"] = qc1["global"] + qc1["file"]
    qc1["native_full"] = qc1["global_full"] + qc1["file"]
    runs = []
    for p in paths:
        res = json.load(open(p))
        err = open(os.path.join(os.path.dirname(p), "stderr.txt")).read()
        m = re.search(r"([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys", err)
        rss = int(re.search(r"(\d+)\s+maximum resident set size", err).group(1))
        real, user, sys_ = map(float, m.groups())
        per_rule = {r["check"].replace("_", "-"): r["ns"] / S for r in res["rules"]}
        qc1_ns = next(c["ns"] for r in res["rules"] for c in r["clauses"] if c["clause"] == "quality-contracts-1") / S
        runs.append({"run": os.path.dirname(p), "wall": real, "cpu": user + sys_, "user": user, "sys": sys_, "rss": rss, "per_rule": per_rule, "qc1": qc1_ns})
    out = {
        "native": {"baseline": BASELINE, "file_phase": FILE_PHASE, "global_csv_total": sum(g.values()), "global_full_total": sum(full.values()),
                   "matcher_inputs_total": total_inputs, "per_rule": nat, "qc1": qc1},
        "runs": runs,
    }
    if runs:
        med = sorted(runs, key=lambda r: r["cpu"])[len(runs) // 2]
        out["median_run"] = med["run"]
        out["a2_i"] = {"facts_cpu": med["cpu"], "native_cpu": BASELINE, "ratio": med["cpu"] / BASELINE}
        facts_ii = med["cpu"] - med["qc1"]
        out["a2_ii"] = {"facts_cpu": facts_ii, "facts_qc1_removed": med["qc1"], "native_cpu": BASELINE - qc1["native"],
                        "native_qc1_removed": qc1["native"], "ratio": facts_ii / (BASELINE - qc1["native"]),
                        "ratio_full": facts_ii / (BASELINE - qc1["native_full"])}
        out["spread"] = {"cpu": [r["cpu"] for r in runs], "wall": [r["wall"] for r in runs], "rss": [r["rss"] for r in runs]}
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main(sys.argv[1:])
