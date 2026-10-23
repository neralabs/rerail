#!/usr/bin/env python3
"""The facts engine's gate (Phase 2 deliverable 5, G2.6); run it through `facts-gate.sh` / `just facts-gate`.

Checks (clean build, `FACTS_MUTANT` unset), in seven concurrent lanes:

  a  a-replay         facts-a replay of batch 6's recordings (1,259 calls): 0 FP, every call replayed, every
                      miss or failure accepted residue (RESIDUE, empty now).
     a-repo           facts-a repo over the 9,010-file corpus + repo-compare against the suite oracle.
     a-subset         facts-a repo over the subset (the rewritten per-file path).
     a3-check         FACTS_A3_CHECK=1 over the subset: every rewritten relation equals the unrewritten one,
                      row for row (a difference panics).
     a-reference      FACTS_A3_OFF=1 over the subset (the unrewritten plan); findings and failures equal
                      a-subset's as multisets.
  b  b-exactness      facts-b replay: Spike B's hand-written delta rules (the generator's test vectors)
                      against their from-scratch oracle over the first EDITS_PREFIX (300) events of the
                      1,111-event edit script; `--full` replays all of it.
  g  b-generated      facts-b greplay: the delta-plan generator over IR plans against
                      rerail_facts::reference, same events.
  t  b-ties           facts-b greplay --plans rt --limit 200: S3's constructed-tie vector; every tie under a
                      `Total::Checked` order is refused at run time (expected_refusals met), 0 differences.
  c  rerail-facts-lib cargo test -p rerail-facts --lib (rewrite differential tests, classifier, audit,
                      reference evaluator).
     classifier-leak  cargo test -p rerail-facts leak: a per-file plan reading cross-file state is refused.
     planner-audit    cargo test -p rerail-facts phys::tests::audit: an inner scan is refused.
     explain-stable   facts-a explain over the pack: byte-identical across two runs and per-rule processes.
     harness-replay   Jeremy's replay.ts --engine facts: same acceptance as a-replay.
  v  views-derived    S4's facts-a derived-check --shard 0/8 (VIEWS_SHARD): schema views equal the policies'
                      stage texts over every 8th corpus file in sorted path order (1,127 of 9,010).
  h  harness-b1..b5   Jeremy's replay.ts --engine facts over batch n's recordings and probes (BATCH_RECORDINGS).
     suite-b1..b5     suite.ts --engine facts --cold-only over batch n's repository oracle (SUITES/suite-b<n>.json).
                      Per check: exact, differs (FP/FN) or refused (any failure: compile error, unsupported
                      construct, panic, Rust source). Passes when every check in GATED is exact; the rest
                      is measured and reported, so the summary line is the per-batch parity table.
     explain-physical facts-a explain --physical --root <subset> --audit, twice: byte-identical runs, every
                      subset file printed, each clause's plan block one text in every file (audit lines
                      informational).

The subset is deterministic: every corpus file whose sha256(relative path)[0] % 8 == 0, plus every file
holding an oracle finding (1,150 files of 9,010). It is hard-linked under SUBSET.

Mutants (`--mutants`): MUTANTS (1-3 S2's interim A3 views, 4-6 S3's delta generator, 7 the classifier,
8 the plan audit, 9 S3's runtime tie check, 10 the order-totality proof) plus one per rewrite registered
in docs/facts/REWRITES.md; each runs the checks of its family with `FACTS_MUTANT=<n>` and is killed when
one of them detects it (a3-check: the FACTS_A3_CHECK panic; a-reference: findings differ from the clean
reference; b-*: an exactness difference or a missed expected refusal; cargo checks: a failing test,
named in failed-tests.txt). A registered rewrite without a mutant fails the gate.

Output (stdout), one line per event, every line prefixed `facts-gate: `:

  facts-gate: run <run dir> commit <sha>[+dirty] load <1-min load>
  facts-gate: build <ok|FAIL> <s>s <package>[mutants] ...
  facts-gate: check <id> <pass|FAIL> <s>s <summary>
  facts-gate: mutant <n> <name> <killed|SURVIVED> <s>s <check>=<killed|survived|error> ...
  facts-gate: <passed|FAILED> <s>s: <p>/<c> checks pass, <k>/<m> mutants killed[; failing: <ids>]

and <run dir>/gate.json (format rerail.facts-gate/v1) with the same rows, each check's and mutant run's
directory (command.txt, stdout.txt, stderr.txt with /usr/bin/time -l, uptime before and after).
Exit 0: passed; 1: a check failed or a mutant survived; 2: usage.
"""
from __future__ import annotations

import argparse
import collections
import datetime
import hashlib
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HOME = Path.home()
BENCH = HOME / "rerail-bench"
CORPUS = BENCH / "nera-data-layer-2b1504ad"
ORACLE = BENCH / "facts/spike-a/work/suite-b6.json"
EVENTS = BENCH / "facts/spike-b/edits/seed-5eedb/events.jsonl"
SUBSET = BENCH / "facts/phase-2/s5-gate/subset-8"
RUNS = BENCH / "facts/phase-2/s5-gate/runs"
RECORDINGS = ["fixtures/host/data-layer-port/batch-06-recording.json", "fixtures/host/data-layer-port/batch-06-probes.json"]
BATCH6 = ["serverplan_bypass", "serverplan_steps", "serverplan_state_tripwires", "serverplan_dispatch_architecture",
          "serverplan_quality", "serverplan_target_architecture", "execution_authority_bypass",
          "impl_direct_execution", "raw_execute_regression", "legacy_execution_surfaces"]
# Accepted residue, (check, clause, message part): failures the gate tolerates. Empty since S4's
# `string_type_definition` (275b016d1) made quality-contracts-1 exact; nothing else is accepted.
RESIDUE: list[tuple[str, str, str]] = []
THREADS = "4"

