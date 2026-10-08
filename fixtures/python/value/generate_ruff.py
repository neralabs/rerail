"""Writes ruff.expected.json: where Ruff reports each rule on its fixtures ruff/<CODE>.py and ruff/<CODE>-<topic>.py, as
byte offsets keyed by the fixture's stem. The value rules F601, B033, PIE796, SIM222 and SIM223 read literals' values:
F601 and B033 by hash (1, 1.0 and True collide), PIE796 structurally, SIM222 and SIM223 by Ruff's syntactic
truthiness. The structure fixtures pin compare's structural modes: ComparableExpr (PIE796, SIM101, SIM114),
HashableExpr (F601, B033) and ComparableLiteral (SIM116). SIM910, SIM911, ASYNC230 and B019 read bindings: a name's
value, annotation or parameter kind, and a base class's own bases. M9b's fix round adds unpacked values (ASYNC230,
SIM113), lookup_symbol (ASYNC220), only_binding (ASYNC240), find_argument (B905, ASYNC115) and node identity (SIM113); binds.bindings counts SIM113's counter bindings in the
loop (a comprehension walrus, a match capture and a del count; a comprehension target, a keyword and a nested
function's local do not). M12b's E712-each and PIE810-each keep repeated offsets: Ruff reports one diagnostic per
offending operand or per group of calls at the same range (violation.each); W605-each one per invalid escape in an
f-string's format spec, several in one node's text.
Run offline, not in CI:
python3 generate_ruff.py <path to a ruff executable, 0.16.9>"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

HERE: pathlib.Path = pathlib.Path(__file__).parent


def byte_offset(lines: list[str], row: int, column: int) -> int:
    return len(("".join(lines[: row - 1]) + lines[row - 1][: column - 1]).encode())


def main() -> None:
    ruff: str = sys.argv[1]
    version: str = subprocess.run([ruff, "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]
    results: dict[str, object] = {"ruff": version}
    for path in sorted((HERE / "ruff").glob("*.py")):
        code: str = path.stem.split("-")[0]
        lines: list[str] = path.read_text().splitlines(keepends=True)
        output: str = subprocess.run(
            [ruff, "check", "--isolated", "--no-cache", "--select", code, "--output-format", "json", str(path)],
            capture_output=True,
            text=True,
            check=False,
        ).stdout
        reports: list[dict[str, object]] = json.loads(output)
        results[path.stem] = sorted(
            byte_offset(lines, int(report["location"]["row"]), int(report["location"]["column"]))  # type: ignore[index]
            for report in reports
            if report["code"] == code
        )
    (HERE / "ruff.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
