#!/usr/bin/env python3
"""The new engine's coding rules, run by the old engine (docs/rule-syntax/packs/rerail-engine/).

Each rule is a `.rerail` policy over the Rust of decision 0686's crates (blocks, delta_core, delta_exec,
delta_device, delta_twins): the requirements AGENTS.md, the decisions and the user's rules set for that code, as
syntax the rule engine reads. This script runs them with the old engine's `rerail-engine`:

1. `test-rr` on every rule: each rule's own tests (`.test` ... `.end`), its caught and passing cases;
2. `run-policies` over this checkout: every finding, by rule.

A rule in Steer mode holds on the tree today, and a finding of one fails the check (exit 1). A rule in Log mode
names code that still breaks it: its findings are counted, and listed with --list. Run from anywhere in the
checkout:

    python3 scripts/check/check_engine_rules.py [--list] [RULE...]

The engine is `$RERAIL_ENGINE`, else the checkout's `target/debug/rerail-engine` (`cargo build --bin
rerail-engine --features test-hooks`, or a farm build fetched with `scripts/dev/test-remote --fetch
debug/rerail-engine -- …`). It runs at nice 19, in a few seconds.
"""
import collections
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
RULES = ROOT / "docs/rule-syntax/packs/rerail-engine"


def engine() -> str:
    path = os.environ.get("RERAIL_ENGINE") or str(ROOT / "target/debug/rerail-engine")
    if not os.access(path, os.X_OK):
        sys.exit(f"check_engine_rules: no engine at {path}: build it (cargo build --bin rerail-engine --features "
                 "test-hooks) or name one with RERAIL_ENGINE")
    return path


def run(args: list) -> subprocess.CompletedProcess:
    # The engine reads core's library from this checkout (the rules' own library is here), whichever tree built it.
    env = {**os.environ, "RERAIL_SOURCE_ROOT": str(ROOT)}
    return subprocess.run(["nice", "-n", "19", engine(), *args], cwd=ROOT, env=env, capture_output=True, text=True)


def mode(path: pathlib.Path) -> str:
    m = re.search(r"^\.mode\s+(\w+)", path.read_text(), re.M)
    return m.group(1) if m else "log"


def main() -> int:
    listing = "--list" in sys.argv
    names = [a for a in sys.argv[1:] if not a.startswith("--")]
    rules = sorted(p for p in RULES.glob("engine-*.rerail") if not names or p.stem in names)
    if not rules:
        sys.exit(f"check_engine_rules: no rule {names} in {RULES.relative_to(ROOT)}")
    failed = 0
    tests = run(["test-rr", *map(str, rules)])
    for line in tests.stdout.splitlines():
        if line.startswith("FAIL") or line.startswith("    "):
            print(line)
    if tests.returncode != 0:
        print(f"check_engine_rules: the rules' own tests failed\n{tests.stderr.strip()}")
        failed = 1
    out = pathlib.Path(tempfile.mkdtemp(prefix="rerail-engine-rules-"))
    try:
        found = run(["run-policies", ".", str(out), *map(str, rules)])
    finally:
        shutil.rmtree(out, ignore_errors=True)
    if found.returncode != 0:
        print(f"check_engine_rules: run-policies failed\n{found.stderr.strip()}")
        return 1
    by_rule = collections.defaultdict(list)
    for d in json.loads(found.stdout)["diagnostics"]:
        by_rule[d["policy"]].append(d)
    for rule in rules:
        hits = sorted(by_rule.get(rule.stem, []), key=lambda d: (d["path"], d["line"]))
        steer = mode(rule) == "steer"
        print(f"{rule.stem} ({'steer' if steer else 'log'}): {len(hits)}")
        if hits and (steer or listing):
            for d in hits:
                print(f"    {d['path']}:{d['line']}: {d['code']}")
        if hits and steer:
            failed = 1
    return failed


if __name__ == "__main__":
    sys.exit(main())
