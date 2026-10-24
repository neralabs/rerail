#!/usr/bin/env python3
"""Run structural SQL reference contracts and preserve every failed gate."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

SUITES = [
    ("spike_stages", None),
    ("continuation_store::tests", None),
    ("sql_depth_spike", None),
    ("cost_relation_spike::", "RERAIL_COST_RELATION_SPIKE_EVIDENCE"),
    ("demand_relation_spike::", "RERAIL_DEMAND_RELATION_SPIKE_EVIDENCE"),
    ("sim905_relation_spike::", "RERAIL_SIM905_RELATION_SPIKE_EVIDENCE"),
    ("sim905_effect_spike::", "RERAIL_SIM905_EFFECT_SPIKE_EVIDENCE"),
    ("sim905_full_spike::", "RERAIL_SIM905_FULL_SPIKE_EVIDENCE"),
    ("parsed_relation_spike::", "RERAIL_PARSED_RELATION_SPIKE_EVIDENCE"),
    ("job_relation_spike::", "RERAIL_JOB_RELATION_SPIKE_EVIDENCE"),
    ("b615_captured_relation_spike::", "RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE"),
    ("b615_binding_boundary_spike::", "RERAIL_B615_BINDING_BOUNDARY_SPIKE_EVIDENCE"),
    ("staged_job_spike::", "RERAIL_STAGED_JOB_SPIKE_EVIDENCE"),
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    sources = sorted((root / "native/src/code_rule_dsl").rglob("*.rs"))
    sources += sorted((root / "native/src/code_rule_dsl").rglob("*.sql"))
    sources += sorted((root / "packs/python/rules").glob("*.json"))
    for tool in ["ruff", "bandit"]:
        sources += sorted(p for p in (root / f"packs/python/upstream/{tool}/fixtures").rglob("*")
                          if p.suffix in [".py", ".pyi"])
    sources += [root / name for name in [
        "Cargo.toml", "Cargo.lock", "native/Cargo.toml", "native/build.rs",
        "rust-toolchain.toml", ".cargo/config.toml",
    ] if (root / name).exists()]
    sources.append(Path(__file__).resolve())
    pins = {str(p.relative_to(root)): digest(p) for p in sources}
    executable = digest(binary)
    listing = subprocess.check_output([str(binary), "--list"], text=True, cwd=root)
    results = []
    for index, (name, evidence) in enumerate(SUITES):
        selected = [line for line in listing.splitlines() if name in line and line.endswith(": test")]
        if not selected:
            results.append({"suite": name, "passed": False, "error": "no registered tests"})
            continue
        stem = f"{index:02d}-{name.split('::')[0]}"
        env = dict(os.environ, RUST_MIN_STACK="67108864")
        for override in ["RERAIL_SIM905_REFERENCE_SQL_OVERRIDE",
                         "RERAIL_PARSED_RELATION_SPIKE_SQL_OVERRIDE"]:
            env.pop(override, None)
        if evidence:
            env[evidence] = str(output / f"{stem}.json")
        if name == "b615_binding_boundary_spike::":
            env["RERAIL_B615_IMPORTED_CALLEE_SPIKE_EVIDENCE"] = str(output / f"{stem}-imported.json")
        with (output / f"{stem}.log").open("w") as log:
            try:
                run = subprocess.run([str(binary), name, "--nocapture", "--test-threads=1"],
                                     cwd=root, env=env, stdout=log, stderr=log, timeout=900)
                result = {"suite": name, "tests": len(selected), "exitCode": run.returncode,
                          "passed": run.returncode == 0, "log": f"{stem}.log"}
            except subprocess.TimeoutExpired:
                result = {"suite": name, "tests": len(selected), "passed": False,
                          "error": "900 second timeout", "log": f"{stem}.log"}
        results.append(result)
        print(json.dumps(result), flush=True)
    changes = [name for name, sha in pins.items() if digest(root / name) != sha]
    report = {"suites": results, "sourceSha256": pins, "binarySha256": executable,
              "sourcesChanged": changes, "binaryUnchanged": executable == digest(binary),
              "passed": all(r["passed"] for r in results) and not changes and executable == digest(binary),
              "scope": "experimental reference contracts, not production compiler qualification"}
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
