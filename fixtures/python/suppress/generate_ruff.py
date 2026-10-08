"""Writes ruff.expected.json: Ruff 0.16.9's `# noqa` matrix. Each fixture ruff/<name>.py is checked twice, with
`--ignore-noqa` (every diagnostic) and without (the ones its `# noqa`, `# ruff: noqa` and `# flake8: noqa` comments
leave), keyed `<name>` then `all` or `kept`, each diagnostic as [code, start byte, end byte]. The fixtures sit in a
directory without `__init__.py`, so INP001 reports each file that has no shebang at byte 0. Run offline, not in CI,
from anywhere:
python3 generate_ruff.py <path to a ruff executable, 0.16.9>"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

HERE: pathlib.Path = pathlib.Path(__file__).parent
CODES: list[str] = ["F401", "F403", "F541", "F811", "F822", "F841", "E741", "W605", "SIM300", "INP001", "EXE003"]


def byte_offset(source: bytes, row: int, column: int) -> int:
    """Ruff's row and column (1-based, characters) as a byte offset, with Ruff's line breaks."""
    text: str = source.decode()
    start: int = 0
    for _ in range(row - 1):
        breaks: list[int] = [at for at in (text.find("\r", start), text.find("\n", start)) if at >= 0]
        at: int = min(breaks)
        start = at + (2 if text[at : at + 2] == "\r\n" else 1)
    return len(text[:start].encode()) + len(text[start : start + column - 1].encode())


def check(ruff: str, path: pathlib.Path, honour: bool) -> list[list[object]]:
    source: bytes = path.read_bytes()
    flags: list[str] = [] if honour else ["--ignore-noqa"]
    output: str = subprocess.run(
        [ruff, "check", "--isolated", "--no-cache", *flags, "--select", ",".join(CODES), "--output-format", "json", path.relative_to(HERE).as_posix()],
        capture_output=True,
        text=True,
        check=False,
        cwd=HERE,
    ).stdout
    return sorted(
        [
            str(report["code"]),
            byte_offset(source, int(report["location"]["row"]), int(report["location"]["column"])),
            byte_offset(source, int(report["end_location"]["row"]), int(report["end_location"]["column"])),
        ]
        for report in json.loads(output)
    )


def main() -> None:
    ruff: str = sys.argv[1]
    version: str = subprocess.run([ruff, "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]
    results: dict[str, object] = {"ruff": version}
    for path in sorted((HERE / "ruff").glob("*.py")):
        results[path.name] = {"all": check(ruff, path, False), "kept": check(ruff, path, True)}
    (HERE / "ruff.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