# Batches 1-5 through Jeremy's harness with `--engine facts`: each batch's recordings for replay.ts (as
# /tmp/dl60/replay-all.sh combines them) and its repository oracle for suite.ts (recorded by
# data_layer_suite_record.py at 2b1504adc, copied from /tmp/dl60 with SHA256SUMS beside them).
PORT = "fixtures/host/data-layer-port"
BATCH_RECORDINGS = {
    1: ["batch-01-recording.json"],
    2: ["batch-02-recording.json", "batch-02-probes.json"],
    3: ["batch-03-recording.json", "batch-03-probes.json"],
    4: ["batch-04-recording.json", "batch-04-probes.json"],
    5: ["batch-05-recording.json", "batch-05-probes.json", "batch-05-graph-probes.json",
        "batch-05-remaining-graph-probes.json", "batch-05-relational-probes.json"],
}
SUITES = BENCH / "facts/phase-2/s5-gate/suites"
# The batch 1-5 checks the gate holds exact through the harness (replay: 0 FP, 0 FN, 0 failures, at least
# one positive and one clean call; suite: 0 FP, 0 FN, 0 failures; refusals are failures). A check joins
# when S6's leaf constructs make it exact, and never leaves: the list is a ratchet. Every other check is
# measured and reported, not gated.
GATED: set[str] = {
    # exact in both at 7d23ab4a7 (run 20261022T204421-gate)
    "removed_compat_surfaces", "versioned_lifecycle_legacy_surfaces", "no_api_root_imports", "serverplan_grammar_surface",
    "conn_or_txn_union", "render_fstrings",
    # exact in both at c176eefc7, S6's 39d33188c and 8bdf55063 merged (run 20261022T213016-gate)
    "backend_type_strings", "hasattr_async", "no_source_inspection", "serverplan_name_inference", "no_blanket_grammar_bypass",
    "permission_error",
    "dynamic_module_loading", "forwardref", "unresolved_forward_refs", "classvar_check", "type_safety_patterns", "async_executor",
    "async_patterns", "async_sync_boundary", "admin_async_purity",
    "plan_scope_side_effects", "plan_scope_ownership", "transaction_closures", "transaction_callback_wrappers",
    "transaction_patterns", "lazy_result_usage", "swallowed_exceptions", "error_handling", "module_level_caches", "connection_caches",
    "render_raw_strings", "regex_sql_parsing", "grammar_render_methods", "grammar_str_params", "grammar_allowlist_burndown",
    "rust_sql_grammar_boundary",
    "missing_async_pairs", "sync_async_signatures", "async_sync_delegation", "data_layer_facade_purity", "data_layer_compat_aliases",
    "layer_violations", "backend_coupling",
    # exact in both with S6's 31d76fa73 (budget evidence closure; run 20261022T234304-gate)
    "sql_grammar_escape_hatches", "grammar_callable_constructors", "grammar_fragment_usage",
}

# The generic mutants: (number, name, crate, site, what it breaks, the checks that run against it). They
# target components, not rules: the delta generator (S3), the classifier, the plan audit and the
# order-totality proof (S1). Rewrite
# mutants come from docs/facts/REWRITES.md (`rewrite_mutants`), one per registered rewrite; 1-3 are the
# interim mutants of S2's A3 views (gone with subst.rs). Numbering: REWRITES.md § Numbers.
MUTANTS = [
    (1, "a3-dropped-side-condition", "facts-a", "subst.rs::edges (A3 view)", "dropped side condition: edges keeps rows whose edge_parent is NULL", ["a3-check", "a-reference"]),
    (2, "a3-lost-union-arm", "facts-a", "subst.rs::nodes (A3 view)", "lost union arm: nodes drops the last arm of its source union", ["a3-check", "a-reference"]),
    (3, "a3-interval-bound", "facts-a", "ast.rs::has_end_memo (A3 memo)", "wrong interval bound: the descendant range starts at id + 2", ["a3-check", "a-reference"]),
    (4, "delta-cross-term", "facts-b", "gen: join tags (tags.rs, lower.rs)", "the join drops the ΔR⋈ΔS cross term", ["b-generated"]),
    (5, "delta-count-retraction", "facts-b", "gen: presence/count (lower.rs)", "counts ignore retractions (negative deltas)", ["b-generated"]),
    (6, "delta-fixpoint-early", "facts-b", "gen: fixpoint (lower.rs)", "the fixpoint stops early (rounds >= 2 dropped)", ["b-generated"]),
    (7, "classifier-membership", "rerail-facts", "classify::file_source", "a per-content plan reaching a membership relation is accepted", ["classifier-leak", "rerail-facts-lib"]),
    (8, "planner-inner-scan", "rerail-facts", "phys.rs plan audit", "the plan audit admits an inner scan", ["planner-audit", "rerail-facts-lib"]),
    (9, "tie-detection-off", "facts-b", "gen: runtime tie check", "a tie between distinct rows under a Checked order is not refused", ["b-ties"]),
    (10, "order-proven-total", "rerail-facts", "classify.rs::KeyEnv::total", "every order is marked proven total (no runtime tie check)", ["rerail-facts-lib"]),
]
REWRITES = ROOT / "docs/facts/REWRITES.md"


def rewrite_mutants() -> tuple[list[tuple], list[str]]:
    """One mutant per row of REWRITES.md's registry: the first markdown table whose header has `rewrite`
    and `mutant` columns (contiguous lines); a mutant cell reads `<n> <kind>`. Each runs the crate's lib
    tests, which hold the rewrite's differential test. Returns (mutants, problems); a rewrite without a
    well-formed mutant is a problem, so the gate fails until every registered rewrite has one."""
    if not REWRITES.exists():
        return [], ["docs/facts/REWRITES.md is missing"]
    lines = REWRITES.read_text().splitlines()
    table: list[list[str]] = []
    for i, line in enumerate(lines):
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if line.strip().startswith("|") and any(c.lower() == "mutant" for c in cells) and any(c.lower() == "rewrite" for c in cells):
            for row in lines[i:]:
                if not row.strip().startswith("|"):
                    break
                table.append([c.strip() for c in row.strip().strip("|").split("|")])
            break
    if not table:
        return [], ["REWRITES.md has no table with `rewrite` and `mutant` columns"]
    cols = [c.lower() for c in table[0]]
    ri, mi = cols.index("rewrite"), cols.index("mutant")
    ii = cols.index("id") if "id" in cols else ri
    out, problems = [], []
    for r in table[2:]:
        # The mutant cell counted from the row's end: a `|` inside an earlier cell cannot shift it.
        name, cell = f"{r[ii]} {r[ri]}".strip(), r[mi - len(cols)] if len(r) >= len(cols) else ""
        words = cell.split(maxsplit=1)
        if len(words) < 2 or not words[0].isdigit():
            problems.append(f"rewrite {name}: mutant cell {cell!r} is not `<n> <kind>`")
            continue
        out.append((int(words[0]), f"rewrite-{r[ii].lower()}", "rerail-facts", f"rewrite {name}", words[1], ["rerail-facts-lib"]))
    return out, problems


