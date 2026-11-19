"""Performance model for the data-layer rule corpus, calibrated on the fusion-model runs.

Level 1 (mechanistic): per-file evaluation time = sum over executed-operation counters of a unit
cost, fitted by non-negative least squares on sample A (all modes) and tested on sample B and on
the full corpus. It answers "do the counters explain the time?".

Level 2 (planner, no execution of the target corpus): per-query time = candidates x kappa_q.
  - feedback:  kappa_q measured on sample A (plan_feedback), candidates from the target's postings.
  - cold:      kappa_q predicted from rule structure alone (static features from analyze.py),
               coefficients fitted on A, tested on B.
Fusion (memo/mask) is modelled as a measured ratio on A, validated on B and full.

Then stage-by-stage predictions for the full gate and a per-edit check, CPU / Metal / SQL, with
ranges from the measured unit costs in docs/rule-engine-direction/research/{fusion,metal-recursion}.md.

Usage: python3 -I model.py OUT_DIR   (reads run_A.json, run_B.json, run_full.json, queries.json)
"""
import json
import math
import os
import statistics as st
import sys

OUT = sys.argv[1]


def load(n):
    p = os.path.join(OUT, n)
    return json.load(open(p)) if os.path.exists(p) else None


A, B, F = load("run_A.json"), load("run_B.json"), load("run_full.json")
QSTATIC = [q for q in load("queries.json") if q["active"] and q["language"] == "python"]
NAMES = A["counter_names"]
FEATS = ["kind", "all", "any", "not", "regex", "regex_bytes", "pattern", "filepath", "inside", "has_nb", "has_end",
         "has_rule", "sibling", "util", "nth", "scan_visit", "memo_hit", "mask_lookup", "mask_nodes", "const", "cand_scan", "dispatch", "memo_alloc_bytes"]
IDX = [NAMES.index(f) if f in NAMES else None for f in FEATS]


def nnls(X, y, iters=4000):
    """Non-negative least squares by projected coordinate descent on the normal equations."""
    k = len(X[0])
    G = [[sum(r[i] * r[j] for r in X) for j in range(k)] for i in range(k)]
    b = [sum(r[i] * yy for r, yy in zip(X, y)) for i in range(k)]
    w = [0.0] * k
    for _ in range(iters):
        delta = 0.0
        for i in range(k):
            if G[i][i] <= 0:
                continue
            g = b[i] - sum(G[i][j] * w[j] for j in range(k) if j != i)
            nw = max(0.0, g / G[i][i])
            delta = max(delta, abs(nw - w[i]))
            w[i] = nw
        if delta < 1e-15:
            break
    return w


def rows(run, modes):
    X, y = [], []
    for pf in run["per_file"]:
        for m in modes:
            if m in pf:
                c = pf[m + "_c"]
                X.append([1.0] + [float(c[i]) if i is not None and i < len(c) else 0.0 for i in IDX])
                y.append(pf[m] / 1e3)
    return X, y


def pred(w, X):
    return [sum(a * b for a, b in zip(w, r)) for r in X]


def err_report(name, run, w, modes):
    out = {}
    for m in modes:
        X, y = rows(run, [m])
        if not X:
            continue
        p = pred(w, X)
        tot_err = (sum(p) - sum(y)) / sum(y)
        rel = [abs(a - b) / b for a, b in zip(p, y) if b > 2e-3]
        out[m] = {"measured_s": sum(y), "predicted_s": sum(p), "total_err": tot_err,
                  "file_median_abs_err": st.median(rel) if rel else None,
                  "file_p90_abs_err": sorted(rel)[int(0.9 * (len(rel) - 1))] if rel else None, "files>2ms": len(rel)}
    return out


