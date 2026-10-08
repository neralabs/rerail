"""Writes phpstan.expected.json with PHPStan 2.2.16's reports for each suppress/phpstan fixture,
with its local ignore comments and with the directives stripped (renamed in a byte-for-byte copy,
so line endings and offsets are the original's). PHPStan 2.2 rejects a bare `@phpstan-ignore`;
those parse errors remain recorded in `kept`. Run offline, never in CI:
python3 generate_phpstan.py /tmp/rerail-php-tools/vendor/bin/phpstan

Requires PHP 8.5.11 and PHPStan 2.2.16. Reports are [identifier, start byte, end byte], at the
start of the reported line."""
from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).parent


def offset(source: bytes, line: int) -> int:
    """Where `line` starts: PHP's lexer ends a line at `\\n`, `\\r\\n` or a lone `\\r`."""
    starts = [0] + [match.end() for match in re.finditer(rb"\r\n|\r|\n", source)]
    return starts[line - 1]


def report(phpstan: str, path: pathlib.Path) -> list[list[object]]:
    output = subprocess.run(
        [phpstan, "analyse", "--no-progress", "--level=max", "--error-format=json", str(path)],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    data = json.loads(output)
    source = path.read_bytes()
    return sorted(
        [
            str(message.get("identifier", "")),
            offset(source, int(message["line"])),
            offset(source, int(message["line"])),
        ]
        for messages in data["files"].values()
        for message in messages["messages"]
    )

def without_directives(source: bytes) -> bytes:
    return source.replace(b"@phpstan-ignore", b"@xxxxxxx-ignore")


def main() -> None:
    phpstan = sys.argv[1]
    version = subprocess.run([phpstan, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    results: dict[str, object] = {"phpstan": version}
    for path in sorted((HERE / "phpstan").glob("*.php")):
        with tempfile.TemporaryDirectory() as temp:
            stripped = pathlib.Path(temp) / path.name
            stripped.write_bytes(without_directives(path.read_bytes()))
            results[path.name] = {"all": report(phpstan, stripped), "kept": report(phpstan, path)}
    (HERE / "phpstan.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