BATCH_CHECKS = [f"harness-b{n}" for n in BATCH_RECORDINGS] + [f"suite-b{n}" for n in BATCH_RECORDINGS]
LANES = {"a": ["a-replay", "a-repo", "a-subset", "a3-check", "a-reference"], "b": ["b-exactness"], "g": ["b-generated"],
         "c": ["rerail-facts-lib", "classifier-leak", "planner-audit", "explain-stable", "harness-replay"],
         "v": ["views-derived"], "h": [*BATCH_CHECKS, "explain-physical"], "t": ["b-ties"]}
# The edit replays' prefix (b-exactness, b-generated and their mutants): events 0-299 hold every
# single-file edit kind, one new worktree (event 100) and one 200-file bulk replace (event 161), but none
# of the 51 real commits (events 1060-1110). `--full` replays all 1,111 events.
EDITS_PREFIX = 300
# S4's views differential on a fixed eighth of the corpus: derived-check's `--shard k/n` keeps every n-th
# file of the sorted walk from the k-th.
VIEWS_SHARD = "0/8"
# Lane v outlives every other lane, which finish within about 2 minutes, so it gets 8 workers.
VIEWS_THREADS = "8"
PACKAGES = ["facts-a", "facts-b", "rerail-facts"]

print_lock = threading.Lock()


def say(line: str) -> None:
    with print_lock:
        print(f"facts-gate: {line}", flush=True)


def load1() -> str:
    return subprocess.run(["sysctl", "-n", "vm.loadavg"], capture_output=True, text=True).stdout.split()[1]


def run(cmd: list[str], d: Path, env: dict[str, str] | None = None, cwd: Path = ROOT) -> tuple[int, float]:
    """Runs `cmd` under /usr/bin/time -l with the run-directory protocol; returns (status, seconds)."""
    d.mkdir(parents=True, exist_ok=True)
    extra = {k: v for k, v in (env or {}).items()}
    (d / "command.txt").write_text(
        f"commit {commit()}\ncwd {cwd}\nenv {json.dumps(extra, sort_keys=True)}\n{' '.join(cmd)}\n")
    (d / "uptime-before.txt").write_text(subprocess.run(["uptime"], capture_output=True, text=True).stdout)
    t0 = time.monotonic()
    with open(d / "stdout.txt", "w") as out, open(d / "stderr.txt", "w") as err:
        status = subprocess.run(["/usr/bin/time", "-l", *cmd], stdout=out, stderr=err, cwd=cwd,
                                env={**os.environ, **extra}).returncode
    seconds = time.monotonic() - t0
    (d / "uptime-after.txt").write_text(subprocess.run(["uptime"], capture_output=True, text=True).stdout)
    return status, seconds


_commit: str | None = None


def commit() -> str:
    global _commit
    if _commit is None:
        sha = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        dirty = subprocess.run(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        _commit = sha + ("+dirty" if dirty else "")
    return _commit


# ---------------------------------------------------------------------------------------------------
# Build


def workspace_packages() -> dict[str, dict]:
    meta = json.loads(subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT,
                                     capture_output=True, text=True, check=True).stdout)
    return {p["name"]: p for p in meta["packages"]}


def build(rd: Path) -> tuple[bool, dict]:
    target = Path(os.environ["FACTS_GATE_TARGET"])
    seed = ROOT / "target" / "release"
    if not (target / "release").exists() and seed.is_dir():
        # An APFS clone of the workspace's release build, so dependencies are not rebuilt.
        target.mkdir(parents=True, exist_ok=True)
        subprocess.run(["cp", "-cR", str(seed), str(target / "release")], check=False)
    pkgs = workspace_packages()
    present = [p for p in PACKAGES if p in pkgs]
    mutants = [p for p in present if "mutants" in pkgs[p].get("features", {})]
    env = {"CARGO_TARGET_DIR": str(target)}
    bins = [p for p in present if p in ("facts-a", "facts-b")]
    features = ["--features", ",".join(f"{p}/mutants" for p in bins if p in mutants)] if any(p in mutants for p in bins) else []
    t0 = time.monotonic()
    status, _ = run(["cargo", "build", "--release", *sum((["-p", p] for p in bins), []), *features], rd / "build", env)
    if status == 0 and "rerail-facts" in present:
        f = ["--features", "rerail-facts/mutants"] if "rerail-facts" in mutants else []
        status, _ = run(["cargo", "test", "--release", "-p", "rerail-facts", *f, "--no-run"], rd / "build-tests", env)
    info = {"seconds": round(time.monotonic() - t0, 1), "packages": present, "mutants_feature": mutants,
            "target": str(target), "ok": status == 0}
    say(f"build {'ok' if status == 0 else 'FAIL'} {info['seconds']:.0f}s "
        + " ".join(p + ("[mutants]" if p in mutants else "") for p in present))
    return status == 0, info


def binary(name: str) -> str:
    return str(Path(os.environ["FACTS_GATE_TARGET"]) / "release" / name)


# ---------------------------------------------------------------------------------------------------
# Inputs


def subset_root() -> Path:
    """The deterministic subset, hard-linked from the corpus; rebuilt when its list changes."""
    oracle = json.loads(ORACLE.read_text())
    finding_paths = {f["path"] for e in oracle["events"] for f in e["findings"]}
    paths = sorted(f["path"] for f in oracle["files"]
                   if hashlib.sha256(f["path"].encode()).digest()[0] % 8 == 0 or f["path"] in finding_paths)
    digest = hashlib.sha256("\n".join(paths).encode()).hexdigest()
    stamp = SUBSET / ".subset-sha256"
    if stamp.exists() and stamp.read_text().strip() == digest:
        return SUBSET
    if SUBSET.exists():
        subprocess.run(["rm", "-rf", str(SUBSET)], check=True)
    for p in paths:
        dst = SUBSET / p
        dst.parent.mkdir(parents=True, exist_ok=True)
        os.link(CORPUS / p, dst)
    (SUBSET.parent / "subset-8.list").write_text("\n".join(paths) + "\n")
    stamp.write_text(digest + "\n")
    return SUBSET


def recorded_calls() -> int:
    n = 0
    for rec in RECORDINGS:
        for e in json.loads((ROOT / rec).read_text())["events"]:
            if e["check"] in BATCH6 and "retained_reason" not in e and e.get("upstream_passed") is not False and "files" in e:
                n += 1
    return n


def residue(check: str, clause: str, message: str) -> bool:
    return any(check == c and clause == cl and m in message for c, cl, m in RESIDUE)


# ---------------------------------------------------------------------------------------------------
# Checks: each returns (ok, summary, details)


