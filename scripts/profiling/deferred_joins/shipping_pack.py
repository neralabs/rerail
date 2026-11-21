"""Compare automatic deferred joins with the control on the shipping manifest's Python corpus."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import tempfile

from corpus import ROOT, RunFailed, bounded_run, native


def verify_sources(root, spec):
    # Limit traversal to the expected sample trees even if given an unrelated directory.
    tools = {p.parent.name for p in (ROOT / "packs/python/upstream").glob("*/samples.lock")}
    if {p.name for p in root.iterdir()} != tools:
        raise ValueError(f"expected the shipping corpus with entries {sorted(tools)}")
    digest = hashlib.sha256()
    files = sorted(p for p in root.rglob("*") if p.is_file())
    if len(files) != spec["files"]:
        raise ValueError(f"manifest expects {spec['files']} files, found {len(files)}")
    for path in files:
        content = hashlib.sha256(path.read_bytes()).hexdigest()
        digest.update(f"{path.relative_to(root).as_posix()}\0{content}\n".encode())
    if digest.hexdigest() != spec["sha256"]:
        raise ValueError("the Python corpus digest differs from the shipping manifest")


def check_rules(engine, policies, out, rss_limit_mib, deadline_s):
    # The shipping harness runs check-rr once and records a reason for every unavailable policy.
    with tempfile.TemporaryDirectory(dir=out) as scratch:
        work = Path(scratch)
        env = {k: v for k, v in os.environ.items() if not k.startswith("RERAIL_")}
        env.update(RERAIL_THREADS="4", RERAIL_TAPE_CACHE=str(work / "check.sqlite"),
                   RERAIL_RR_LIB=str(ROOT / "docs/rule-syntax/lib"), TMPDIR=scratch)
        bounded_run([engine, "check-rr", *policies], work, env, rss_limit_mib, deadline_s, 8 * 1024**3, accepted_codes=(0, 1))
        rows = json.loads((work / "report.json").read_text())
    if sorted(r["path"] for r in rows) != sorted(str(p) for p in policies):
        raise ValueError("check-rr did not report each shipped policy exactly once")
    excluded = {Path(r["path"]).stem: dict(status=r["status"], error=r.get("error"))
                for r in rows if r["status"] != "reader-passed"}
    return [p for p in policies if p.stem not in excluded], excluded


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("engine", "manifest", "corpus", "out"):
        parser.add_argument(f"--{field}", type=Path, required=True)
    parser.add_argument("--rss-mib", type=int, default=8192)
    parser.add_argument("--seconds", type=int, default=900)
    parser.add_argument("--no-reach-index", action="store_true", help="use recursive rules for reachability in both modes")
    args = parser.parse_args()
    if min(args.rss_mib, args.seconds) <= 0:
        parser.error("RSS and time limits must be positive")
    manifest = json.loads(args.manifest.read_text())
    spec = manifest["corpora"][manifest["packs"]["python"]]
    root = args.corpus.resolve()
    verify_sources(root, spec)
    policies = [ROOT / "docs/rule-syntax/packs/python" / (p.stem + ".rerail")
                for p in sorted((ROOT / "packs/python/rules").glob("*.json"))]
    if not policies or any(not p.is_file() for p in policies):
        raise ValueError("the full shipped Python pack requires a .rerail file for every rule")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    summary = dict(corpus=spec["sha256"], files=spec["files"], rules=len(policies),
                   rss_limit_mib=args.rss_mib, deadline_s=args.seconds, reach_index=not args.no_reach_index, runs={})
    policies, excluded = check_rules(args.engine.resolve(), policies, out, args.rss_mib, args.seconds)
    summary.update(runnable=len(policies), excluded=excluded)
    print(json.dumps(dict(runnable=len(policies), excluded=excluded), sort_keys=True), flush=True)
    (out / "comparison.json").write_text(json.dumps(summary, indent=2) + "\n")
    if not policies:
        raise ValueError("check-rr rejected every rule")
    for mode in ("off", "auto"):
        # Release each cold store before starting the other run. Keep both diagnostic reports.
        reserve = 8 * 1024**3
        if shutil.disk_usage(out).free < reserve:
            raise RuntimeError("fewer than 8 GiB free on the benchmark volume")
        with tempfile.TemporaryDirectory(dir=out) as scratch:
            trial = Path(scratch)
            try:
                result = native(args.engine.resolve(), root, trial / "run", trial / "store.sqlite",
                                mode=mode, policy=policies, strict_pack=True, reach_index=not args.no_reach_index, rss_limit_mib=args.rss_mib,
                                deadline_s=args.seconds, min_free_bytes=reserve)
                runs = json.loads((trial / "run/report.json").read_text())["runs"]
                result["policies_run_together"] = sum(r["together"] > 1 for r in runs)
                result["largest_group"] = max(r["together"] for r in runs)
                summary["runs"][mode] = result
            except RuntimeError as err:
                summary["runs"][mode] = dict(stopped=str(err), **(err.metrics if isinstance(err, RunFailed) else {}))
            evidence = out / mode
            evidence.mkdir()
            for name in ("report.json", "stderr.log"):
                path = trial / "run" / name
                if path.is_file():
                    path.replace(evidence / name)
            print(json.dumps({mode: summary["runs"][mode]}, sort_keys=True), flush=True)
            (out / "comparison.json").write_text(json.dumps(summary, indent=2) + "\n")
    control, auto = (summary["runs"][mode] for mode in ("off", "auto"))
    if "sha256" not in control or "sha256" not in auto:
        raise SystemExit("incomplete comparison; see comparison.json")
    if (control["diagnostics"], control["sha256"]) != (auto["diagnostics"], auto["sha256"]):
        raise SystemExit("the canonical findings differ")
    print(f"All {len(policies)} rules agree on {auto['diagnostics']} findings.")


if __name__ == "__main__":
    def stop(signum, _frame):
        # Unwind bounded_run so a disconnected farm client leaves no engine process behind.
        raise SystemExit(128 + signum)

    for sig in (signal.SIGHUP, signal.SIGTERM):
        signal.signal(sig, stop)
    main()
