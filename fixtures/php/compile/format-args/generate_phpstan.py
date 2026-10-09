"""Writes phpstan.expected.json: where PHPStan 2.2.16's PrintfParametersRule and
PrintfArrayParametersRule (`argument.sprintf`, `argument.printf`, `argument.vsprintf`,
`argument.vprintf`) report on each format_args/phpstan fixture. Each report is [line, placeholders,
values], the counts PHPStan's message names, or [line, "invalid"] for "contains an invalid
placeholder". Run offline, never in CI:
python3 generate_phpstan.py /tmp/rerail-php-tools/vendor/bin/phpstan

Requires PHP 8.5.11 and PHPStan 2.2.16."""
from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).parent
IDENTIFIERS = {"argument.sprintf", "argument.printf", "argument.vsprintf", "argument.vprintf"}
COUNTS = re.compile(r"^Call to \w+ contains (\d+) placeholders?, (\d+) values? given\.$")


def report(phpstan: str, path: pathlib.Path) -> list[list[object]]:
    output = subprocess.run(
        [phpstan, "analyse", "--no-progress", "--level=max", "--error-format=json", str(path)],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    data = json.loads(output)
    reports: list[list[object]] = []
    for messages in data["files"].values():
        for message in messages["messages"]:
            if message.get("identifier") not in IDENTIFIERS:
                continue
            text = str(message["message"])
            counts = COUNTS.match(text)
            if counts:
                reports.append([int(message["line"]), int(counts[1]), int(counts[2])])
            elif text.endswith("contains an invalid placeholder."):
                reports.append([int(message["line"]), "invalid"])
            else:
                raise SystemExit(f"unexpected report: {text}")
    return sorted(reports, key=lambda item: item[0])


def main() -> None:
    phpstan = sys.argv[1]
    version = subprocess.run([phpstan, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    if "2.2.16" not in version:
        raise SystemExit(f"PHPStan 2.2.16 required, found {version}")
    php = subprocess.run(["php", "-r", "echo PHP_VERSION;"], capture_output=True, text=True, check=True).stdout
    if php != "8.5.11":
        raise SystemExit(f"PHP 8.5.11 required, found {php}")
    results: dict[str, object] = {"phpstan": version, "php": php}
    for path in sorted((HERE / "phpstan").glob("*.php")):
        results[path.name] = report(phpstan, path)
    (HERE / "phpstan.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