def check_a_replay(d: Path, env: dict) -> tuple[bool, str, dict]:
    out = d / "results.json"
    cmd = [binary("facts-a"), "replay", *sum((["--recording", r] for r in RECORDINGS), []), "--output", str(out)]
    status, _ = run(cmd, d, env)
    if status != 0 or not out.exists():
        return False, f"exit {status}", {}
    r = json.loads(out.read_text())
    t = r["totals"]
    bad = []
    for row in r["mismatches"]:
        if "compile_error" in row:
            bad.append(f"compile error {row['check']}")
            continue
        fails = row.get("failures", [])
        if not fails or not all(residue(row["check"], f["clause"], f["message"]) for f in fails) or row.get("false_positives"):
            bad.append(f"{row['check']} {row['test']} call {row['call']}")
    want = recorded_calls()
    matched = t.get("rows", 0) - len(r["mismatches"])
    ok = t.get("false_positives", 0) == 0 and not bad and t.get("rows", 0) == want
    summary = (f"{matched}/{t.get('rows', 0)} calls (recorded {want}), {t.get('false_positives', 0)} FP, "
               f"{t.get('false_negatives', 0)} FN, {t.get('failures', 0)} failures"
               + ("" if not bad else f", unaccepted misses: {'; '.join(bad[:5])}")
               + (", misses all accepted residue" if not bad and r["mismatches"] else ""))
    return ok, summary, {"totals": t, "unaccepted": bad, "recorded_calls": want}


def repo_cmd(d: Path, root: Path | None) -> list[str]:
    return [binary("facts-a"), "repo", "--threads", THREADS, "--out-dir", str(d), *(["--root", str(root)] if root else [])]


def failures_ok(results: dict) -> list[str]:
    return [f"{f['check']}/{f['clause']} {f['path']}: {f['message'][:80]}" for f in results["failures"]
            if not residue(f["check"], f["clause"], f["message"])]


def check_a_repo(d: Path, env: dict) -> tuple[bool, str, dict]:
    status, _ = run(repo_cmd(d, None), d, env)
    if status != 0 or not (d / "results.json").exists():
        return False, f"exit {status}", {}
    cmp = [binary("facts-a"), "repo-compare", "--oracle", str(ORACLE), "--results", str(d / "results.json"), "--output", str(d / "parity.json")]
    status, _ = run(cmp, d / "compare", env)
    if status != 0:
        return False, f"repo-compare exit {status}", {}
    p = json.loads((d / "parity.json").read_text())
    t, files = p["totals"], p["files"]
    bad = failures_ok(json.loads((d / "results.json").read_text()))
    ok = (t["false_positives"] == 0 and t["false_negatives"] == 0 and not bad and not p["config_issues"]
          and not files["missing"] and not files["extra"] and files["walked"] == files["oracle"])
    actual = t["expected"] - t["false_negatives"] + t["false_positives"]
    summary = (f"{actual}/{t['expected']} findings, {t['false_positives']} FP, {t['false_negatives']} FN, "
               f"{t['failures']} failures{'' if not t['failures'] else (' (all accepted residue)' if not bad else f' ({len(bad)} unaccepted)')}, "
               f"{files['walked']}/{files['oracle']} files")
    return ok, summary, {"totals": t, "files": files, "unaccepted_failures": bad[:20]}


def check_a3(d: Path, env: dict) -> tuple[bool, str, dict]:
    status, _ = run(repo_cmd(d, subset_root()), d, {**env, "FACTS_A3_CHECK": "1"})
    err = (d / "stderr.txt").read_text(errors="replace")
    marker = next((l.strip() for l in err.splitlines() if "FACTS_A3_CHECK" in l), None)
    if status != 0:
        return False, f"exit {status}" + (f": {marker[:160]}" if marker else ""), {"detected": marker is not None, "marker": marker}
    r = json.loads((d / "results.json").read_text())
    return True, f"{r['files']} files, every substitution equal to the unsubstituted path", {"detected": False}


def findings_of(path: Path) -> tuple[collections.Counter, collections.Counter]:
    r = json.loads(path.read_text())
    return (collections.Counter(json.dumps(f) for f in r["findings"]),
            collections.Counter(json.dumps([f["check"], f["clause"], f["path"], f["message"]]) for f in r["failures"]))


def check_a_subset(d: Path, env: dict) -> tuple[bool, str, dict]:
    status, _ = run(repo_cmd(d, subset_root()), d, env)
    if status != 0 or not (d / "results.json").exists():
        return False, f"exit {status}", {}
    f, x = findings_of(d / "results.json")
    return True, f"{sum(f.values())} findings, {sum(x.values())} failures", {}


def compare_reference(compiled: Path, reference: Path) -> tuple[bool, str, dict]:
    cf, cx = findings_of(compiled)
    rf, rx = findings_of(reference)
    diff = {"compiled_only": list((cf - rf).elements())[:10], "reference_only": list((rf - cf).elements())[:10],
            "failures_compiled_only": list((cx - rx).elements())[:10], "failures_reference_only": list((rx - cx).elements())[:10]}
    n = sum(len(v) for v in diff.values())
    if n == 0:
        return True, f"{sum(cf.values())} findings, {sum(cx.values())} failures, equal to the reference", diff
    return False, (f"{sum((cf - rf).values())} findings only compiled, {sum((rf - cf).values())} only reference, "
                   f"{sum((cx - rx).values()) + sum((rx - cx).values())} failures differ"), diff


def check_a_reference(d: Path, env: dict, gate_dir: Path) -> tuple[bool, str, dict]:
    status, _ = run(repo_cmd(d, subset_root()), d, {**env, "FACTS_A3_OFF": "1", "FACTS_MUTANT": "0"})
    if status != 0 or not (d / "results.json").exists():
        return False, f"exit {status}", {}
    compiled = gate_dir / "a-subset" / "results.json"
    if not compiled.exists():
        return False, "a-subset did not run", {}
    return compare_reference(compiled, d / "results.json")