report = {}
modes_fit = ["sep", "memo", "mask", "maskall"]
X, y = rows(A, modes_fit)
# scale columns for conditioning
scale = [max(1.0, max(abs(r[i]) for r in X)) for i in range(len(X[0]))]
Xs = [[r[i] / scale[i] for i in range(len(r))] for r in X]
ws = nnls(Xs, y)
w = [ws[i] / scale[i] for i in range(len(ws))]
unit = {"per_file": w[0]}
unit.update({f: w[i + 1] for i, f in enumerate(FEATS)})
report["level1_unit_costs_ns"] = {k: v * 1e9 for k, v in unit.items() if v > 0}
report["level1_fit_A"] = err_report("A", A, w, modes_fit + ["fused"])
if B:
    report["level1_test_B"] = err_report("B", B, w, modes_fit + ["fused"])
if F:
    report["level1_test_full"] = err_report("full", F, w, ["sep", "memo", "mask"])

# ---------- level 2: per-query kappa ----------
def per_query(run):
    return {(q["rule"], q["clause"], i): q for i, q in enumerate(run["queries"])}


def kappa(q):
    return q["sep_seconds"] / q["candidates"] if q["candidates"] else 0.0


def static_feats(s):
    sites = s["sites"]
    f = {
        "c": 1.0,
        "kind": s["atoms"].get("kind", 0),
        "regex": s["atoms"].get("regex", 0),
        "pattern": s["atoms"].get("pattern", 0),
        "has_nb": sum(1 for t in sites if t[0] == "has" and t[1] == "neighbor"),
        "has_scan": sum(1 for t in sites if t[0] == "has" and t[1] != "neighbor"),
        "inside_nb": sum(1 for t in sites if t[0] == "inside" and t[1] == "neighbor"),
        "inside_scan": sum(1 for t in sites if t[0] == "inside" and t[1] != "neighbor"),
        "sibling": sum(1 for t in sites if t[0] in ("precedes", "follows")),
        "bool": sum(s["bools"].values()),
        "recursive": len(s["recursive_utils"]),
        "file_root": 1.0 if (s["root_kinds"] or []) == ["module"] else 0.0,
    }
    return f


SF = ["c", "kind", "regex", "pattern", "has_nb", "has_scan", "inside_nb", "inside_scan", "sibling", "bool", "recursive", "file_root"]


def level2(train, test, name):
    tq, sq = train["queries"], test["queries"]
    assert len(tq) == len(sq) == len(QSTATIC), (len(tq), len(sq), len(QSTATIC))
    # feedback: kappa from train, candidates from test
    pred_fb = [kappa(a) * b["candidates"] for a, b in zip(tq, sq)]
    meas = [b["sep_seconds"] for b in sq]
    # cold: kappa from static features; fit on train queries (log-free NNLS on kappa, weighted by candidates)
    Xr, yr = [], []
    for a, s in zip(tq, QSTATIC):
        if a["candidates"] == 0:
            continue
        f = static_feats(s)
        wgt = math.sqrt(a["candidates"])
        Xr.append([f[k] * wgt for k in SF])
        yr.append(kappa(a) * wgt)
    sc = [max(1.0, max(abs(r[i]) for r in Xr)) for i in range(len(SF))]
    wc = nnls([[r[i] / sc[i] for i in range(len(SF))] for r in Xr], yr)
    wc = [wc[i] / sc[i] for i in range(len(SF))]
    pred_cold = []
    for b, s in zip(sq, QSTATIC):
        f = static_feats(s)
        pred_cold.append(sum(wc[i] * f[k] for i, k in enumerate(SF)) * b["candidates"])

    def stats(p):
        tot = (sum(p) - sum(meas)) / sum(meas)
        le = [abs(math.log(max(x, 1e-7) / y)) for x, y in zip(p, meas) if y > 1e-3]
        return {"predicted_s": sum(p), "measured_s": sum(meas), "total_err": tot,
                "query_median_abs_log_err": st.median(le) if le else None,
                "query_median_factor": math.exp(st.median(le)) if le else None, "queries>1ms": len(le)}

    return {"feedback": stats(pred_fb), "cold_static": stats(pred_cold),
            "cold_coeffs_ns_per_candidate": {k: wc[i] * 1e9 for i, k in enumerate(SF) if wc[i] > 0}}


