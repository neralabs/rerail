#!/usr/bin/env python3
"""Profile the compiler-generated target-slot utility over a frozen workload.

This is transition phase 1's first slice, not the complete B023 rule or policy job.
Timings include counters. This driver makes no repeated-timing speedup claim.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


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
    config["corpus"] = str(out / "corpus")
    (out / "config.json").write_text(json.dumps(config, indent=2) + "\n")
    env.update(RERAIL_B023_CONFIG=str(out / "config.json"),
               RERAIL_RELATIONAL_OUTPUT=str(out / "results.json"),
               RERAIL_RELATIONAL_SCALE_OUTPUT=str(out / "scaling.json"))
    for label, selection, extra in [("semantics", "sql::relational::tests::", []),
                                     ("fixtures", "b023_compiler_target_slot", []),
                                     ("corpus", "b023_compiler_target_slot_corpus", ["--ignored"])]:
        with (out / f"{label}.log").open("w") as log:
            subprocess.run([str(binary), selection, *extra, "--nocapture", "--test-threads=1"],
                           cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    result = json.loads((out / "results.json").read_text())
    assert result["parity"] and result["framePublications"] == 0 and result["matchingStatements"] == 1
    assert result["inputs"] == fingerprints
    for path, sha in pinned.items():
        assert digest(path) == sha, f"Artifact changed during the run: {path}"
    (out / "query.sql").write_text(result["sql"] + "\n")
    evidence = {
        "scope": __doc__.strip(),
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "workingTree": subprocess.check_output(["git", "status", "--short"], cwd=root, text=True),
        "sourceHashes": {str(p.relative_to(root)): pinned[p] for p in sources},
        "executableSha256": pinned[binary],
        "querySha256": digest(out / "query.sql"),
        "result": result,
        "scaling": json.loads((out / "scaling.json").read_text()),
    }
    (out / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("files", "matches", "vmSteps", "framePublications", "matchingStatements")}))


if __name__ == "__main__":
    main()