def check_b(d: Path, env: dict, sub: str, fail_fast: bool = False, plans: str | None = None, limit: int | None = None) -> tuple[bool, str, dict]:
    """`facts-b <sub> --events E`: `replay` (Spike B's hand-written delta rules, the generator's test
    vectors) or `greplay` (the delta-plan generator over IR plans, against rerail_facts::reference;
    `plans`/`limit` select plans and a prefix of the events). A run-time refusal passes only on a plan
    whose refusals the run expects (`expected_refusals`, S3's constructed-tie vector `rt`); an expected
    refusal the dataflow misses (`missing_expected_refusals`) is a detection, like a difference."""
    cmd = [binary("facts-b"), sub, "--events", str(EVENTS), "--run-dir", str(d), *(["--fail-fast"] if fail_fast else []),
           *(["--plans", plans] if plans else []), *(["--limit", str(limit)] if limit else [])]
    status, _ = run(cmd, d, env)
    if status != 0 or not (d / "results.json").exists():
        return False, f"exit {status}", {"detected": False}
    r = json.loads((d / "results.json").read_text())
    x = r["exactness"]
    want = limit or sum(1 for l in EVENTS.read_text().splitlines() if l.strip())
    refusals = r.get("runtime_refusals", {})
    expected = r.get("expected_refusals", {})
    missing = r.get("missing_expected_refusals", 0)
    seen = {k.split(":", 1)[0] for k in r.get("runtime_refusals_seen", {})} if "runtime_refusals_seen" in r else ({"?"} if refusals else set())
    unexpected = sorted(seen - set(expected))
    names = ",".join(p["plan"] for p in r.get("plans", [])) or "R1-R3 (hand-written)"
    ok = x["differences"] == 0 and x["checks"] > 0 and r["events"] == want and not unexpected and missing == 0
    return ok, (f"{x['differences']} differences in {x['checks']} checkpoints, {x['worktree_checks']} worktree checks, "
                f"{r['events']}/{want} events, plans {names}"
                + "".join(f", expected refusals {p} {e['refused']}/{e['expected']}" for p, e in expected.items())
                + (f", MISSING expected refusals {missing}" if missing else "")
                + (f", unexpected runtime refusals on {','.join(unexpected)}: {refusals}" if unexpected else "")), \
        {"exactness": x, "expected_refusals": expected, "missing_expected_refusals": missing, "unexpected_refusals": unexpected,
         "detected": x["differences"] > 0 or missing > 0}


TIES_LIMIT = 200


def check_ties(d: Path, env: dict) -> tuple[bool, str, dict]:
    """S3's constructed-tie vector: `facts-b greplay --plans rt` over the first TIES_LIMIT events. `rt`
    ties in every worktree, so every worktree's tie must be refused at run time (expected refusals all
    met), with 0 differences against the reference (which refuses the same ties, `tie: …`)."""
    ok, summary, det = check_b(d, env, "greplay", plans="rt", limit=TIES_LIMIT)
    met = det.get("expected_refusals", {}).get("rt")
    if ok and not (met and met["expected"] > 0):
        return False, summary + ", NO expected refusals reported for rt", det
    return ok, summary, det


def cargo_test(d: Path, env: dict, *names: str) -> tuple[int, int, list[str]]:
    """`cargo test --release -p rerail-facts [--features mutants] <names>`: (exit status, tests run,
    failed tests)."""
    pkgs = workspace_packages()
    f = ["--features", "rerail-facts/mutants"] if "mutants" in pkgs["rerail-facts"].get("features", {}) else []
    status, _ = run(["cargo", "test", "--release", "-p", "rerail-facts", *f, *names], d,
                    {**env, "CARGO_TARGET_DIR": os.environ["FACTS_GATE_TARGET"]})
    out = (d / "stdout.txt").read_text(errors="replace")
    ran = sum(int(l.split()[1]) for l in out.splitlines() if l.startswith("running ") and l.split()[1].isdigit())
    failed = [l.split()[1] for l in out.splitlines() if l.startswith("test ") and l.rstrip().endswith("FAILED")]
    return status, ran, failed


def check_cargo(d: Path, env: dict, what: str, *names: str) -> tuple[bool, str, dict]:
    if "rerail-facts" not in workspace_packages():
        return False, "package rerail-facts is not in the workspace", {"detected": False}
    status, ran, failed = cargo_test(d, env, *names)
    return (status == 0 and ran > 0, f"{ran} {what}, exit {status}" + (f", failed: {', '.join(failed[:6])}" if failed else ""),
            {"tests": ran, "failed": failed, "detected": status != 0 and ran > 0})


def check_explain(d: Path, env: dict) -> tuple[bool, str, dict]:
    """EXPLAIN stability (INVARIANTS I5): `facts-a explain` over the pack, twice in fresh processes, and
    once per rule in its own process; all three texts byte-identical (no addresses, no hash order, no
    state carried between rules). Per-file identity holds by construction while EXPLAIN prints the
    rule's plan with no file input; it becomes a per-file comparison once the physical EXPLAIN takes one."""
    rules = ROOT / "packs/data-layer/candidates/rules"
    texts = []
    for i in (1, 2):
        status, _ = run([binary("facts-a"), "explain", "--rules", str(rules)], d / f"all-{i}", env)
        if status != 0:
            return False, f"explain exit {status}", {}
        texts.append((d / f"all-{i}" / "stdout.txt").read_bytes())
    ids = [json.loads(p.read_text())["id"] for p in sorted(rules.glob("*.json"))]
    per_rule = b""
    for rid in ids:
        status, _ = run([binary("facts-a"), "explain", "--rules", str(rules), "--rule", rid], d / "rule" / rid, env)
        if status != 0:
            return False, f"explain --rule {rid} exit {status}", {}
        per_rule += (d / "rule" / rid / "stdout.txt").read_bytes()
    texts.append(per_rule)
    same = texts[0] == texts[1] == texts[2]
    digest = hashlib.sha256(texts[0]).hexdigest()[:16]
    return same and len(texts[0]) > 0, (f"{len(ids)} rules, {len(texts[0])} bytes, sha256 {digest}, "
                                        + ("identical across 2 runs and per-rule processes" if same else "TEXT DIFFERS")), {}


