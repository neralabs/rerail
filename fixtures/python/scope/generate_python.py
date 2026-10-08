"""Writes <fixture>.expected.json for every python/*.py here: the references Ruff reports as undefined (F821), as byte
offsets, at --target-version py313 or at the version a first line `# target-version: pyXY` names. Run offline, not in
CI: python3 generate_python.py <path to a ruff executable>."""

from __future__ import annotations

import builtins
import json
import pathlib
import subprocess
import sys

MODULE_NAMES: frozenset[str] = frozenset(
    {
        "__annotations__",
        "__builtins__",
        "__class__",
        "__dict__",
        "__doc__",
        "__file__",
        "__loader__",
        "__module__",
        "__name__",
        "__package__",
        "__path__",
        "__qualname__",
        "__spec__",
    }
)


def byte_offset(lines: list[str], location: dict[str, int]) -> int:
    row: int = location["row"]
    column: int = location["column"]
    return len(("".join(lines[: row - 1]) + lines[row - 1][: column - 1]).encode())


def expected_output(ruff: str, source_path: pathlib.Path) -> dict[str, object]:
    source: str = source_path.read_text()
    lines: list[str] = source.splitlines(keepends=True)
    encoded: bytes = source.encode()
    header: str = lines[0].removeprefix("# target-version: ").strip() if lines and lines[0].startswith("# target-version: ") else ""
    target: str = header or "py313"
    command: list[str] = [ruff, "check", "--isolated", "--no-cache", "--target-version", target, "--select", "F821"]
    report = subprocess.run([*command, "--output-format", "json", str(source_path)], capture_output=True, text=True, check=False)
    unresolved: list[dict[str, object]] = []
    for item in json.loads(report.stdout):
        if item["code"] != "F821":
            raise SystemExit(f"{source_path}: {item['message']}")
        start: int = byte_offset(lines, item["location"])
        end: int = byte_offset(lines, item["end_location"])
        unresolved.append({"start": start, "name": encoded[start:end].decode()})
    version: str = subprocess.run([ruff, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    expected: dict[str, object] = {
        "oracle": f"ruff F821 ({version}, --target-version {target})",
        "builtins": sorted(set(dir(builtins)) | MODULE_NAMES),
        "unresolved": sorted(unresolved, key=lambda item: int(str(item["start"]))),
    }
    if header:
        expected["targetVersion"] = header
    return expected


def main() -> None:
    ruff: str = sys.argv[1]
    root: pathlib.Path = pathlib.Path(__file__).parent / "python"
    for source_path in sorted(root.glob("*.py")):
        expected: dict[str, object] = expected_output(ruff, source_path)
        source_path.with_suffix(".expected.json").write_text(json.dumps(expected, indent=1) + "\n")


if __name__ == "__main__":
    main()
