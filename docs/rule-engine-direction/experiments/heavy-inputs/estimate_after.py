"""Before/after estimates for the heaviest relational clauses.

Before: fusion-model `sep` seconds and exported rows/cells of the clause's AST inputs (inputs.json),
        plus the derived `descendants` closure DLRE builds over exported nodes (rows x mean depth 7.98).
After:  sum over the rewritten programs' root sets of (root count x kappa), with kappa = measured
        per-candidate cost of ordinary forbid queries with that root kind in the same harness
        (p50..p90 from run_full.json), plus set-stage passes at 6.4-17.3 ns/node (fusion.md masks).
        Root counts are the rule scope's kind counts (rule-profiles files.tsv) - an upper bound,
        since filePath/doc gates shrink them further.

Usage: python3 -I estimate_after.py INPUTS_JSON RUN_FULL_JSON
"""
import json, sys, statistics as st, collections

inputs = json.load(open(sys.argv[1]))
run = json.load(open(sys.argv[2]))
kap = collections.defaultdict(list)
for q in run["queries"]:
    if q["role"] != "relational" and q["candidates"]:
        for k in (q["root_kinds"] or []):
            kap[k].append(q["sep_seconds"] / q["candidates"])
K = {k: (st.median(v), sorted(v)[int(0.9 * len(v))]) for k, v in kap.items() if len(v) >= 2}
K["cheap_test"] = K["identifier"]           # kind + one interned-name / key-set test
PASS = (6.4e-9, 17.3e-9)                     # set-stage pass per node (fusion.md mask pass)

def before(pred):
    rs = [r for r in inputs if pred(r)]
    return sum(r["sep_seconds"] for r in rs), sum(r["matches"] for r in rs), sum(r["matches"] * len(r["project_cols"] or []) for r in rs)

# (label, input predicate, descendants-closure flag, [(root kind or 'pass', count, repeat)], rows after, note)
G = [
 ("quality-conditional-1", lambda r: r["clause"] == "quality-conditional-1", True,
  [("class_definition", 77, 1), ("function_definition", 411, 1), ("module", 4, 1),   # 4 anchor files
   ("cheap_test", 32728, 1),                                                          # classes vs derived-name key set
   ("call", 1843 + 846 + 74 + 158, 1)],
  "3 derived names + findings; shared fact_class_base 22,061"),
 ("step-contracts-1..6", lambda r: r["clause"].startswith("step-contracts"), True,
  [("class_definition", 2386, 1), ("call", 94314, 1), ("function_definition", 15960, 1), ("pass", 2343210, 3)],
  "0 exports; t_match = findings"),
 ("legacy-execution-graphs", lambda r: r["clause"] == "legacy-execution-graphs", False,
  [("call", 163216, 1), ("attribute", 19426, 1), ("function_definition", 24969, 1), ("pass", 4311171, 2)],
  "0 exports; t_match = findings"),
 ("28 dedupe-only state clauses", lambda r: r["rule"].endswith("state-tripwires") and r["input"] == "events", False,
  [("module", 3095, 1), ("call", 412415, 1)],
  "0 exports (findings only)"),
 ("state-flow-1 / -project-1 / projection-recovery-2 string feeders",
  lambda r: r["input"] and r["input"].startswith("raw_strings") and r["rule"].endswith("state-tripwires"), False,
  [("string", 274145, 1)],
  "0 string rows (key tests inline)"),
 ("executor-provenance", lambda r: r["clause"] == "executor-provenance", False,
  [("call", 120563, 1), ("function_definition", 19680, 1), ("pass", 3046108, 3)],
  "0 exports"),
]
tb = tl = th = 0
print(f"{'clause group':62s} {'before s':>8s} {'rows':>10s} {'cells':>12s} {'desc.closure':>12s} | {'after s (p50..p90)':>20s}  rows after")
for label, pred, desc, roots, after_rows in G:
    s, rows, cells = before(pred)
    lo = hi = 0.0
    for kind, n, rep in roots:
        a, b = PASS if kind == "pass" else K[kind]
        lo += a * n * rep; hi += b * n * rep
    tb += s; tl += lo; th += hi
    print(f"{label:62s} {s:8.2f} {rows:>10,} {cells:>12,} {(f'~{int(rows*7.98):,}' if desc else '-'):>12s} | {lo:8.3f} .. {hi:8.3f}  {after_rows}")
alls = sum(r["sep_seconds"] for r in inputs)
print(f"\nheavy groups before {tb:.1f}s of {alls:.1f}s (all 191 relational inputs) of {sum(q['sep_seconds'] for q in run['queries']):.1f}s (all 535 queries, sep)")
print(f"heavy groups after  {tl:.2f} .. {th:.2f} s  (single core, same harness unit costs)")
print("kappa (s/candidate) p50,p90:", {k: (f"{v[0]*1e9:.0f}ns", f"{v[1]*1e9:.0f}ns") for k, v in K.items() if k in ("call","class_definition","function_definition","module","string","attribute","identifier")})
