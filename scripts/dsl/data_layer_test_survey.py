#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Map pinned data-layer suite entries to tests without importing the repository.

python3 scripts/dsl/data_layer_test_survey.py --repo /path/to/data-layer \
  --inventory /tmp/data-layer-rule-audit.json > /tmp/data-layer-test-survey.json

Associations are import evidence, not certified behavioral coverage. Test counts
are function definitions before pytest collection and parameter expansion.
"""

from __future__ import annotations

import argparse
import ast
from collections import Counter, defaultdict
import io
import json
from pathlib import Path
import subprocess
import tarfile

from data_layer_rule_audit import fixtures_path


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True)


def imports(tree: ast.AST) -> set[str]:
    modules = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom):
            module = node.module or ""
            if module == "scripts":
                modules.update("scripts." + alias.name for alias in node.names)
            else:
                modules.add("scripts." + module if module.startswith("lint_") else module)
        elif isinstance(node, ast.Import):
            modules.update("scripts." + alias.name if alias.name.startswith("lint_") else alias.name for alias in node.names)
    return modules


def signals(test: ast.AST) -> list[str]:
    names = {node.id for node in ast.walk(test) if isinstance(node, ast.Name)}
    attributes = {node.attr for node in ast.walk(test) if isinstance(node, ast.Attribute)}
    result = []
    for name, targets in {
        "filesystem_fixture": {"tmp_path", "tmpdir", "TemporaryDirectory"},
        "monkeypatch_or_mock": {"monkeypatch", "mocker", "patch", "MagicMock", "Mock"},
        "output_contract": {"capsys", "capfd", "caplog"},
        "parameterized": {"parametrize"},
        "visitor_or_ast": {"visit", "parse", "walk"},
        "cli_or_subprocess": {"main", "cli_main", "subprocess", "argv", "run_suite"},
        "synthetic_graph": {"_FakeGraph", "FakeGraph", "_FakeSymbol", "FakeSymbol"},
    }.items():
        if (names | attributes) & targets:
            result.append(name)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--batches", type=Path, default=fixtures_path("host", "data-layer-port", "batches.json"))
    args = parser.parse_args()
    inventory = json.loads(args.inventory.read_text())["current"]
    revision = inventory["commit"]
    batches = json.loads(args.batches.read_text())
    planned = [name for batch in batches["batches"] for name in batch["checks"]]
    known = {check["name"] for check in inventory["checks"]}
    sizes = [len(batch["checks"]) for batch in batches["batches"]]
    expected_sizes = [10] * (len(known) // 10) + ([len(known) % 10] if len(known) % 10 else [])
    if batches["source_commit"] != revision or set(planned) != known or len(planned) != len(known) or sizes != expected_sizes:
        raise ValueError("Batch plan must cover the pinned inventory exactly once, in groups of ten and a final remainder")
    check_batch = {name: batch["id"] for batch in batches["batches"] for name in batch["checks"]}
    source = git(args.repo, "show", f"{revision}:scripts/lint_all.py")
    suite = ast.parse(source)
    by_module = defaultdict(set)
    for check in inventory["checks"]:
        by_module["scripts." + Path(check["script"]).stem].add(check["name"])
    imported_names = {}
    for node in ast.walk(suite):
        if isinstance(node, ast.ImportFrom) and node.module:
            for alias in node.names:
                imported_names[alias.asname or alias.name] = node.module
    # Factory modules matter where the configured script is a facade or alias.
    for node in ast.walk(suite):
        if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name) or len(node.args) < 2:
            continue
        if node.func.id not in {"_plugin", "_rootless_plugin", "_plugin_group", "_facts"}:
            continue
        try:
            names = ast.literal_eval(node.args[0])
        except (ValueError, TypeError):
            continue
        target = node.args[1]
        if isinstance(target, ast.Name) and target.id in imported_names:
            by_module[imported_names[target.id]].update([names] if isinstance(names, str) else names)
        elif isinstance(names, str) and isinstance(target, ast.Lambda):
            for value in ast.walk(target):
                if isinstance(value, ast.Constant) and isinstance(value.value, str) and value.value.endswith(".py"):
                    by_module["scripts." + Path(value.value).stem].add(names)

    paths = git(args.repo, "ls-tree", "-r", "--name-only", revision).splitlines()
    candidates = [p for p in paths if p.endswith(".py") and Path(p).name.startswith("test_")]
    # One archive avoids a separate Git process for every test module.
    outside_scripts = [path for path in candidates if not path.startswith("scripts/")]
    data = subprocess.check_output(["git", "-C", str(args.repo), "archive", revision, "scripts", *outside_scripts])
    modules = []
    errors = []
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        # Several suite registrations point at a facade whose implementation lives
        # in another linter module. Follow only linter-to-linter imports, not the
        # shared framework or product imports.
        dependencies = {}
        for entry in archive.getmembers():
            if entry.isfile() and entry.name.startswith("scripts/lint_") and entry.name.count("/") == 1 and entry.name.endswith(".py"):
                tree = ast.parse(archive.extractfile(entry).read().decode("utf-8"))
                dependencies["scripts." + Path(entry.name).stem] = {
                    module for module in imports(tree)
                    if module.startswith("scripts.lint_") and module.count(".") == 1 and module != "scripts.lint_lib"
                }
        changed = True
        while changed:
            changed = False
            for module, children in dependencies.items():
                for child in children:
                    added = by_module.get(module, set()) - by_module[child]
                    if added:
                        by_module[child].update(added)
                        changed = True
        for entry in archive:
            if not entry.isfile() or entry.name not in candidates:
                continue
            try:
                tree = ast.parse(archive.extractfile(entry).read().decode("utf-8"), filename=entry.name)
            except (SyntaxError, UnicodeDecodeError) as error:
                errors.append({"path": entry.name, "error": str(error)})
                continue
            imported = imports(tree)
            imported |= {"scripts." + module for module in imported if "scripts." + module in by_module}
            script_references = {
                "scripts." + Path(value.value).stem
                for node in ast.walk(tree) if isinstance(node, (ast.Assign, ast.AnnAssign)) and node.value is not None
                for value in ast.walk(node.value) if isinstance(value, ast.Constant)
                and isinstance(value.value, str) and value.value.endswith(".py")
                and "\n" not in value.value and len(value.value) < 200
            }
            checks = sorted({check for module in imported | script_references for check in by_module.get(module, ())})
            if not checks:
                continue
            tests = [node for node in ast.walk(tree) if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith("test_")]
            modules.append({
                "path": entry.name,
                "checks": checks,
                "imported_check_modules": sorted(module for module in imported if by_module.get(module)),
                "referenced_check_scripts": sorted(module for module in script_references if by_module.get(module)),
                "tests": [{"name": node.name, "line": node.lineno, "signals": signals(node)} for node in tests],
            })
    modules.sort(key=lambda item: item["path"])
    per_check = []
    for check in inventory["checks"]:
        associated = [module for module in modules if check["name"] in module["checks"]]
        per_check.append({
            "name": check["name"],
            "batch": check_batch[check["name"]],
            "test_modules": [module["path"] for module in associated],
            "associated_test_definitions": sum(len(module["tests"]) for module in associated),
        })
    counts = Counter(signal for module in modules for test in module["tests"] for signal in test["signals"])
    print(json.dumps({
        "format": "rerail.data-layer-test-survey/v1",
        "commit": revision,
        "batch_sizes": sizes,
        "scanned_test_modules": len(candidates),
        "associated_test_modules": len(modules),
        "associated_test_definitions": sum(len(module["tests"]) for module in modules),
        "test_signal_counts": dict(sorted(counts.items())),
        "unassociated_checks": [check["name"] for check in per_check if not check["test_modules"]],
        "parse_errors": errors,
        "checks": per_check,
        "modules": modules,
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