def check_explain_physical(d: Path, env: dict) -> tuple[bool, str, dict]:
    """Physical EXPLAIN per file (S1's `facts-a explain --physical --root <subset> --audit`), twice in
    fresh processes: the two outputs are byte-identical, every subset file is printed, and every
    clause's block (`clause <check>/<clause>` up to the next header) is the same text in every file
    that prints it. The ~400 MB outputs are hashed and deleted; one text per clause is kept
    (`clauses.txt`). Inner scans the audit reports are counted, not failed (informational)."""
    root = subset_root()
    want = len((SUBSET.parent / "subset-8.list").read_text().splitlines())
    digests: list[str] = []
    blocks: dict[str, dict[str, list]] = {}
    files = inner = 0
    for i in (1, 2):
        rd = d / f"run-{i}"
        status, _ = run([binary("facts-a"), "explain", "--physical", "--root", str(root), "--audit"], rd, env)
        out = rd / "stdout.txt"
        if status != 0:
            return False, f"explain --physical exit {status}", {}
        h, files, inner = hashlib.sha256(), 0, 0
        clause: str | None = None
        block: list[str] = []

        def close() -> None:
            if clause is not None and i == 1:
                text = "".join(block)
                blocks.setdefault(clause, {}).setdefault(hashlib.sha256(text.encode()).hexdigest(), [0, text])[0] += 1

        with open(out, encoding="utf-8") as f:
            for line in f:
                h.update(line.encode())
                if line.startswith("== file ") or line.startswith("clause "):
                    close()
                    clause, block = (line.strip(), []) if line.startswith("clause ") else (None, [])
                    files += line.startswith("== file ")
                elif clause is not None:
                    block.append(line)
                    inner += line.lstrip().startswith("! ")
            close()
        digests.append(h.hexdigest())
        size = out.stat().st_size
        out.write_text(f"sha256 {digests[-1]}, {size} bytes, {files} files; deleted after checking (re-run command.txt to reproduce)\n")
    varying = {c: len(v) for c, v in blocks.items() if len(v) > 1}
    with open(d / "clauses.txt", "w") as f:
        for c in sorted(blocks):
            for n, text in blocks[c].values():
                f.write(f"{c}  ({n} files)\n{text}")
    same = digests[0] == digests[1]
    ok = same and files == want and not varying and bool(blocks)
    return ok, (f"{files}/{want} files, {len(blocks)} clauses, sha256 {digests[0][:16]}, "
                + ("identical across 2 runs" if same else "RUNS DIFFER")
                + (f", clauses varying across files: {', '.join(sorted(varying))}" if varying else ", each clause one text in every file")
                + f"; {inner} inner-scan lines (audit, informational)"), {"digests": digests, "varying": varying, "inner_scan_lines": inner}


def check_views(d: Path, env: dict) -> tuple[bool, str, dict]:
    """S4's views differential (`facts-a derived-check`): the schema's SQL views over the syntactic core
    against the policies' own stage texts on VIEWS_SHARD (every stage text and clause)."""
    status, _ = run([binary("facts-a"), "derived-check", "--root", str(CORPUS), "--threads", VIEWS_THREADS, "--shard", VIEWS_SHARD, "--out-dir", str(d)], d, env)
    if status != 0 or not (d / "results.json").exists():
        return False, f"exit {status}", {}
    r = json.loads((d / "results.json").read_text())
    ok = r.get("bag_mismatch_total") == 0 and not r.get("nodes_mismatch") and not r.get("failures")
    return ok, (f"bag mismatches {r.get('bag_mismatch_total')}, nodes mismatches {len(r.get('nodes_mismatch') or [])}, "
                f"failures {len(r.get('failures') or [])}, order mismatches {r.get('order_mismatch_total')} (informational)"), {}


def check_harness_replay(d: Path, env: dict) -> tuple[bool, str, dict]:
    """Jeremy's harness (`replay.ts --engine facts`) over the same recordings: the same acceptance as a-replay."""
    if not (ROOT / "node_modules" / "tsx").exists():
        return False, "node_modules missing in the checkout (npm ci, or link a checkout's node_modules)", {}
    out = d / "results.json"
    cmd = ["node", "--import", "tsx", "conformance/data-layer/replay.ts", *sum((["--recording", r] for r in RECORDINGS), []),
           "--output", str(out), "--engine", "facts"]
    status, _ = run(cmd, d, {**env, "FACTS_ENGINE_BIN": binary("facts-a")})
    if not out.exists():
        return False, f"exit {status}", {}
    r = json.loads(out.read_text())
    bad = [f"{row['check']} {row['test']}" for row in r["rows"] if row["falsePositives"] or row["executionPathDifferences"]
           or ((row["falseNegatives"] or row["engineFailures"]) and not (row["engineFailures"] and all(
               any(c == row["check"] and f"{c}/{cl}: " in f["message"] and m in f["message"] for c, cl, m in RESIDUE) for f in row["engineFailures"])))]
    want = recorded_calls()
    matched = sum(1 for row in r["rows"] if not (row["falsePositives"] or row["falseNegatives"] or row["engineFailures"]))
    ok = not bad and r["replayedCalls"] == want and r["factsCalls"] == want and not r["missingCoverage"]
    return ok, (f"{matched}/{r['replayedCalls']} calls, {r['falsePositives']} FP, {r['falseNegatives']} FN, {r['engineFailures']} failures"
                + ("" if not bad else f", unaccepted: {'; '.join(bad[:5])}")), {"unaccepted": bad}


def batch_checks(n: int) -> list[str]:
    return json.loads((ROOT / PORT / "batches.json").read_text())["batches"][n - 1]["checks"]


def batch_verdict(n: int, per: dict[str, dict], unattributed: list[str]) -> tuple[bool, str, dict]:
    """Per-check status (exact / uncovered / differs / refused) for batch `n`; passes when every gated
    check of the batch is exact and nothing unattributed failed while one is gated. A failure of any kind
    (compile error, unsupported construct, panic, Rust source) is a refusal and counts against
    exactness; a replayed check without both a positive and a clean call is uncovered."""
    status = {c: ("refused" if v["failures"] else "differs" if v["fp"] or v["fn"] else
                  "uncovered" if v.get("positive") == 0 or v.get("clean") == 0 else "exact") for c, v in per.items()}
    gated = [c for c in per if c in GATED]
    bad = [c for c in gated if status[c] != "exact"] + (["unattributed failures"] if unattributed and gated else [])
    by = {s: [c for c in per if status[c] == s] for s in ("exact", "uncovered", "differs", "refused")}
    summary = (f"b{n}: {len(by['exact'])}/{len(per)} checks exact, {sum(v['fp'] for v in per.values())} FP, "
               f"{sum(v['fn'] for v in per.values())} FN, {sum(v['failures'] for v in per.values()) + len(unattributed)} failures; "
               f"gated {len(gated) - len(bad)}/{len(gated)} exact"
               + "".join(f"; {s}: {','.join(by[s])}" for s in ("uncovered", "differs", "refused") if by[s])
               + (f"; unattributed failures {len(unattributed)}" if unattributed else "")
               + (f"; GATED NOT EXACT: {', '.join(bad)}" if bad else ""))
    return not bad, summary, {"checks": per, "status": status, "unattributed": unattributed[:20], "gated_failing": bad}


