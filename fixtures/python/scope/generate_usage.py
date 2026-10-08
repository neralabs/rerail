"""Writes usage.expected.json: where Ruff reports each Pyflakes usedness and resolution rule on its fixtures
usage/<CODE>.py and usage/<CODE>_<case>.py(i), as byte ranges (start, end), keyed by the fixture's name without `.py`,
and where Pyflakes itself reports the same check on it, by line and name, with the lines on which the two disagree. A
first line `# target-version: pyXY` sets Ruff's target version (py310 otherwise). Run offline, not in CI:
python3 generate_usage.py <path to a ruff executable, 0.16.9> <path to a python with pyflakes 3.4.0>"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

HERE: pathlib.Path = pathlib.Path(__file__).parent
# Each code with the Pyflakes message class it corresponds to.
CODES: dict[str, str] = {
    "F401": "UnusedImport",
    "F841": "UnusedVariable",
    "F821": "UndefinedName",
    "F405": "ImportStarUsage",
    "F811": "RedefinedWhileUnused",
    "F402": "ImportShadowedByLoopVar",
    "F822": "UndefinedExport",
    "F823": "UndefinedLocal",
    "F842": "UnusedAnnotation",
}
PYFLAKES: str = """
import json, sys
import pyflakes
from pyflakes import checker
import ast
source = open(sys.argv[1]).read()
tree = ast.parse(source, filename=sys.argv[1])
messages = checker.Checker(tree, filename=sys.argv[1]).messages
print(json.dumps({"version": pyflakes.__version__, "python": sys.version.split()[0], "messages": [
    {"class": type(message).__name__, "line": message.lineno, "name": str(message.message_args[0])}
    for message in messages
]}))
"""


def byte_offset(lines: list[str], row: int, column: int) -> int:
    return len(("".join(lines[: row - 1]) + lines[row - 1][: column - 1]).encode())


def fixtures(code: str) -> list[pathlib.Path]:
    """The fixtures of a code: <CODE>.py first, then its cases by name."""
    usage: pathlib.Path = HERE / "usage"
    cases: list[pathlib.Path] = sorted(
        path for path in usage.iterdir() if path.name.startswith(f"{code}_") and path.suffix in (".py", ".pyi")
    )
    plain: pathlib.Path = usage / f"{code}.py"
    return ([plain] if plain.exists() else []) + cases


def main() -> None:
    ruff: str = sys.argv[1]
    python: str = sys.argv[2]
    version: str = subprocess.run([ruff, "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]
    results: dict[str, object] = {"ruff": version}
    for code, message_class in CODES.items():
        for path in fixtures(code):
            source: str = path.read_text()
            lines: list[str] = source.splitlines(keepends=True)
            encoded: bytes = source.encode()
            header: str = lines[0].removeprefix("# target-version: ").strip() if lines and lines[0].startswith("# target-version: ") else ""
            target: list[str] = ["--target-version", header] if header else []
            output: str = subprocess.run(
                [ruff, "check", "--isolated", "--no-cache", *target, "--select", code, "--output-format", "json", str(path)],
                capture_output=True,
                text=True,
                check=False,
            ).stdout
            reports: list[dict[str, object]] = [report for report in json.loads(output) if report["code"] == code]
            ranges: list[tuple[int, int]] = sorted(
                (
                    byte_offset(lines, int(report["location"]["row"]), int(report["location"]["column"])),  # type: ignore[index]
                    byte_offset(lines, int(report["end_location"]["row"]), int(report["end_location"]["column"])),  # type: ignore[index]
                )
                for report in reports
            )
            ruff_lines: set[tuple[int, str]] = {
                (encoded[: start].count(b"\n") + 1, encoded[start:end].decode()) for start, end in ranges
            }
            flakes: dict[str, object] = json.loads(
                subprocess.run([python, "-c", PYFLAKES, str(path)], capture_output=True, text=True, check=True).stdout
            )
            flakes_lines: set[tuple[int, str]] = {
                (int(message["line"]), str(message["name"]))  # type: ignore[index]
                for message in flakes["messages"]  # type: ignore[union-attr]
                if message["class"] == message_class  # type: ignore[index]
            }
            results["pyflakes"] = f"pyflakes {flakes['version']} on CPython {flakes['python']}"
            # Pyflakes names imports by what they import and reports `__all__` on its last line, so the lines are
            # compared.
            ruff_rows: set[int] = {line for line, _ in ruff_lines}
            flakes_rows: set[int] = {line for line, _ in flakes_lines}
            results[path.name.removesuffix(".py")] = {
                "ranges": [list(pair) for pair in ranges],
                "onlyRuff": [f"{line}:{name}" for line, name in sorted(ruff_lines) if line not in flakes_rows],
                "onlyPyflakes": [f"{line}:{name}" for line, name in sorted(flakes_lines) if line not in ruff_rows],
            }
    (HERE / "usage.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
