"""Runs PHPStan's `variable.undefined` over PHP corpora for `benchmark_php_definedness_corpus`
(rerail/native/src/code_rule_dsl/scope/php/tests.rs). Offline, not in CI:

    python3 generate_corpus.py <composer project with phpstan/phpstan 2.2.16> <output.json> \
        <name>=<path>[,<path>...] ...

Files PHP cannot parse (`php -l`) and Blade templates are excluded, as tape parity excludes them.
Run PHPStan's environment clean (`env -i PATH="$PATH" HOME="$HOME" python3 ...`): in an agent
environment it prints a banner before its JSON. Writes {"phpstan", "php", "corpora": [{"name",
"commit", "paths", "excluded", "reports": {file: [{"line", "name", "kind"}]}}]}, where `kind` is
`undefined` ("Undefined variable"), `maybe` ("might not be defined") or `compact` (an undefined
variable `compact()` names); the benchmark reports Rerail's recall of each kind.
"""

from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys
import tempfile

PATTERNS = (
    (r"^Undefined variable: \$(\w+)$", "undefined"),
    (r"^Variable \$(\w+) might not be defined\.$", "maybe"),
    (r"^Call to function compact\(\) contains undefined variable \$(\w+)\.$", "compact"),
)


def unparsable(paths: list[pathlib.Path]) -> list[str]:
    bad = []
    for root in paths:
        for path in sorted(root.rglob("*.php")):
            if path.name.endswith(".blade.php"):
                continue
            if subprocess.run(["php", "-l", str(path)], capture_output=True).returncode != 0:
                bad.append(str(path))
    return bad


def phpstan(tools: pathlib.Path, paths: list[pathlib.Path], excluded: list[str]) -> dict[str, list[dict]]:
    while True:
        with tempfile.NamedTemporaryFile("w", suffix=".neon", delete=False) as config:
            config.write("parameters:\n  level: max\n  paths:\n")
            config.writelines(f"    - '{path}'\n" for path in paths)
            config.write("  excludePaths:\n    - '*.blade.php'\n")
            config.writelines(f"    - '{path}'\n" for path in excluded)
        run = subprocess.run(
            [str(tools / "vendor/bin/phpstan"), "analyse", "-c", config.name, "--no-progress",
             "--error-format=json", "--memory-limit=16G"],
            capture_output=True, text=True,
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
        for message in file["messages"]:
            if message.get("identifier") != "variable.undefined":
                continue
            for pattern, kind in PATTERNS:
                matched = re.match(pattern, message["message"])
                if matched:
                    found.setdefault(path, []).append(
                        {"line": message["line"], "name": matched.group(1), "kind": kind})
                    break
            else:
                raise AssertionError(message["message"])
    return found


def main() -> None:
    tools = pathlib.Path(sys.argv[1])
    output = pathlib.Path(sys.argv[2])
    corpora = []
    for argument in sys.argv[3:]:
        name, _, paths = argument.partition("=")
        roots = [pathlib.Path(path).resolve() for path in paths.split(",")]
        excluded = unparsable(roots)
        reports = phpstan(tools, roots, excluded)
        commit = subprocess.run(["git", "-C", str(roots[0]), "rev-parse", "HEAD"],
                                capture_output=True, text=True, check=True).stdout.strip()
        corpora.append({"name": name, "commit": commit, "paths": [str(root) for root in roots],
                        "excluded": excluded, "reports": reports})
    version = subprocess.run([str(tools / "vendor/bin/phpstan"), "--version", "--no-ansi"],
                             capture_output=True, text=True).stdout.strip()
    php = subprocess.run(["php", "-r", "echo PHP_VERSION;"], capture_output=True, text=True).stdout
    output.write_text(json.dumps({"phpstan": version, "php": php, "corpora": corpora}, indent=1) + "\n")


if __name__ == "__main__":
    main()
