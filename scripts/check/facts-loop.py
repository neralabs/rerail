#!/usr/bin/env python3
"""The evaluation loop's fast tiers (docs/facts/LOOP.md): `just facts-t0`, `just facts-t1`, `just facts-bench`.

One build for all of them: the `loop` profile (release without LTO, 16 codegen units, incremental) of
facts-a, facts-b and rerail-facts' tests, with the `mutants` features on (the hooks read FACTS_MUTANT and
are free while it is unset), into the tree's own `target/loop`. The gate (T2, `just facts-gate`) builds
`release` into `target/release`; both use scripts/check/facts-gate.py's checks and its output format.

  t0 [--base REF]   unit tests (the rewrite and engine differentials against bundled SQLite 3.53.2 and
                    rerail_facts::reference), then every seeded mutant whose site (`mutant::on(<n>)`) is in
                    a file changed since the merge-base with REF (default: the integration branch
                    Moti-Velz/facts-engine), against the T0 checks of its family: the unit-test binary of
                    its crate, `b-generated` over the first T0_EDITS events (mutants 4-6), `b-ties` (9).
                    Budget: 60 s with the incremental build.
  t1 [--rebase NOTE] [--replay]
                    sub200-parity: `facts-a repo` over sub200 (the digest memo consulted) against
                    benchmark/facts/sub200-reference.json, findings and failures per clause (--rebase
                    rewrites the reference, NOTE kept in its history); ref50-vs-reference: stage-check
                    over benchmark/facts/ref50.list, stages and their algebraic counts against
                    rerail_facts::reference; b-exactness/b-generated over T1_EDITS events when a file
                    under spikes/facts-b changed (--replay: always); bench (below). Budget: 2 min.
  bench [archetype…] [--compare RUN_DIR] [--serial]
                    benchmark/facts/archetypes.json: per archetype its counts (algebraic, then
                    physical; facts-b's dataflow records for the repository one), then wall-clock
                    (not calibrated). An A/B is two runs, the second with --compare the first's dir.

Output (stdout), facts-gate.py's format, one line per event, every line prefixed `facts-gate: `:

  facts-gate: run <run dir> commit <sha>[+dirty] load <1-min load>
  facts-gate: build <ok|FAIL> <s>s <package>[mutants] ...
  facts-gate: changed <n> files since <merge-base sha> (<base ref>); mutant sites in <files>
  facts-gate: check <id> <pass|FAIL> <s>s <summary>
  facts-gate: mutant <n> <name> <killed|SURVIVED> <s>s <check>=<killed|survived|error> ...
  facts-gate: <passed|FAILED> <s>s: <p>/<c> checks pass, <k>/<m> mutants killed[; failing: <ids>]

and <run dir>/loop.json (format rerail.facts-loop/v1) with the same rows. Exit 0: passed; 1: failed;
2: usage.
"""
from __future__ import annotations

import argparse
import collections
import concurrent.futures
import datetime
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location("facts_gate", ROOT / "scripts/check/facts-gate.py")
gate = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gate)
gate.PROFILE = "loop"
say = gate.say

LOOP = gate.BENCH / "facts/loop"
RUNS = LOOP / "runs"
BASE = "Moti-Velz/facts-engine"
# The edit replay's prefix for the generator's mutants 4-6 in T0 (each killed within it, --fail-fast).
T0_EDITS = 30
# Unit-test binaries: check id -> (package, test binary target, test-name filters).
UNIT = {
    "rerail-facts-lib": ("rerail-facts", "rerail_facts", []),
    "facts-a-unit": ("facts-a", "facts-a", []),
    "facts-b-unit": ("facts-b", "facts-b", ["norm::tests"]),
}
CRATES = {"rerail-facts": ROOT / "facts/src", "facts-a": ROOT / "spikes/facts-a/src", "facts-b": ROOT / "spikes/facts-b/src"}
BIN_FEATURES = "facts-a/mutants,facts-b/mutants"
FEATURES = BIN_FEATURES + ",rerail-facts/mutants"
# T1: sub200 (the 200 heaviest corpus files, S2) against its committed reference; the 50 fixed files of
# REF50 (gate subset files of 400-2,000 nodes, first 50 by sha256 of the path) through stage-check; the
# edit replays over T1_EDITS events when facts-b changed.
SUB200 = gate.BENCH / "facts/phase-2/s2-perfile/sub200"
REFERENCE = ROOT / "benchmark/facts/sub200-reference.json"
REF50 = ROOT / "benchmark/facts/ref50.list"
REF50_ROOT = LOOP / "ref50"
T1_EDITS = 100
T1_THREADS = "8"


