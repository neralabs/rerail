#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Inventory data-layer checks without importing or running the source repository.

python3 scripts/dsl/data_layer_rule_audit.py --repo /path/to/nera-data-layer

Optional oracle probes require the dependencies of the supplied data-layer checkout:
  /path/to/venv/bin/python scripts/dsl/data_layer_rule_audit.py \
    --repo /path/to/nera-data-layer --oracle-root /path/to/pinned/source

The inventory counts LinterConfig declarations, not individual diagnostic codes.
Source signals describe references in the configured script, not proven DSL gaps.
"""

from __future__ import annotations

import argparse
import ast
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys


CURRENT = "2b1504adc4291daaf080b2e932962b134da1a69a"
HISTORICAL = "281f500d7c8d413f17c6a27de2f3c518bb45258a"
ROOT = Path(__file__).resolve().parents[2]


def fixtures_path(*parts: str) -> Path:
    """The shared Python resolver for repository-owned fixture data."""
    override = os.environ.get("RERAIL_ORACLES_DIR")
    root = Path(override) if override else ROOT / "fixtures"
    if not root.is_absolute():
        root = ROOT / root
    return root.joinpath(*parts)


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True)


def literal_keywords(node: ast.Call) -> dict:
    values = {}
    for keyword in node.keywords:
        if keyword.arg is None:
            continue
        try:
            values[keyword.arg] = ast.literal_eval(keyword.value)
        except (ValueError, TypeError):
            pass
    return values


def inventory(repo: Path, revision: str) -> dict:
    commit = git(repo, "rev-parse", "--verify", f"{revision}^{{commit}}").strip()
    tree = ast.parse(git(repo, "show", f"{commit}:scripts/lint_all.py"))
    paths = set(git(repo, "ls-tree", "-r", "--name-only", commit, "scripts").splitlines())
    configs = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name):
            continue
        if node.func.id != "LinterConfig":
            continue
        config = literal_keywords(node)
        if "name" not in config:
            raise ValueError(f"Nonliteral LinterConfig name at line {node.lineno}")
        config["declaration_line"] = node.lineno
        config.setdefault("enabled", True)
        path = "scripts/" + config["script"]
        config["script_exists"] = path in paths
        if path in paths:
            source = git(repo, "show", f"{commit}:{path}")
            config["script_sha256"] = hashlib.sha256(source.encode()).hexdigest()
            # These signals guide source review. Helpers and registered factories can
            # add requirements that the configured entry point does not mention.
            signals = {
                "symbol_graph": ("symbol_graph", "build_symbol_graph"),
                "project_facts": ("project_input_facts", 'workload_scope="project"'),
                "policy_artifacts": ("ratchet", "signature", "allowlist", "baseline"),
                "ast_collectors": ("AstCollectorSpec", "AstEventCollector"),
            }
            config["source_signals"] = [
                name for name, needles in signals.items() if any(s in source for s in needles)
            ]
        configs.append(config)
    configs.sort(key=lambda item: item["name"])
    names = [item["name"] for item in configs]
    if len(names) != len(set(names)):
        raise ValueError("Duplicate LinterConfig names require manual interpretation")
    return {
        "commit": commit,
        "declared_checks": len(configs),
        "enabled_declarations": sum(item["enabled"] for item in configs),
        "enforcement_counts": dict(sorted(Counter(item["enforcement"] for item in configs).items())),
        "checks": configs,
    }


def oracle_probes(root: Path, expected_commit: str) -> list[dict]:
    # A .git-less archive is valid, but a checkout must be at the inventoried revision.
    if (root / ".git").exists():
        actual = git(root, "rev-parse", "HEAD").strip()
        if actual != expected_commit or git(root, "status", "--porcelain", "--", "scripts"):
            raise ValueError("Oracle checkout must have clean scripts at the current inventory commit")
    # An archive can lack Git metadata. Check every archived script against Git before
    # calling this mode, as described in the report.
    sys.path.insert(0, str(root))
    from scripts.lint_lib.fact_engine import materialize_source_facts
    from scripts.lint_plan_scope_side_effects import (
        PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR,
        plan_scope_side_effects_fact_extractors,
    )

    probes = json.loads(fixtures_path("host", "code-rule-dsl", "data-layer-port-probes.json").read_text())
    # The existing example's regex is in the common subset of Python and Rust regex.
    # This compares its coverage, not its parser or performance.
    import re

    example = json.loads(fixtures_path("host", "code-rule-dsl", "example-data-layer-plan-scope-side-effects.json").read_text())
    terms = example["clauses"][0]["forbid"]["ast"]["rule"]["all"]
    call_regex = re.compile(terms[1]["regex"])
    with_regex = re.compile(terms[2]["inside"]["regex"])
    results = []
    for case in probes:
        path = root / case["path"]
        bundle = materialize_source_facts(
            path=path,
            source=case["source"],
            specs=(PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR,),
            extractors=plan_scope_side_effects_fact_extractors(root),
        )
        if bundle.errors:
            raise RuntimeError(bundle.errors)
        findings = bundle.facts[PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id]
        observed = sorted(finding["code"] for finding in findings)
        if observed != sorted(case["expected_codes"]):
            raise AssertionError(f"{case['id']}: {observed} != {case['expected_codes']}")
        # These probes contain exactly one with statement and one expression.
        # Deliberately do not generalize this text projection to arbitrary Python.
        projection = bool(with_regex.search(case["source"]) and call_regex.search(case["source"]))
        if projection != case["existing_example_regex_projection"]:
            raise AssertionError(f"Unexpected example projection: {case['id']}")
        results.append({"id": case["id"], "oracle_codes": observed, "example_regex_projection": projection})
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--current", default=CURRENT)
    parser.add_argument("--historical", default=HISTORICAL)
    parser.add_argument("--oracle-root", type=Path)
    args = parser.parse_args()
    current = inventory(args.repo, args.current)
    historical = inventory(args.repo, args.historical)
    now = {item["name"]: item for item in current["checks"]}
    before = {item["name"]: item for item in historical["checks"]}
    common = now.keys() & before.keys()
    output = {
        "format": "rerail.data-layer-rule-audit/v1",
        "current": current,
        "historical": historical,
        "added_checks": sorted(now.keys() - before.keys()),
        "removed_checks": sorted(before.keys() - now.keys()),
        "common_checks": len(common),
        "common_checks_with_changed_entry_script": sorted(
            name for name in common if now[name].get("script_sha256") != before[name].get("script_sha256")
        ),
    }
    if args.oracle_root is not None:
        # Refuse archives with modified or missing scripts, including transitive helpers.
        for path in git(args.repo, "ls-tree", "-r", "--name-only", current["commit"], "scripts").splitlines():
            if not path.endswith(".py"):
                continue
            if (args.oracle_root / path).read_text() != git(args.repo, "show", f"{current['commit']}:{path}"):
                raise ValueError(f"Oracle source differs from pinned commit: {path}")
        output["oracle_probes"] = oracle_probes(args.oracle_root.resolve(), current["commit"])
    print(json.dumps(output, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