def check_harness_batch(d: Path, env: dict, n: int) -> tuple[bool, str, dict]:
    """`replay.ts --engine facts` over batch `n`'s recordings; per check: calls, exact calls, FP, FN,
    failures (the first messages kept)."""
    if not (ROOT / "node_modules" / "tsx").exists():
        return False, "node_modules missing in the checkout", {}
    out = d / "results.json"
    cmd = ["node", "--import", "tsx", "conformance/data-layer/replay.ts",
           *sum((["--recording", f"{PORT}/{r}"] for r in BATCH_RECORDINGS[n]), []), "--output", str(out), "--engine", "facts"]
    status, _ = run(cmd, d, {**env, "FACTS_ENGINE_BIN": binary("facts-a")})
    if not out.exists():
        return False, f"exit {status}", {}
    r = json.loads(out.read_text())
    blank = {"calls": 0, "exact_calls": 0, "positive": 0, "clean": 0, "fp": 0, "fn": 0, "failures": 0, "messages": []}
    per: dict[str, dict] = {c: {**blank, "messages": []} for c in batch_checks(n)}
    for row in r["rows"]:
        v = per.setdefault(row["check"], {**blank, "messages": []})
        fails = row["engineFailures"] + row["statelessFailures"]
        v["calls"] += 1
        v["positive" if row["positive"] else "clean"] += 1
        v["fp"] += len(row["falsePositives"])
        v["fn"] += len(row["falseNegatives"])
        v["failures"] += len(fails)
        v["exact_calls"] += not (row["falsePositives"] or row["falseNegatives"] or fails or row["executionPathDifferences"])
        for f in fails:
            if f["message"] not in v["messages"] and len(v["messages"]) < 3:
                v["messages"].append(f["message"][:200])
    ok, summary, det = batch_verdict(n, per, [])
    calls = f"{sum(v['exact_calls'] for v in per.values())}/{r['replayedCalls']} calls exact"
    return ok, summary.replace(f"b{n}: ", f"b{n}: {calls}, ", 1), {**det, "replayed_calls": r["replayedCalls"],
                                                                    "facts_calls": r["factsCalls"], "host_guard_calls": r["hostGuardCalls"]}


def check_suite_batch(d: Path, env: dict, n: int) -> tuple[bool, str, dict]:
    """`suite.ts --engine facts --cold-only` over batch `n`'s repository oracle (SUITES/suite-b<n>.json);
    per check: oracle findings, FP, FN, failures. The incremental phases are from-scratch reruns for the
    facts engine (report § 5.2), so the gate runs the cold pass only."""
    if not (ROOT / "node_modules" / "tsx").exists():
        return False, "node_modules missing in the checkout", {}
    oracle = SUITES / f"suite-b{n}.json"
    if not oracle.exists():
        return False, f"missing oracle {oracle}", {}
    out = d / "results.json"
    cmd = ["node", "--import", "tsx", "conformance/data-layer/suite.ts", "--recording", str(oracle), "--output", str(out),
           "--engine", "facts", "--cold-only"]
    status, _ = run(cmd, d, {**env, "FACTS_ENGINE_BIN": binary("facts-a")})
    if not out.exists():
        err = (d / "stderr.txt").read_text(errors="replace")
        why = next((l.strip() for l in err.splitlines() if "panicked" in l or l.startswith("Error")), "")
        return False, f"exit {status}" + (f": {why[:200]}" if why else ""), {}
    r = json.loads(out.read_text())
    rule = lambda key: json.loads(key)[0].removeprefix("data-layer-").replace("-", "_")
    per: dict[str, dict] = {c: {"expected": 0, "fp": 0, "fn": 0, "failures": 0, "messages": []} for c in batch_checks(n)}
    for e in json.loads(oracle.read_text())["events"]:
        per[e["check"]]["expected"] += len(e["findings"])
    for key in r["falsePositives"]:
        per[rule(key)]["fp"] += 1
    for key in r["falseNegatives"]:
        per[rule(key)]["fn"] += 1
    unattributed = []
    for f in r["failures"]:
        if f["check"] not in per:
            unattributed.append(f"{f['path']}: {f['message'][:160]}")
            continue
        v = per[f["check"]]
        v["failures"] += 1
        if f["message"] not in v["messages"] and len(v["messages"]) < 3:
            v["messages"].append(f["message"][:200])
    return batch_verdict(n, per, unattributed)



# ---------------------------------------------------------------------------------------------------
# Driver


def run_check(cid: str, gate_dir: Path, env: dict, edits: int | None) -> tuple[bool, str, dict]:
    """Runs check `cid`; `edits` is the edit replays' event prefix (None: all events)."""
    d = gate_dir / cid
    if cid == "a-replay":
        return check_a_replay(d, env)
    if cid == "a-repo":
        return check_a_repo(d, env)
    if cid == "a-subset":
        return check_a_subset(d, env)
    if cid == "a3-check":
        return check_a3(d, env)
    if cid == "a-reference":
        return check_a_reference(d, env, gate_dir)
    if cid == "b-exactness":
        return check_b(d, env, "replay", limit=edits)
    if cid == "b-generated":
        return check_b(d, env, "greplay", limit=edits)
    if cid == "b-ties":
        return check_ties(d, env)
    if cid == "rerail-facts-lib":
        return check_cargo(d, env, "lib tests (rewrite differentials, classifier, audit, reference)", "--lib")
    if cid == "classifier-leak":
        return check_cargo(d, env, "leak tests", "leak")
    if cid == "planner-audit":
        return check_cargo(d, env, "plan audit tests", "phys::tests::audit")
    if cid == "explain-stable":
        return check_explain(d, env)
    if cid == "explain-physical":
        return check_explain_physical(d, env)
    if cid == "harness-replay":
        return check_harness_replay(d, env)
    if cid == "views-derived":
        return check_views(d, env)
    if cid.startswith("harness-b"):
        return check_harness_batch(d, env, int(cid.removeprefix("harness-b")))
    if cid.startswith("suite-b"):
        return check_suite_batch(d, env, int(cid.removeprefix("suite-b")))
    raise ValueError(cid)


