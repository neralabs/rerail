"""Runs PHPStan 2.2.16's PrintfParametersRule and PrintfArrayParametersRule (`argument.sprintf`,
`argument.printf`, `argument.vsprintf`, `argument.vprintf`, and the rule's `argument.sscanf`,
`argument.fscanf`) over whole PHP corpora for `benchmark_php_format_corpus`
(rerail/native/src/policy_dsl/compile/format_args/php.rs). Offline, not in CI:

    python3 generate_corpus.py <composer project with phpstan/phpstan 2.2.16> <output.json> \
        <name>=<corpus root>:<path>[,<path>...] ...

e.g. from the checkout root:

    python3 rerail-fixtures/php/format_args/generate_corpus.py .nera/tools/php .nera/t3/format.input.json \
        laravel=.nera/corpora/php/laravel:src,tests symfony=.nera/corpora/php/symfony:src

Both rules are level 0 rules (`#[RegisteredRule(0)]`) and read the format through scope types,
which do not depend on the level: PHPStan runs at level 0. It runs with the environment cleared
(PATH and HOME only): in an agent's environment PHPStan prints a banner before its JSON. Files
PHP cannot parse (`php -l`) and Blade templates are excluded, as tape parity excludes them; a
file PHPStan cannot reflect stops its analysis, so it is excluded and PHPStan runs again.
Writes {"phpstan", "php", "corpora": [{"name", "root", "paths", "excluded", "reports":
{file: [{"line", "function", "message", "kind"}]}}]}, `kind` being `invalid` ("contains an
invalid placeholder"), `missing` (more placeholders than values), `unused` (fewer) or `range`
(a count PHPStan gives as a range). A trait's report, which PHPStan gives once per class using
the trait, is recorded once.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile

FUNCTIONS = ("sprintf", "printf", "vsprintf", "vprintf", "sscanf", "fscanf")
IDENTIFIERS = {f"argument.{function}" for function in FUNCTIONS}
COUNT = r"(\d+(?:-\d+| or more| or less)?)"
COUNTS = re.compile(rf"^Call to (\w+) contains {COUNT} placeholders?, {COUNT} values? given\.$")
INVALID = re.compile(r"^Call to (\w+) contains an invalid placeholder\.$")
ENV = {"PATH": os.environ["PATH"], "HOME": os.environ["HOME"]}


def unparsable(paths: list[pathlib.Path]) -> list[str]:
    bad = []
    for root in paths:
        for path in sorted(root.rglob("*.php")):
            if path.name.endswith(".blade.php"):
                continue
            if subprocess.run(["php", "-l", str(path)], capture_output=True).returncode != 0:
                bad.append(str(path))
    return bad


def kind(message: str) -> tuple[str, str]:
    counts = COUNTS.match(message)
    if counts:
        placeholders, values = counts[2], counts[3]
        if placeholders.isdigit() and values.isdigit():
            return counts[1], "missing" if int(placeholders) > int(values) else "unused"
        return counts[1], "range"
    invalid = INVALID.match(message)
    if invalid:
        return invalid[1], "invalid"
    raise AssertionError(message)


def phpstan(tools: pathlib.Path, paths: list[pathlib.Path], excluded: list[str]) -> dict[str, list[dict]]:
    while True:
        with tempfile.TemporaryDirectory() as directory:
            config = pathlib.Path(directory) / "phpstan.neon"
            lines = ["parameters:\n  level: 0\n", f"  tmpDir: {directory}/cache\n", "  paths:\n"]
            lines += [f"    - '{path}'\n" for path in paths]
            lines.append("  excludePaths:\n    - '*.blade.php'\n")
            lines += [f"    - '{path}'\n" for path in excluded]
            config.write_text("".join(lines))
            run = subprocess.run(
                [str(tools / "vendor/bin/phpstan"), "analyse", "-c", str(config), "--no-progress",
                 "--error-format=json", "--memory-limit=16G"],
                capture_output=True, text=True, env=ENV,
            )
        report = json.loads(run.stdout)
        assert not report["errors"], report["errors"]
        # A file PHPStan cannot reflect stops the run; exclude it and run again.
        severe = [path for path, file in report["files"].items()
                  if any(message.get("identifier", "").startswith("phpstan.") or
                         message["message"].startswith("Reflection error")
                         for message in file["messages"])]
        if not severe:
            break
        excluded.extend(severe)
    found: dict[str, list[dict]] = {}
    for path, file in report["files"].items():
        seen = set()
        for message in file["messages"]:
            if message.get("identifier") not in IDENTIFIERS:
                continue
            key = (message["line"], message["message"])
            if key in seen:
                continue
            seen.add(key)
            function, shape = kind(message["message"])
            found.setdefault(path, []).append({"line": message["line"], "function": function,
                                               "message": message["message"], "kind": shape})
    return found


def main() -> None:
    tools = pathlib.Path(sys.argv[1]).resolve()
    output = pathlib.Path(sys.argv[2])
    version = subprocess.run([str(tools / "vendor/bin/phpstan"), "--version", "--no-ansi"],
                             capture_output=True, text=True, env=ENV).stdout.strip()
    if "2.2.16" not in version:
        raise SystemExit(f"PHPStan 2.2.16 required, found {version}")
    php = subprocess.run(["php", "-r", "echo PHP_VERSION;"], capture_output=True, text=True).stdout
    if php != "8.5.11":
        raise SystemExit(f"PHP 8.5.11 required, found {php}")
    corpora = []
    for argument in sys.argv[3:]:
        name, _, spec = argument.partition("=")
        root_text, _, paths_text = spec.partition(":")
        root = pathlib.Path(root_text).resolve()
        roots = [root / path for path in paths_text.split(",")]
        excluded = unparsable(roots)
        reports = phpstan(tools, roots, excluded)
        corpora.append({"name": name, "root": str(root), "paths": [str(path) for path in roots],
                        "excluded": excluded, "reports": reports})
    output.write_text(json.dumps({"phpstan": version, "php": php, "corpora": corpora}, indent=1) + "\n")


if __name__ == "__main__":
    main()