# ---------------------------------------------------------------------------------------------------
# Build


def build(rd: Path) -> tuple[bool, dict, dict[str, str]]:
    """`cargo build --profile loop` of both binaries and `cargo test --no-run` of the three crates' unit
    tests, one feature set (so nothing builds twice); returns the test executables by package."""
    env = {"CARGO_TARGET_DIR": str(gate.TARGET)}
    t0 = time.monotonic()
    status, _ = gate.run(["cargo", "build", "--profile", gate.PROFILE, "-p", "facts-a", "-p", "facts-b", "--features", BIN_FEATURES],
                         rd / "build", env)
    exes: dict[str, str] = {}
    if status == 0:
        status, _ = gate.run(["cargo", "test", "--profile", gate.PROFILE, *sum((["-p", p] for p in CRATES), []), "--features", FEATURES,
                              "--no-run", "--message-format=json"], rd / "build-tests", env)
        for line in (rd / "build-tests" / "stdout.txt").read_text(errors="replace").splitlines():
            if not line.startswith("{"):
                continue
            m = json.loads(line)
            if m.get("reason") == "compiler-artifact" and m.get("executable") and m.get("profile", {}).get("test"):
                exes[m["target"]["name"]] = m["executable"]
    ok = status == 0 and all(t in exes for _, t, _ in UNIT.values())
    info = {"seconds": round(time.monotonic() - t0, 1), "profile": gate.PROFILE, "target": str(gate.TARGET / gate.PROFILE), "ok": ok, "tests": exes}
    say(f"build {'ok' if ok else 'FAIL'} {info['seconds']:.0f}s " + " ".join(f"{p}[mutants]" for p in CRATES))
    return ok, info, exes


# ---------------------------------------------------------------------------------------------------
# Checks


def unit(d: Path, exe: str, names: list[str], env: dict) -> tuple[int, int, list[str]]:
    """A unit-test binary run directly (no cargo): (exit status, tests run, failed tests)."""
    status, _ = gate.run([exe, *names], d, env)
    out = (d / "stdout.txt").read_text(errors="replace")
    ran = sum(int(l.split()[1]) for l in out.splitlines() if l.startswith("running ") and l.split()[1].isdigit())
    failed = [l.split()[1] for l in out.splitlines() if l.startswith("test ") and l.rstrip().endswith("FAILED")]
    return status, ran, failed


def check_unit(cid: str, d: Path, exes: dict[str, str], env: dict) -> tuple[bool, str, dict]:
    pkg, target, names = UNIT[cid]
    status, ran, failed = unit(d, exes[target], names, env)
    return (status == 0 and ran > 0, f"{ran} tests, exit {status}" + (f", failed: {', '.join(failed[:6])}" if failed else ""),
            {"tests": ran, "failed": failed, "detected": status != 0 and ran > 0})


# ---------------------------------------------------------------------------------------------------
# Changed files and the mutants they hold


def merge_base(ref: str) -> str:
    r = subprocess.run(["git", "merge-base", "HEAD", ref], cwd=ROOT, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"facts-loop: no merge-base with {ref}: {r.stderr.strip()}")
    return r.stdout.strip()