def run_mutant_check(n: int, cid: str, gate_dir: Path, edits: int | None) -> str:
    """killed | survived | error, for mutant `n` under check `cid`. A kill is a detection the check
    names (an A3_CHECK panic, a findings difference, an exactness difference, a failing test), never a
    crash or a missing tool."""
    d = gate_dir / "mutants" / f"{n}-{cid}"
    env = {"FACTS_MUTANT": str(n)}
    if cid == "a3-check":
        ok, _, det = check_a3(d, env)
    elif cid == "a-reference":
        status, _ = run(repo_cmd(d, subset_root()), d, env)
        reference = gate_dir / "a-reference" / "results.json"
        if status != 0 or not (d / "results.json").exists() or not reference.exists():
            return "error"
        same, _, diff = compare_reference(d / "results.json", reference)
        (d / "difference.json").write_text(json.dumps(diff, indent=1))
        return "survived" if same else "killed"
    elif cid in ("b-exactness", "b-generated"):
        ok, _, det = check_b(d, env, "replay" if cid == "b-exactness" else "greplay", fail_fast=True, limit=edits)
    elif cid == "b-ties":
        ok, _, det = check_ties(d, env)
    elif cid in ("rerail-facts-lib", "classifier-leak", "planner-audit"):
        names = {"rerail-facts-lib": ["--lib"], "classifier-leak": ["leak"], "planner-audit": ["phys::tests::audit"]}[cid]
        ok, _, det = check_cargo(d, env, "tests", *names)
        (d / "failed-tests.txt").write_text("\n".join(det.get("failed", [])) + "\n")
    else:
        raise ValueError(cid)
    return "survived" if ok else ("killed" if det.get("detected") else "error")


def main() -> int:
    ap = argparse.ArgumentParser(prog="facts-gate.sh", description="The facts engine's gate.")
    ap.add_argument("--mutants", action="store_true", help="also run every seeded mutant against its checks")
    ap.add_argument("--only", help="comma-separated check ids (default: all)")
    ap.add_argument("--no-build", action="store_true", help="reuse the last gate build")
    ap.add_argument("--run-dir", help="run directory (default ~/rerail-bench/facts/phase-2/s5-gate/runs/<ts>-gate)")
    ap.add_argument("--full", action="store_true", help=f"replay all 1,111 edit events (default: the first {EDITS_PREFIX})")
    try:
        args = ap.parse_args()
    except SystemExit as e:
        return 2 if e.code else 0
    edits = None if args.full else EDITS_PREFIX
    all_checks = [c for lane in LANES.values() for c in lane]
    only = args.only.split(",") if args.only else all_checks
    unknown = [c for c in only if c not in all_checks]
    if unknown:
        print(f"facts-gate: unknown checks {unknown}; known: {all_checks}", file=sys.stderr)
        return 2
    if "a-reference" in only and "a-subset" not in only:
        only.append("a-subset")
    gate_dir = Path(args.run_dir) if args.run_dir else RUNS / (datetime.datetime.now().strftime("%Y%m%dT%H%M%S") + "-gate")
    gate_dir.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    say(f"run {gate_dir} commit {commit()} load {load1()}")
    report: dict = {"format": "rerail.facts-gate/v1", "commit": commit(), "run_dir": str(gate_dir),
                    "argv": sys.argv[1:], "load_before": load1(), "edit_events": edits or "all", "views_shard": VIEWS_SHARD,
                    "checks": {}, "mutants": []}
    if not args.no_build:
        ok, info = build(gate_dir)
        report["build"] = info
        if not ok:
            report["passed"] = False
            (gate_dir / "gate.json").write_text(json.dumps(report, indent=1))
            say(f"FAILED {time.monotonic() - started:.0f}s: build failed, see {gate_dir}/build")
            return 1
    if any(c.startswith("a") for c in only) and (args.mutants or {"a3-check", "a-subset", "a-reference"} & set(only)):
        subset_root()

    def lane(checks: list[str], mutants: list[tuple]) -> None:
        for cid in checks:
            if cid not in only:
                continue
            t0 = time.monotonic()
            try:
                ok, summary, details = run_check(cid, gate_dir, {}, edits)
            except Exception as e:  # a harness error is a failed check, never a pass
                ok, summary, details = False, f"gate error: {e!r}", {}
            secs = time.monotonic() - t0
            report["checks"][cid] = {"ok": ok, "seconds": round(secs, 1), "summary": summary, "details": details,
                                     "dir": str(gate_dir / cid)}
            say(f"check {cid} {'pass' if ok else 'FAIL'} {secs:.0f}s {summary}")
        for (n, name, crate, site, what, tests) in mutants:
            t0 = time.monotonic()
            cells = {}
            for cid in tests:
                try:
                    cells[cid] = run_mutant_check(n, cid, gate_dir, edits)
                except Exception as e:
                    cells[cid] = "error"
                    (gate_dir / "mutants").mkdir(exist_ok=True)
                    (gate_dir / "mutants" / f"{n}-{cid}.error").write_text(repr(e))
            killed = any(v == "killed" for v in cells.values())
            secs = time.monotonic() - t0
            report["mutants"].append({"n": n, "name": name, "crate": crate, "site": site, "breaks": what,
                                      "killed": killed, "seconds": round(secs, 1), "cells": cells})
            say(f"mutant {n} {name} {'killed' if killed else 'SURVIVED'} {secs:.0f}s "
                + " ".join(f"{c}={v}" for c, v in cells.items()))

    # A mutant runs the selected checks of its family; with none selected, it does not run.
    rewrites, registry_problems = rewrite_mutants()
    report["registry_problems"] = registry_problems if args.mutants else []
    for p in report["registry_problems"]:
        say(f"registry FAIL {p}")
    family = {k: [(n, name, crate, site, what, [t for t in tests if t in only]) for (n, name, crate, site, what, tests) in rewrites + MUTANTS
                  if any(t in LANES[k] for t in tests) and any(t in only for t in tests)] for k in LANES}
    threads = [threading.Thread(target=lane, args=(LANES[k], family[k] if args.mutants else [])) for k in LANES]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    secs = time.monotonic() - started
    checks_ok = [c for c, v in report["checks"].items() if v["ok"]]
    failing = ([c for c, v in report["checks"].items() if not v["ok"]] + [f"mutant {m['n']}" for m in report["mutants"] if not m["killed"]]
               + (["rewrite registry"] if report["registry_problems"] else []))
    killed = sum(1 for m in report["mutants"] if m["killed"])
    passed = not failing
    report.update({"passed": passed, "seconds": round(secs, 1), "load_after": load1()})
    report["mutants"].sort(key=lambda m: m["n"])
    (gate_dir / "gate.json").write_text(json.dumps(report, indent=1, ensure_ascii=False))
    mut = f", {killed}/{len(report['mutants'])} mutants killed" if args.mutants else ""
    say(f"{'passed' if passed else 'FAILED'} {secs:.0f}s: {len(checks_ok)}/{len(report['checks'])} checks pass{mut}"
        + (f"; failing: {', '.join(failing)}" if failing else ""))
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
