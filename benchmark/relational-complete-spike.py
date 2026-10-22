#!/usr/bin/env python3
"""Measure the complete compiler-generated B023 rule on the frozen reference corpus.

Both queries use fresh connections and identical execution instrumentation. Parsing is separate.
This test-only milestone does not establish the complete-job 100-execution gate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import statistics


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, RUST_MIN_STACK="67108864")
    for key in ("RERAIL_SQL_PROFILE_DIR", "RERAIL_SQL_TRACE_DIR", "RERAIL_SQL_TRACE_FILTER"):
        env.pop(key, None)
    binary = args.binary
    if binary is None:
        with (out / "build.jsonl").open("w") as stdout, (out / "build.stderr").open("w") as stderr:
            subprocess.run(["cargo", "test", "-p", "rerail-engine", "--profile", "loop",
                            "--features", "unbounded-work", "--no-run", "--message-format=json"],
                           cwd=root, env=env, stdout=stdout, stderr=stderr, check=True)
        artifacts = [json.loads(line) for line in (out / "build.jsonl").read_text().splitlines()]
        binary = Path(next(a["executable"] for a in artifacts
                           if a.get("reason") == "compiler-artifact" and a.get("executable")
                           and a["target"]["name"] == "rerail_engine"))
    binary = binary.resolve()
    sources = sorted((root / "native/src/code_rule_dsl/sql/relational").glob("*.rs"))
    sources += [root / "native/src/code_rule_dsl/sql/spikes/whole_call/b023.rs",
                root / "native/src/code_rule_dsl/sql/spikes/whole_call/b023.sql",
                root / "native/src/code_rule_dsl/sql/spikes/whole_call/run.rs",
                root / "native/src/code_rule_dsl/sql/mod.rs",
                root / "packs/python/rules/ruff-b023-function-uses-loop-variable.json", Path(__file__).resolve()]
    pinned = {p: digest(p) for p in [binary, *sources]}
    config = json.loads(args.config.read_text())
    fingerprints = []
    for file in config["files"]:
        relative = Path(file)
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"Corpus path escapes its root: {file}")
        target = out / "corpus" / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(Path(config["corpus"]) / relative, target)
        fingerprints.append({"path": file, "sha256": digest(target)})
    reference_manifest = root / "benchmark/sql-b023-relational-evidence.json"
    assert fingerprints == json.loads(reference_manifest.read_text())["inputs"], \
        "The milestone gate requires the frozen reference corpus"
    config["corpus"] = str(out / "corpus")
    (out / "config.json").write_text(json.dumps(config, indent=2) + "\n")
    env.update(RERAIL_B023_CONFIG=str(out / "config.json"),
               RERAIL_COMPLETE_OUTPUT=str(out / "results.json"),
               RERAIL_COMPLETE_SCALING=str(out / "scaling.json"))
    selections = [("semantics", "sql::relational::", []),
                  ("fixtures", "b023_compiler_", []),
                  ("adversarial", "b023_direct_adversarial", []),
                  ("corpus", "b023_compiler_complete_corpus", ["--ignored"])]
    for label, selection, extra in selections:
        with (out / f"{label}.log").open("w") as log:
            subprocess.run([str(binary), selection, *extra, "--nocapture", "--test-threads=1"],
                           cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    result = json.loads((out / "results.json").read_text())
    assert result["parity"] and result["framePublications"] == 0
    assert result["matchingStatements"] == 1
    samples = result["samples"]
    for sample in samples:
        for shape in ("generated", "reference"):
            report = sample[shape]
            assert report["parity"] and report["matches"] == result["matches"]
            assert report["direct"]["metrics"]["sqlExecutions"] == 1
            assert report["direct"]["counters"]["framePublications"] == 0
            assert report["inputs"] == fingerprints
    generated = statistics.median(s["generated"]["direct"]["coldKernelCpuNs"] for s in samples)
    reference = statistics.median(s["reference"]["direct"]["coldKernelCpuNs"] for s in samples)
    vm = [s["generated"]["direct"]["metrics"]["vmSteps"] for s in samples]
    result["gates"] = {"maxVmSteps": 4649592, "generatedVmSteps": vm,
                       "generatedMedianColdKernelCpuNs": generated,
                       "referenceMedianColdKernelCpuNs": reference,
                       "cpuRatio": generated / reference,
                       "vmPassed": max(vm) <= 4649592, "cpuPassed": generated <= 2 * reference}
    assert result["inputs"] == fingerprints
    for path, sha in pinned.items():
        assert digest(path) == sha, f"Artifact changed during the run: {path}"
    (out / "query.sql").write_text(result["sql"] + "\n")
    evidence = {"scope": __doc__.strip(),
                "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
                "workingTree": subprocess.check_output(["git", "status", "--short"], cwd=root, text=True),
                "sourceHashes": {str(p.relative_to(root)): pinned[p] for p in sources},
                "executableSha256": pinned[binary], "querySha256": digest(out / "query.sql"),
                "result": result, "scaling": json.loads((out / "scaling.json").read_text())}
    (out / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({"files": result["files"], "matches": result["matches"], **result["gates"]}))
    assert result["gates"]["vmPassed"], "Full-query VM gate failed; evidence retained"
    assert result["gates"]["cpuPassed"], "Full-query CPU gate failed; evidence retained"


if __name__ == "__main__":
    main()