if B:
    report["level2_A_to_B"] = level2(A, B, "B")
if F:
    report["level2_A_to_full"] = level2(A, F, "full")

# ---------- fusion ratios ----------
def secs(run, m):
    for r in run["runs"]:
        if r["mode"] == m:
            return r["seconds"]


fr = {}
for name, run in (("A", A), ("B", B), ("full", F)):
    if not run:
        continue
    base = secs(run, "sep")
    fr[name] = {m: base / secs(run, m) for m in ("fused", "memo", "mask", "maskall") if secs(run, m)}
    fr[name]["nodes"] = run["corpus"]["nodes"]
report["fusion_speedup_vs_sep"] = fr

# ---------- stage model for the full gate and per-edit ----------
R = F or A
nodes = R["corpus"]["nodes"]
files = R["corpus"]["files"]
scale_to_full = 1.0 if F else 9010 / A["corpus"]["files"]
nodes_full = nodes * scale_to_full
bytes_full = R["corpus"]["bytes"] * scale_to_full


def rng(lo, hi):
    return [lo, hi]


stage = {}
# 1. substrate: parse (cold) or tape restore (warm, v7 3.7-7.8 ns/node)
parse_cpu = R["corpus"]["parse_build_cpu_s"] * scale_to_full
stage["parse_cold_cpu_s"] = rng(parse_cpu * 0.9, parse_cpu * 1.2)
stage["tape_restore_warm_cpu_s"] = rng(nodes_full * 3.7e-9, nodes_full * 7.8e-9)
# 2. index / candidates: kind postings 12.4 ns/node (or FTS read)
stage["postings_cpu_s"] = rng(nodes_full * 8e-9, nodes_full * 16e-9)
# 3. instruction programs (structural eval), measured, single thread, per mode
for m in ("sep", "memo", "mask"):
    s = secs(R, m)
    if s is not None:
        s *= scale_to_full
        stage[f"structural_{m}_cpu_s"] = rng(s * 0.75, s * 1.35)  # observed run-to-run spread on a shared machine
# 4. file-content regex clauses
tr = R["text_regex"]["seconds"] * scale_to_full
stage["text_regex_cpu_s"] = rng(tr * 0.8, tr * 1.5)
# 5. evidence rows into SQL (relational clauses) and consolidation
ev_rows = sum(q["matches"] for q in R["queries"] if q["role"] == "relational") * scale_to_full
findings = sum(q["matches"] for q in R["queries"] if q["role"] != "relational") * scale_to_full
stage["relational_evidence_rows"] = ev_rows
stage["sql_relational_s"] = rng(ev_rows * 1e-6, ev_rows * 5e-6)  # SQLite insert + join 1-5 us/row
stage["consolidation_s"] = rng(findings * 0.5e-6, findings * 3e-6)
report["stages_full_gate"] = stage

memo_steps = None
for r in R["runs"]:
    if r["mode"] == "memo":
        memo_steps = r["steps"] + r["counters"]["memo_hit"]
memo_steps_full = memo_steps * scale_to_full if memo_steps else None

# Metal: stratified range execution. One set operator per distinct DAG op that is evaluated densely,
# one command buffer; set-op cost from measured posting/range kernels (0.3-12 ns per element),
# element count = memo-mode (op, node) evaluations (each set op materialises its demanded set once).
dag = R["dag"]
metal = {
    "dispatches_est": [dag["max_stratum"] * 3, dag["max_stratum"] * 3 + 2 * 34],
    "set_elements": memo_steps_full,
    "kernel_s": rng(memo_steps_full * 0.3e-9, memo_steps_full * 12e-9) if memo_steps_full else None,
    "launch_s": rng(dag["ops"] * 3e-6, dag["ops"] * 10e-6),
    "cpu_only_regex_s": stage["text_regex_cpu_s"],
}
report["metal_structural"] = metal
# SQL lowering (pure SQL per op): rows = memo (op,node) evaluations at 100-500 ns/row in SQLite
report["sql_structural_s"] = rng(memo_steps_full * 100e-9, memo_steps_full * 500e-9) if memo_steps_full else None

