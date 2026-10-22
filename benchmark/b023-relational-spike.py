#!/usr/bin/env python3
"""Run the test-only B023 query against frozen corpus inputs and two output oracles.

Usage: python3 benchmark/b023-relational-spike.py workload.json /absolute/new/output
The caller supplies build environment overrides, including JEMALLOC_OVERRIDE if required.
This measures one rule's matching kernel, not the complete policy job.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, help="Use an existing test executable")
    parser.add_argument("--samples", type=int, default=3)
    args = parser.parse_args()
    if args.samples < 1:
        parser.error("--samples must be positive")
    root = Path(__file__).resolve().parent.parent
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    environment = os.environ.copy()
    for name in ("RERAIL_SQL_PROFILE_DIR", "RERAIL_SQL_TRACE_DIR", "RERAIL_SQL_TRACE_FILTER"):
        environment.pop(name, None)
    environment["RUST_MIN_STACK"] = "67108864"
    binary = args.binary
    if binary is None:
        command = ["cargo", "test", "-p", "rerail-engine", "--profile", "loop", "--features",
                   "unbounded-work", "--no-run", "--message-format=json"]
        with (out / "build.jsonl").open("w") as stdout, (out / "build.stderr").open("w") as stderr:
            subprocess.run(command, cwd=root, env=environment, stdout=stdout, stderr=stderr, check=True)
        artifacts = [json.loads(line) for line in (out / "build.jsonl").read_text().splitlines()]
        binary = Path(next(item["executable"] for item in artifacts
                           if item.get("reason") == "compiler-artifact" and item.get("executable")
                           and item["target"]["name"] == "rerail_engine"))
    binary = binary.resolve()
    pinned = {path: digest(path) for path in (
        binary, Path(__file__), root / "native/src/code_rule_dsl/sql/spikes/whole_call/b023.rs",
        root / "native/src/code_rule_dsl/sql/spikes/whole_call/b023.sql",
        root / "packs/python/rules/ruff-b023-function-uses-loop-variable.json",
    )}
    with (out / "fixtures.log").open("w") as log:
        subprocess.run([str(binary), "b023_direct_", "--nocapture", "--test-threads=1"],
                       cwd=root, env=dict(environment, RERAIL_B023_SCALE_OUTPUT=str(out / "scaling.json")),
                       stdout=log, stderr=log, check=True, timeout=600)
    original = json.loads(args.config.read_text())
    corpus = Path(original["corpus"])
    config = dict(original)
    config["corpus"] = str(out / "corpus")
    fingerprints = []
    for relative in original["files"]:
        relative_path = Path(relative)
        if relative_path.is_absolute() or ".." in relative_path.parts:
            raise ValueError(f"Corpus path must stay inside the corpus: {relative}")
        target = out / "corpus" / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(corpus / relative_path, target)
        fingerprints.append({"path": relative, "sha256": digest(target)})
    config_path = out / "config.json"
    config_path.write_text(json.dumps(config, indent=2) + "\n")
    environment["RERAIL_B023_CONFIG"] = str(config_path)

    def execute(label, profiling=False):
        destination = out / label
        env = dict(environment, RERAIL_B023_OUTPUT=str(destination))
        if profiling:
            env["RERAIL_SQL_PROFILE_DIR"] = str(destination / "raw")
        with (out / f"{label}.log").open("w") as log:
            subprocess.run([str(binary), "b023_direct_corpus", "--ignored", "--nocapture",
                            "--test-threads=1"], cwd=root, env=env, stdout=log, stderr=log,
                           check=True, timeout=600)
        result = json.loads((destination / "results.json").read_text())
        if not result["parity"]:
            raise AssertionError(f"Parity failed: {label}")
        if result["inputs"] != fingerprints:
            raise AssertionError(f"Corpus changed: {label}")
        return result

    profile = execute("profile", profiling=True)
    events = []
    for path in (out / "profile" / "raw").glob("profile-*.jsonl"):
        events.extend(json.loads(line) for line in path.read_text().splitlines())
    sql_events = [event for event in events if event.get("kind") == "sql"]
    if not sql_events:
        raise AssertionError("Missing current-engine SQL instrumentation")
    baseline_vm = sum(event["counters"][0] for event in sql_events)
    baseline_executions = len(sql_events)
    samples = [execute(f"sample-{index}") for index in range(args.samples)]
    for sample in samples:
        if sample["querySha256"] != profile["querySha256"] or sample["policySha256"] != profile["policySha256"]:
            raise AssertionError("Query or policy changed during the experiment")
    baseline_cpu = statistics.median(s["baseline"]["coldKernelCpuNs"] for s in samples)
    direct_cpu = statistics.median(s["direct"]["coldKernelCpuNs"] for s in samples)
    direct_vm = profile["direct"]["metrics"]["vmSteps"]
    for path, expected in pinned.items():
        if digest(path) != expected:
            raise AssertionError(f"Experiment source or executable changed: {path}")
    report = {
        "scope": "One complete B023 matching kernel across all configured files; not the full policy job",
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "testExecutableSha256": digest(binary),
        "querySha256": profile["querySha256"], "policySha256": profile["policySha256"],
        "harnessSha256": digest(root / "native/src/code_rule_dsl/sql/spikes/whole_call/b023.rs"),
        "driverSha256": digest(Path(__file__)),
        "files": len(fingerprints), "inputs": fingerprints,
        "sourceBytes": profile["sourceBytes"], "matches": profile["matches"],
        "parity": True, "samples": samples,
        "scaling": json.loads((out / "scaling.json").read_text()),
        "environment": {key: environment[key] for key in ("JEMALLOC_OVERRIDE", "RUST_MIN_STACK", "LIBSQLITE3_FLAGS") if key in environment},
        "measurement": "Fresh test process per sample; current engine precedes direct SQL; frame counters and SQL profiling disabled in timing samples; parsing is reported separately; SQL generation and connection setup included; artifact IO and diagnostic queries excluded",
        "baseline": {"vmSteps": baseline_vm, "sqlExecutions": baseline_executions,
                     "medianColdKernelCpuNs": baseline_cpu, "counters": profile["baseline"]["counters"]},
        "direct": {"vmSteps": direct_vm, "sqlExecutions": profile["direct"]["metrics"]["sqlExecutions"],
                   "medianColdKernelCpuNs": direct_cpu, "counters": profile["direct"]["counters"],
                   "metrics": profile["direct"]["metrics"], "relationRows": profile["diagnosticRelationRows"]},
        "cpuRatio": baseline_cpu / direct_cpu, "vmRatio": baseline_vm / direct_vm,
        "tenfoldVmTargetPassed": baseline_vm >= 10 * direct_vm,
    }
    (out / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("files", "matches", "parity", "cpuRatio", "vmRatio", "tenfoldVmTargetPassed")}, indent=2))


if __name__ == "__main__":
    main()