def changed_files(base: str) -> list[str]:
    """Files differing from `base` in the working tree (committed, staged or not), plus untracked ones."""
    diff = subprocess.run(["git", "diff", "--name-only", base], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split()
    new = subprocess.run(["git", "ls-files", "--others", "--exclude-standard"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split()
    return sorted(set(diff) | set(new))


SITE = re.compile(r"mutant::on\(\s*(\d+)")


def mutant_sites() -> dict[int, set[str]]:
    """Every seeded mutant's sites: the files holding `mutant::on(<n>)`, relative to the root."""
    sites: dict[int, set[str]] = {}
    for src in CRATES.values():
        for p in src.rglob("*.rs"):
            for m in SITE.finditer(p.read_text(errors="replace")):
                sites.setdefault(int(m.group(1)), set()).add(str(p.relative_to(ROOT)))
    return sites


def all_mutants() -> tuple[list[tuple], list[str]]:
    """The gate's mutants: REWRITES.md's registry plus facts-gate.py's MUTANTS."""
    rewrites, problems = gate.rewrite_mutants()
    return sorted(rewrites + gate.MUTANTS), problems


def mutant_cell(n: int, cid: str, rd: Path, exes: dict[str, str]) -> tuple[str, float]:
    """(killed | survived | error | n/a, seconds) for mutant `n` under the T0 form of check `cid`; `n/a`
    when the check has no T0 form (the corpus differentials run in T2)."""
    t0 = time.monotonic()
    d = rd / "mutants" / f"{n}-{cid}"
    env = {"FACTS_MUTANT": str(n)}
    if cid in ("classifier-leak", "planner-audit"):
        cid = "rerail-facts-lib"  # their tests are in the lib binary, which runs whole
        d = rd / "mutants" / f"{n}-{cid}"
    if cid in UNIT:
        ok, _, det = check_unit(cid, d, exes, env)
        (d / "failed-tests.txt").write_text("\n".join(det.get("failed", [])) + "\n")
    elif cid == "b-generated":
        ok, _, det = gate.check_b(d, env, "greplay", fail_fast=True, limit=T0_EDITS)
    elif cid == "b-ties":
        ok, _, det = gate.check_ties(d, env)
    else:
        return "n/a", 0.0
    return ("survived" if ok else ("killed" if det.get("detected") else "error")), time.monotonic() - t0


# ---------------------------------------------------------------------------------------------------
# T1 checks


def by_clause(results: dict) -> dict[str, dict[str, list]]:
    """A `facts-a repo` run's findings and failures per `check/clause` (sorted): findings `[path, line,
    code]`, failures `[path, message]`; failures of no clause (a file, a compile refusal) under `check/`."""
    out: dict[str, dict[str, list]] = {}
    for check, clause, path, line, code in results["clause_findings"]:
        out.setdefault(f"{check}/{clause}", {"findings": [], "failures": []})["findings"].append([path, line, code])
    for f in results["failures"]:
        out.setdefault(f"{f['check']}/{f['clause']}", {"findings": [], "failures": []})["failures"].append([f["path"], f["message"]])
    for v in out.values():
        v["findings"].sort()
        v["failures"].sort()
    return dict(sorted(out.items()))


def multiset_diff(a: list, b: list) -> tuple[list, list]:
    """(rows only in a, rows only in b) as multisets."""
    ca = collections.Counter(json.dumps(x) for x in a)
    cb = collections.Counter(json.dumps(x) for x in b)
    return [json.loads(x) for x in (ca - cb).elements()], [json.loads(x) for x in (cb - ca).elements()]


def check_sub200(d: Path, args) -> tuple[bool, str, dict]:
    """`facts-a repo` over sub200 (T1_THREADS workers, the memo consulted) against the committed
    reference: per clause, findings and failures equal as multisets. `--rebase NOTE` rewrites the
    reference from this run instead, NOTE kept in its history."""
    status, _ = gate.run([gate.binary("facts-a"), "repo", "--threads", T1_THREADS, "--root", str(SUB200), "--out-dir", str(d)], d)
    if status != 0 or not (d / "results.json").exists():
        return False, f"exit {status}", {}
    results = json.loads((d / "results.json").read_text())
    got = by_clause(results)
    nf = sum(len(v["findings"]) for v in got.values())
    nx = sum(len(v["failures"]) for v in got.values())
    memo = results.get("memo", {})
    memo_note = f"; memo {memo['hits']}/{memo['lookups']} clause runs memoised" if memo.get("lookups") else ""
    if args.rebase:
        old = json.loads(REFERENCE.read_text()) if REFERENCE.exists() else {"history": []}
        ref = {"format": "rerail.facts-sub200/v1", "root": str(SUB200), "files": results["files"],
               "history": old["history"] + [{"commit": gate.commit(), "date": datetime.date.today().isoformat(), "note": args.rebase,
                                             "findings": nf, "failures": nx}],
               "clauses": got}
        REFERENCE.write_text(json.dumps(ref, indent=1, ensure_ascii=False) + "\n")
        return True, f"REBASED {REFERENCE.relative_to(ROOT)}: {nf} findings, {nx} failures in {len(got)} clauses ({args.rebase}){memo_note}", {}
    if not REFERENCE.exists():
        return False, f"no reference at {REFERENCE.relative_to(ROOT)} (just facts-t1 --rebase NOTE)", {}
    want = json.loads(REFERENCE.read_text())["clauses"]
    diffs = {}
    for c in sorted(set(got) | set(want)):
        g, w = got.get(c, {"findings": [], "failures": []}), want.get(c, {"findings": [], "failures": []})
        fo, fr = multiset_diff(g["findings"], w["findings"])
        xo, xr = multiset_diff(g["failures"], w["failures"])
        if fo or fr or xo or xr:
            diffs[c] = {"findings_run_only": fo[:10], "findings_reference_only": fr[:10], "failures_run_only": xo[:10], "failures_reference_only": xr[:10],
                        "counts": [len(fo), len(fr), len(xo), len(xr)]}
    (d / "difference.json").write_text(json.dumps(diffs, indent=1, ensure_ascii=False))
    wf = sum(len(v["findings"]) for v in want.values())
    summary = (f"{nf}/{wf} findings, {nx} failures, {results['files']} files; "
               + ("equal to the reference in every clause" if not diffs else
                  f"{len(diffs)} clauses DIFFER: " + "; ".join(f"{c} +{v['counts'][0]}/-{v['counts'][1]} findings +{v['counts'][2]}/-{v['counts'][3]} failures"
                                                               for c, v in list(diffs.items())[:5]))
               + memo_note)
    return not diffs, summary, {"differences": diffs}


def ref50_root() -> Path:
    """REF50's files hard-linked from the corpus; rebuilt when the list changes."""
    paths = [l for l in REF50.read_text().splitlines() if l.strip()]
    digest = hashlib.sha256("\n".join(paths).encode()).hexdigest()
    stamp = REF50_ROOT / ".list-sha256"
    if stamp.exists() and stamp.read_text().strip() == digest:
        return REF50_ROOT
    if REF50_ROOT.exists():
        subprocess.run(["rm", "-rf", str(REF50_ROOT)], check=True)
    for p in paths:
        dst = REF50_ROOT / p
        dst.parent.mkdir(parents=True, exist_ok=True)
        os.link(gate.CORPUS / p, dst)
    stamp.write_text(digest + "\n")
    return REF50_ROOT


def check_ref50(d: Path) -> tuple[bool, str, dict]:
    """`facts-a stage-check` over REF50: every relational stage the engine stores equals
    rerail_facts::reference's, under the gate's caps."""
    cmd = [gate.binary("facts-a"), "stage-check", "--out-dir", str(d), "--root", str(ref50_root()), "--checks", ",".join(gate.all_checks()),
           "--max-nodes", gate.REFERENCE_MAX_NODES, "--max-input-rows", gate.REFERENCE_MAX_INPUT_ROWS, "--threads", T1_THREADS]
    status, _ = gate.run(cmd, d)
    if status != 0 or not (d / "results.json").exists():
        return False, f"exit {status}", {}
    r = json.loads((d / "results.json").read_text())
    cor = r["corpus"]
    ok = r["mismatch_total"] == 0 and cor["stages_compared"] > 0
    first = cor["mismatches"][:1]
    return ok, (f"{r['mismatch_total']} mismatches; {r['files']} files: {cor['stages_compared']} stages ({cor['rows_compared']} rows) in "
                f"{cor['clauses_compared']} clauses; {sum(cor['not_compared'].values())} not compared (by reason in results.json)"
                + (f"; first: {first[0].get('clause')} stage {first[0].get('stage')}" if first else "")), {"mismatches": first}


# ---------------------------------------------------------------------------------------------------
# The archetype bench


ARCHETYPES = ROOT / "benchmark/facts/archetypes.json"
# facts-a's counts (spikes/facts-a/src/counts.rs), algebraic first.
ALGEBRAIC = ["leaf_rows", "stage_rows", "fix_rounds", "fix_rows", "match_nodes", "match_roots", "input_rows"]
PHYSICAL = ["index_builds", "index_rows", "range_builds", "range_rows", "probes", "rows_scanned", "rows_joined", "rows_out", "subquery_runs"]
# facts-b's (FACTS_PROFILE, gen/prof.rs): records sent and arranged by its dataflow operators.
DATAFLOW = ["nodes", "records_sent", "records_arranged"]


def timing(d: Path) -> tuple[float, float]:
    """(wall, CPU) seconds from a run directory's `/usr/bin/time -l` output."""
    for line in (d / "stderr.txt").read_text(errors="replace").splitlines():
        w = line.split()
        if len(w) >= 6 and w[1] == "real" and w[3] == "user":
            return float(w[0]), float(w[2]) + float(w[4])
    return 0.0, 0.0


def bench_one(a: dict, d: Path) -> dict:
    """One archetype's run: its counts (algebraic, then physical), findings, failures, wall and CPU."""
    row = {"archetype": a["name"], "check": a["check"], "clause": a["clause"]}
    if a.get("engine") == "greplay":
        d.mkdir(parents=True, exist_ok=True)
        spec = d / "spec.json"
        spec.write_text(json.dumps([{"rule": str(ROOT / a["rule"]), "clauses": [a["clause"]], "exports": os.path.expanduser(a["exports"])}]))
        status, _ = gate.run([gate.binary("facts-b"), "greplay", "--plans", "none", "--real", str(spec), "--no-oracle", "--limit", str(a["events"]),
                              "--events", str(gate.EVENTS), "--run-dir", str(d / "run")], d, {"FACTS_PROFILE": "1"})
        prof = d / "run" / "profile.json"
        if status != 0 or not prof.exists():
            return {**row, "error": f"exit {status}"}
        nodes = json.loads(prof.read_text())["nodes"]
        row["counts"] = {"nodes": len(nodes), "records_sent": sum(n["sent"] for n in nodes), "records_arranged": sum(n["arranged"] for n in nodes)}
        row["events"] = a["events"]
    else:
        status, _ = gate.run([gate.binary("facts-a"), "repo", "--threads", "1", "--no-memo", "--checks", a["check"], "--clauses", a["clause"],
                              "--root", str(SUB200), "--out-dir", str(d)], d)
        if status != 0 or not (d / "results.json").exists():
            return {**row, "error": f"exit {status}"}
        r = json.loads((d / "results.json").read_text())
        row["counts"] = next(c["counts"] for rule in r["rules"] for c in rule["clauses"] if c["clause"] == a["clause"])
        row["findings"] = sum(1 for f in r["clause_findings"] if f[1] == a["clause"])
        row["failures"] = sum(1 for f in r["failures"] if f["clause"] == a["clause"])
    row["wall_s"], row["cpu_s"] = timing(d)
    return row


def count_text(c: dict) -> str:
    keys = [k for k in ALGEBRAIC + PHYSICAL + DATAFLOW if k in c]
    return " ".join(f"{k}={c[k]}" for k in keys)


def delta_text(c: dict, base: dict) -> str:
    """The counts that differ from `base`'s, `k a->b (+x%)`."""
    out = []
    for k in [k for k in ALGEBRAIC + PHYSICAL + DATAFLOW if k in c or k in base]:
        a, b = base.get(k, 0), c.get(k, 0)
        if a != b:
            out.append(f"{k} {a}->{b}" + (f" ({(b - a) / a * 100:+.1f}%)" if a else ""))
    return ", ".join(out) or "no count differs"


def run_bench(rd: Path, names: list[str], serial: bool) -> tuple[bool, list[dict]]:
    spec = json.loads(ARCHETYPES.read_text())["archetypes"]
    unknown = [n for n in names if n not in {a["name"] for a in spec}]
    if unknown:
        raise SystemExit(f"facts-loop: unknown archetypes {unknown}; known: {[a['name'] for a in spec]}")
    chosen = [a for a in spec if not names or a["name"] in names]
    with concurrent.futures.ThreadPoolExecutor(max_workers=1 if serial else len(chosen)) as pool:
        rows = list(pool.map(lambda a: bench_one(a, rd / a["name"]), chosen))
    (rd / "bench.json").write_text(json.dumps({"format": "rerail.facts-bench/v1", "commit": gate.commit(), "env": {k: v for k, v in os.environ.items() if k.startswith("FACTS_")},
                                               "serial": serial, "archetypes": rows}, indent=1))
    return all("error" not in r for r in rows), rows


def bench_line(r: dict, base: dict | None) -> str:
    if "error" in r:
        return f"bench {r['archetype']} {r['check']}/{r['clause']} FAIL {r['error']}"
    head = f"bench {r['archetype']} {r['check']}/{r['clause']}" + (f" findings {r['findings']} failures {r['failures']}" if "findings" in r else f" events {r['events']}")
    if base is not None:
        return f"{head} | vs base: {delta_text(r['counts'], base['counts'])} | wall {base['wall_s']:.1f}s->{r['wall_s']:.1f}s (not calibrated)"
    return f"{head} | {count_text(r['counts'])} | wall {r['wall_s']:.1f}s cpu {r['cpu_s']:.1f}s (not calibrated)"


def check_bench(d: Path) -> tuple[bool, str, dict]:
    """T1's bench: every archetype, counts first."""
    d.mkdir(parents=True, exist_ok=True)
    ok, rows = run_bench(d, [], serial=False)
    for r in rows:
        say(bench_line(r, None))
    return ok, f"{len(rows)} archetypes, counts in {d}/bench.json", {"archetypes": rows}


def bench(args) -> int:
    """`just facts-bench [archetype…] [--compare RUN_DIR]`: the archetypes' counts, then wall-clock;
    an A/B is two runs (the second with `--compare` the first's run directory)."""
    rd = Path(args.run_dir) if args.run_dir else RUNS / (datetime.datetime.now().strftime("%Y%m%dT%H%M%S") + "-bench")
    rd.mkdir(parents=True, exist_ok=True)
    say(f"run {rd} commit {gate.commit()} load {gate.load1()} env {' '.join(f'{k}={v}' for k, v in sorted(os.environ.items()) if k.startswith('FACTS_')) or '-'}")
    if not args.no_build:
        ok, info, _ = build(rd)
        if not ok:
            say(f"FAILED: build failed, see {rd}/build*")
            return 1
    base = {r["archetype"]: r for r in json.loads((Path(args.compare) / "bench.json").read_text())["archetypes"]} if args.compare else {}
    t = time.monotonic()
    ok, rows = run_bench(rd, args.archetypes, args.serial)
    for r in rows:
        say(bench_line(r, base.get(r["archetype"]) if args.compare else None))
    say(f"{'passed' if ok else 'FAILED'} {time.monotonic() - t:.0f}s: {len(rows)} archetypes, {rd}/bench.json")
    return 0 if ok else 1


# ---------------------------------------------------------------------------------------------------
# Tiers


def start(tier: str, args) -> tuple[Path, dict, float]:
    rd = Path(args.run_dir) if args.run_dir else RUNS / (datetime.datetime.now().strftime("%Y%m%dT%H%M%S") + f"-{tier}")
    rd.mkdir(parents=True, exist_ok=True)
    say(f"run {rd} commit {gate.commit()} load {gate.load1()}")
    report = {"format": "rerail.facts-loop/v1", "tier": tier, "commit": gate.commit(), "run_dir": str(rd), "argv": sys.argv[1:],
              "load_before": gate.load1(), "checks": {}, "mutants": []}
    return rd, report, time.monotonic()


def finish(rd: Path, report: dict, started: float) -> int:
    secs = time.monotonic() - started
    checks_ok = [c for c, v in report["checks"].items() if v["ok"]]
    failing = [c for c, v in report["checks"].items() if not v["ok"]] + [f"mutant {m['n']}" for m in report["mutants"] if not m["killed"]]
    killed = sum(1 for m in report["mutants"] if m["killed"])
    report.update({"passed": not failing, "seconds": round(secs, 1), "load_after": gate.load1()})
    (rd / "loop.json").write_text(json.dumps(report, indent=1, ensure_ascii=False))
    say(f"{'passed' if not failing else 'FAILED'} {secs:.0f}s: {len(checks_ok)}/{len(report['checks'])} checks pass, "
        f"{killed}/{len(report['mutants'])} mutants killed" + (f"; failing: {', '.join(failing)}" if failing else ""))
    return 0 if not failing else 1


def record(report: dict, cid: str, d: Path, t0: float, result: tuple[bool, str, dict]) -> None:
    ok, summary, details = result
    secs = time.monotonic() - t0
    report["checks"][cid] = {"ok": ok, "seconds": round(secs, 1), "summary": summary, "details": details, "dir": str(d)}
    say(f"check {cid} {'pass' if ok else 'FAIL'} {secs:.0f}s {summary}")


def built(rd: Path, report: dict, args, started: float) -> dict[str, str] | None:
    """The loop build (or the last one with --no-build): the unit-test executables, or None when the
    build failed (reported)."""
    if args.no_build:
        return json.loads((LOOP / "last-build.json").read_text())["tests"]
    ok, info, exes = build(rd)
    report["build"] = info
    if not ok:
        say(f"FAILED {time.monotonic() - started:.0f}s: build failed, see {rd}/build*")
        report["passed"] = False
        (rd / "loop.json").write_text(json.dumps(report, indent=1))
        return None
    (LOOP / "last-build.json").write_text(json.dumps({"tests": exes}))
    return exes


def t0(args) -> int:
    rd, report, started = start("t0", args)
    exes = built(rd, report, args, started)
    if exes is None:
        return 1
    base = merge_base(args.base)
    changed = changed_files(base)
    sites = mutant_sites()
    mutants, problems = all_mutants()
    chosen = [m for m in mutants if sites.get(m[0], set()) & set(changed)]
    site_files = sorted({f for m in chosen for f in sites[m[0]]})
    report.update({"base": {"ref": args.base, "merge_base": base}, "changed": changed, "registry_problems": problems})
    say(f"changed {len(changed)} files since {base[:9]} ({args.base}); mutant sites in {', '.join(site_files) or 'none'}")
    for p in problems:
        say(f"registry FAIL {p}")
        report["checks"].setdefault("rewrite-registry", {"ok": False, "seconds": 0, "summary": p, "details": {}, "dir": ""})
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        checks = {cid: (time.monotonic(), pool.submit(check_unit, cid, rd / cid, exes, {})) for cid in UNIT}
        for cid, (t, fut) in checks.items():
            try:
                res = fut.result()
            except Exception as e:  # a harness error is a failed check
                res = (False, f"loop error: {e!r}", {})
            record(report, cid, rd / cid, t, res)
        cells = {(n, cid): pool.submit(mutant_cell, n, cid, rd, exes) for (n, name, crate, site, what, tests) in chosen for cid in tests}
        for (n, name, crate, site, what, tests) in chosen:
            row, secs = {}, 0.0
            for cid in tests:
                try:
                    row[cid], s = cells[(n, cid)].result()
                    secs += s
                except Exception as e:
                    row[cid] = "error"
                    (rd / "mutants").mkdir(exist_ok=True)
                    (rd / "mutants" / f"{n}-{cid}.error").write_text(repr(e))
            row = {c: v for c, v in row.items() if v != "n/a"}
            killed = any(v == "killed" for v in row.values())
            report["mutants"].append({"n": n, "name": name, "crate": crate, "site": site, "breaks": what, "files": sorted(sites[n]),
                                      "killed": killed, "seconds": round(secs, 1), "cells": row})
            say(f"mutant {n} {name} {'killed' if killed else 'SURVIVED'} {secs:.0f}s " + " ".join(f"{c}={v}" for c, v in row.items()))
    return finish(rd, report, started)


def t1(args) -> int:
    rd, report, started = start("t1", args)
    if built(rd, report, args, started) is None:
        return 1
    base = merge_base(args.base)
    changed = changed_files(base)
    facts_b = [f for f in changed if f.startswith("spikes/facts-b/")]
    report.update({"base": {"ref": args.base, "merge_base": base}, "changed": changed})
    say(f"changed {len(changed)} files since {base[:9]} ({args.base}); facts-b {'changed' if facts_b else 'unchanged'}")
    jobs: dict[str, tuple] = {"sub200-parity": (check_sub200, args), "ref50-vs-reference": (check_ref50,), "bench": (check_bench,)}
    if facts_b or args.replay:
        jobs["b-exactness"] = (lambda d: gate.check_b(d, {}, "replay", limit=T1_EDITS),)
        jobs["b-generated"] = (lambda d: gate.check_b(d, {}, "greplay", limit=T1_EDITS),)
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(jobs)) as pool:
        futs = {cid: (time.monotonic(), pool.submit(f, rd / cid, *rest)) for cid, (f, *rest) in jobs.items()}
        for cid, (t, fut) in futs.items():
            try:
                res = fut.result()
            except Exception as e:  # a harness error is a failed check
                res = (False, f"loop error: {e!r}", {})
            record(report, cid, rd / cid, t, res)
    return finish(rd, report, started)


def main() -> int:
    ap = argparse.ArgumentParser(prog="facts-loop.py", description="The facts engine's loop tiers (docs/facts/LOOP.md).")
    sub = ap.add_subparsers(dest="tier", required=True)
    p0 = sub.add_parser("t0", help="unit tests and the changed files' mutants (<= 60 s)")
    p1 = sub.add_parser("t1", help="sub200 parity, 50 files against the reference, the edit replays when facts-b changed, the archetype bench (<= 2 min)")
    p1.add_argument("--rebase", metavar="NOTE", help="rewrite benchmark/facts/sub200-reference.json from this run, recording NOTE (why the findings changed)")
    p1.add_argument("--replay", action="store_true", help=f"run the {T1_EDITS}-event edit replays even when facts-b is unchanged")
    pb = sub.add_parser("bench", help="the archetype bench: counts first, wall second")
    pb.add_argument("archetypes", nargs="*", help="archetype names (default: all of benchmark/facts/archetypes.json)")
    pb.add_argument("--compare", metavar="RUN_DIR", help="print each archetype's count differences from that bench run (an A/B)")
    pb.add_argument("--serial", action="store_true", help="one archetype at a time (wall-clock with less contention)")
    for p in (p0, p1, pb):
        p.add_argument("--no-build", action="store_true", help="reuse the last loop build")
        p.add_argument("--run-dir", help=f"run directory (default {RUNS}/<ts>-<tier>)")
    for p in (p0, p1):
        p.add_argument("--base", default=os.environ.get("FACTS_LOOP_BASE", BASE), help=f"merge-base ref (default {BASE})")
        p.add_argument("--jobs", type=int, default=8, help="concurrent checks and mutant cells (default 8)")
    try:
        args = ap.parse_args()
    except SystemExit as e:
        return 2 if e.code else 0
    LOOP.mkdir(parents=True, exist_ok=True)
    return {"t0": t0, "t1": t1, "bench": bench}[args.tier](args)


if __name__ == "__main__":
    sys.exit(main())