# per-edit: one file, all rules
pf = R["per_file"]


def q(vals, p):
    v = sorted(vals)
    return v[int(p * (len(v) - 1))]


pe = {}
for m in ("sep", "memo", "mask"):
    if m in pf[0]:
        t = [x[m] + x["parse_ms"] for x in pf]
        pe[m] = {"p50_ms": q(t, 0.5), "p90_ms": q(t, 0.9), "p99_ms": q(t, 0.99), "max_ms": max(t)}
pe["parse_ms"] = {"p50": q([x["parse_ms"] for x in pf], 0.5), "p99": q([x["parse_ms"] for x in pf], 0.99)}
pe["plan_compile_cold_s"] = dag["compile_s"]
report["per_edit"] = pe


def total(lo_hi_list):
    return [sum(x[0] for x in lo_hi_list), sum(x[1] for x in lo_hi_list)]


cpu_memo = total([stage["tape_restore_warm_cpu_s"], stage["postings_cpu_s"], stage.get("structural_mask_cpu_s", stage.get("structural_memo_cpu_s")),
                  stage["text_regex_cpu_s"], stage["sql_relational_s"], stage["consolidation_s"]])
cpu_sep = total([stage["tape_restore_warm_cpu_s"], stage["postings_cpu_s"], stage["structural_sep_cpu_s"],
                 stage["text_regex_cpu_s"], stage["sql_relational_s"], stage["consolidation_s"]])
report["gate_totals"] = {
    "cpu_sep_1core_s": cpu_sep, "cpu_fused_1core_s": cpu_memo,
    "cpu_fused_4w_s": [cpu_memo[0] / 4, cpu_memo[1] / 3.2], "cpu_fused_16w_s": [cpu_memo[0] / 16, cpu_memo[1] / 10],
    "cold_parse_adds_4w_s": [stage["parse_cold_cpu_s"][0] / 4, stage["parse_cold_cpu_s"][1] / 3.2],
    "metal_s": total([metal["kernel_s"], metal["launch_s"], [x / 10 for x in stage["text_regex_cpu_s"]], stage["sql_relational_s"], stage["consolidation_s"]]) if metal["kernel_s"] else None,
    "sql_lowered_s": total([report["sql_structural_s"], stage["text_regex_cpu_s"], stage["sql_relational_s"], stage["consolidation_s"]]) if report["sql_structural_s"] else None,
    # projection: same fused evaluation count on a packed/JIT executor at 3-5 ns per (op,node)
    # evaluation (Tempo/N1562 packed loop class) instead of this harness's ~17 ns enum interpreter
    "cpu_packed_fused_16w_s": [(memo_steps_full * 3e-9 + stage["text_regex_cpu_s"][0] + stage["tape_restore_warm_cpu_s"][0] + stage["postings_cpu_s"][0]) / 16,
                               (memo_steps_full * 5e-9 + stage["text_regex_cpu_s"][1] + stage["tape_restore_warm_cpu_s"][1] + stage["postings_cpu_s"][1]) / 10],
    "harness_ns_per_eval_memo": secs(R, "memo") * scale_to_full / memo_steps_full * 1e9,
    "reference": {"DLRE_s": 958.6, "native_baseline_s": 125.8, "target_s": [1, 2]},
}
json.dump(report, open(os.path.join(OUT, "model.json"), "w"), indent=1)
print(json.dumps(report, indent=1))
