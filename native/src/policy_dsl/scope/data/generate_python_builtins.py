"""Writes python_builtins.json: for each Python minor version Ruff targets, the names Ruff resolves at module level
without a binding (its builtins and magic globals, F821-silent). Run offline, not in CI:
python3 generate_python_builtins.py <ruff 0.16.9 executable> <ruff checkout>/crates/ruff_python_stdlib/src/builtins.rs"""

from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys
import tempfile

PIN: str = "ruff 0.16.9"
MINORS: range = range(7, 16)
# Module attributes Ruff binds in every module; builtins.rs does not list them.
MODULE_NAMES: list[str] = ["__doc__", "__loader__", "__name__", "__package__", "__path__", "__spec__"]


def main() -> None:
    ruff: str = sys.argv[1]
    version: str = subprocess.run([ruff, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    if version != PIN:
        raise SystemExit(f"{version} found; the table is pinned to {PIN}")
    literals: set[str] = set(re.findall(r'"([A-Za-z_][A-Za-z0-9_]*)"', pathlib.Path(sys.argv[2]).read_text()))
    candidates: list[str] = sorted(literals | set(MODULE_NAMES))
    versions: dict[str, list[str]] = {}
    with tempfile.TemporaryDirectory() as scratch:
        probe: pathlib.Path = pathlib.Path(scratch) / "probe.py"
        probe.write_text("".join(f"{name}\n" for name in candidates))
        for minor in MINORS:
            command: list[str] = [ruff, "check", "--isolated", "--no-cache", "--target-version", f"py3{minor}"]
            report = subprocess.run(
                [*command, "--select", "F821", "--output-format", "json", str(probe)], capture_output=True, text=True, check=False
            )
            undefined: set[str] = {candidates[item["location"]["row"] - 1] for item in json.loads(report.stdout)}
            versions[f"3.{minor}"] = [name for name in candidates if name not in undefined]
    table: dict[str, object] = {"source": f"{version} F821 at module level, per --target-version", "versions": versions}
    (pathlib.Path(__file__).parent / "python_builtins.json").write_text(json.dumps(table, indent=1) + "\n")


if __name__ == "__main__":
    main()
