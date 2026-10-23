#!/usr/bin/env python3
"""Profile nested reached traversal and typed child returns against native semantics.

The deterministic workloads live in outcomes/nested_tests.rs. These measurements cover matching
kernels, not whole-job execution budgets or native work/stack/depth limits.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, RUST_MIN_STACK="67108864")
    for name in list(env):
        if name.startswith("RERAIL_"):
            env.pop(name)
    binary = args.binary
    if binary is None:
        with (output / "build.jsonl").open("w") as stdout, (output / "build.stderr").open("w") as stderr:
            subprocess.run(["cargo", "test", "-p", "rerail-engine", "--profile", "loop",
                            "--features", "unbounded-work", "--no-run", "--message-format=json"],
                           cwd=root, env=env, stdout=stdout, stderr=stderr, check=True)
        artifacts = [json.loads(line) for line in (output / "build.jsonl").read_text().splitlines()]
        binary = Path(next(a["executable"] for a in artifacts
                           if a.get("reason") == "compiler-artifact" and a.get("executable")
                           and a["target"]["name"] == "rerail_engine"))
    binary = binary.resolve()
    sources = sorted((root / "native/src/code_rule_dsl/sql/relational").rglob("*.rs"))
    sources += [root / "native/src/code_rule_dsl/matcher/mod.rs",
                root / "native/src/code_rule_dsl/matcher/value.rs",
                root / "native/src/code_rule_dsl/matcher/value_probe.rs",
                root / "native/src/code_rule_dsl/sql/terminals/mod.rs", Path(__file__).resolve()]
    pinned = {path: digest(path) for path in [binary, *sources]}
    env["RERAIL_RELATIONAL_NESTED_EVIDENCE"] = str(output / "results.json")
    with (output / "tests.log").open("w") as log:
        subprocess.run([str(binary), "sql::relational::", "--nocapture", "--test-threads=1"],
                       cwd=root, env=env, stdout=log, stderr=log, check=True, timeout=600)
    result = json.loads((output / "results.json").read_text())
    for path, sha in pinned.items():
        assert digest(path) == sha, f"Artifact changed during run: {path}"
    for sample in result["samples"]:
        assert sample["parity"] and sample["matchingStatements"] == 1
        assert sample["framePublications"] == 0
        (output / f'{sample["dimension"]}-{sample["size"]}.sql').write_text(sample["sql"] + "\n")
    for sample in result["samples"]:
        relations = sample["relations"]["relations"]
        counts = {r["relation"]: r["rows"] for r in sample["diagnosticRelationRows"]}
        roots = counts[sample["relations"]["outcomeDemand"]]
        execution = sample["prepareAndExecuteCpuNs"] - sample["prepareCpuNs"]
        traversal_rows = sum(counts[r["relation"]] for r in relations if r["operator"] in ("ordered reached traversal", "nested ordered traversal"))
        assert traversal_rows % 3 == 0
        def children(relations):
            for relation in relations:
                yield relation
                for child in relation.get("childPrograms", {}).values():
                    if child:
                        yield from children(child["relations"])
        all_relations = list(children(relations))
        flags = []
        if sample["prepareCpuNs"] > max(1_000_000, 2 * max(execution, 1)):
            flags.append("Preparation dominates execution: inspect repeated CTE dependencies and scalar projection boundaries.")
        if sample["automaticIndexRows"] > 8 * max(roots, 1):
            flags.append("Intermediate-key indexes exceed eight entries per root: inspect joins back to demand relations.")
        sample["summary"] = {
            "rootDemands": roots, "relationCount": len(relations),
            "generatedRelationsIncludingChildScopes": len(all_relations),
            "largestNestedRecursiveWidth": max((r.get("recursiveColumns", 0) for r in all_relations), default=0), "sqlBytes": len(sample["sql"]),
            "executionCpuNs": execution,
            "localCandidateCtes": len(re.findall(r"candidate[0-9]+ AS MATERIALIZED", sample["sql"])),
            "vmStepsPerVisitedOuterCandidate": sample["vmSteps"] / max(traversal_rows // 3, 1),
            "captureExports": len(sample["relations"]["captures"]),
            "largestDiagnosticRelationRows": max(counts.values(), default=0),
            "outerTraversalRows": traversal_rows,
            "typedReturnRows": sample["typedReturnRows"],
            "typedReturnCells": sample["typedReturnCells"],
            "typedReturnScope": "Physical integer/NULL cursor inputs, measured before diagnostic queries. No AST access, rule evaluation, or nested SQL in the cursor.",
            "visitedOuterCandidatesDerived": traversal_rows // 3,
            "derivation": "Three logical states per visited outer candidate. Inner child cardinalities are not measured by these top-level diagnostic queries. These counts are not physical writes.",
            "candidateRows": sum(counts[r["relation"]] for r in relations if r["operator"] == "candidate demand"),
            "diagnosticRowsAcrossRelations": sum(counts.values()),
            "cardinalityScope": "Logical outputs from separate diagnostic queries; not physical writes or memory usage.",
            "flags": flags,
            "admissionNotes": sample["relations"]["flags"],
        }
    fault_traces=[json.loads(line.split("REACHED_TRACE ",1)[1]) for line in (output/"tests.log").read_text().splitlines() if "REACHED_TRACE " in line]
    assert fault_traces and all(t["matchingStatements"]==1 for t in fault_traces)
    evidence = {
        "reachedFailureTraces": fault_traces,
        "scope": __doc__.strip(),
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "workingTree": subprocess.check_output(["git", "status", "--short"], cwd=root, text=True),
        "sourceHashes": {str(path.relative_to(root)): pinned[path] for path in sources},
        "executableSha256": pinned[binary],
        "result": result,
        "notes": ["SQL owns all composition; native helpers evaluate atomic value, pattern, or capture-equality leaves.",
                  "Diagnostic CTE cardinalities run separately from matching and can change query plans.",
                  "Fallible node-local walks use ordered recursive traversal; total structural walks retain set-based plans.",
                  "Node-local capture masks admit at most 61 slots. Nested traversal carries ten control columns plus observable child exports.",
                  "Nested local materializations are reported separately from top-level relation count.",
                  "Typed child results cross a single row-value subquery. SQL owns all rule composition; relation_row only copies integer/NULL columns.",
                  "Ordered traversal nesting is admitted through depth four; general recursion and full-file publication remain pending."],
    }
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps([{
        key: sample[key] for key in ("dimension", "size", "matches", "vmSteps", "matchingStatements",
                                    "framePublications", "valueRequests", "captureEqualityRequests", "typedReturnRows", "typedReturnCells", "summary")
    } for sample in result["samples"]], indent=2))


if __name__ == "__main__":
    main()
