"""Writes bandit.expected.json: where Bandit reports its call-argument rules on its own examples, copied unchanged from
the Bandit 1.8.6 source distribution (Apache-2.0) into bandit/, as byte offsets. B113 asks whether `timeout` is passed,
B506 reads `Loader` by keyword or second position, B505 a key size or curve by keyword or position, and B610 binds
`extra`'s six positional slots. Run offline, not in CI, under Python 3.13:
python3 generate_bandit.py <path to a bandit executable, 1.8.6>"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

HERE: pathlib.Path = pathlib.Path(__file__).parent
FIXTURES: dict[str, str] = {
    "B113": "requests-missing-timeout.py",
    "B506": "yaml_load.py",
    "B505": "weak_cryptographic_key_sizes.py",
    "B610": "django_sql_injection_extra.py",
}


def main() -> None:
    bandit: str = sys.argv[1]
    version: str = subprocess.run([bandit, "--version"], capture_output=True, text=True, check=True).stdout.split()[1]
    results: dict[str, object] = {"bandit": version}
    for code, name in FIXTURES.items():
        path: pathlib.Path = HERE / "bandit" / name
        lines: list[bytes] = path.read_bytes().splitlines(keepends=True)
        output: str = subprocess.run(
            [bandit, "-q", "-f", "json", "-t", code, str(path)], capture_output=True, text=True, check=False
        ).stdout
        report: dict[str, list[dict[str, object]]] = json.loads(output)
        if report["errors"]:
            raise SystemExit(f"{name}: {report['errors']}")
        results[code] = sorted(
            len(b"".join(lines[: int(str(item["line_number"])) - 1])) + int(str(item["col_offset"]))
            for item in report["results"]
            if item["test_id"] == code
        )
    (HERE / "bandit.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
