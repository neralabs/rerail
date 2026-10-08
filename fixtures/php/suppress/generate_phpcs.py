"""Writes phpcs.expected.json with PHP_CodeSniffer's reports for each suppress/phpcs fixture under
phpcs/ruleset.xml, from PHP_CodeSniffer 4.0.4 (`phpcs4`) and 3.13.6 (`phpcs3`, the last release
that reads the `@codingStandardsIgnore*` annotations 4.0 removed): with annotations ignored
(`--ignore-annotations`, `all`) and honoured (`kept`). Run offline, never in CI:
python3 generate_phpcs.py /tmp/rerail-php-tools/phpcs /tmp/rerail-php-tools/phpcs3

Requires PHP 8.5.11 and squizlabs/php_codesniffer 4.0.4 and 3.13.6, each installed with Composer
in the directory given. Reports are [message code, start byte, end byte], placed at the
reported line and column (PHP_CodeSniffer counts a column in UTF-8 characters; the fixtures have
no tabs, which it would expand)."""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).parent


def offset(source: bytes, line: int, column: int) -> int:
    rows = source.split(b"\n")
    before = rows[line - 1].decode()[: column - 1].encode()
    return sum(len(row) + 1 for row in rows[: line - 1]) + len(before)


def report(phpcs: str, path: pathlib.Path, *flags: str) -> list[list[object]]:
    output = subprocess.run(
        [phpcs, "--report=json", "--no-cache", "-q", f"--standard={HERE / 'phpcs' / 'ruleset.xml'}", *flags, str(path)],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    data = json.loads(output)
    source = path.read_bytes()
    return sorted(
        [
            str(message["source"]),
            offset(source, int(message["line"]), int(message["column"])),
            offset(source, int(message["line"]), int(message["column"])),
        ]
        for messages in data["files"].values()
        for message in messages["messages"]
    )


def main() -> None:
    installs = {"phpcs4": sys.argv[1], "phpcs3": sys.argv[2]}
    results: dict[str, object] = {}
    for key, install in installs.items():
        phpcs = str(pathlib.Path(install) / "vendor/bin/phpcs")
        results[key] = subprocess.run([phpcs, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    for path in sorted((HERE / "phpcs").glob("*.php")):
        results[path.name] = {
            key: {
                "all": report(str(pathlib.Path(install) / "vendor/bin/phpcs"), path, "--ignore-annotations"),
                "kept": report(str(pathlib.Path(install) / "vendor/bin/phpcs"), path),
            }
            for key, install in installs.items()
        }
    (HERE / "phpcs.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
