"""Runs PHPStan's call-target reflection (php/calls/php/phpstan) over sampled PHP corpora
for `benchmark_php_calls_corpus` (Rerail's native/src/policy_dsl/calls/php_tests.rs). Offline, not
in CI:

    python3 generate_corpus.py <composer project with phpstan/phpstan 2.2.16> <output.json> \
        <name>=<workspace root>[,<workspace root>...]:<sampled path>[,<sampled path>...] ...

Rerail links every PHP file under the workspace roots; PHPStan analyses the sampled paths, with
the workspace roots scanned for the classes and functions they use. Files PHP cannot parse
(`php -l`) and Blade templates are excluded. Writes {"phpstan", "php", "corpora": [{"name",
"roots", "sampled", "excluded", "records": {file: [[start, end, line, kind, targets], ...]}}]},
each call filed under the file it is written in (a trait's call, which PHPStan reads once per
class using it, once: unresolved (`context`) where those readings differ).
"""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
import tempfile

ORACLE = pathlib.Path(__file__).resolve().parent / "phpstan"


def unparsable(paths: list[pathlib.Path]) -> list[str]:
    bad = []
    for root in paths:
        for path in sorted(root.rglob("*.php")):
            if path.name.endswith(".blade.php"):
                continue
            if subprocess.run(["php", "-l", str(path)], capture_output=True).returncode != 0:
                bad.append(str(path))
    return bad


def phpstan(tools: pathlib.Path, roots: list[pathlib.Path], sampled: list[pathlib.Path],
            excluded: list[str]) -> dict[str, list]:
    with tempfile.TemporaryDirectory() as directory:
        config = pathlib.Path(directory) / "phpstan.neon"
        lines = [f"includes:\n    - {ORACLE / 'phpstan.neon'}\n", "parameters:\n    level: 0\n",
                 f"    tmpDir: {directory}/cache\n", "    paths:\n"]
        lines += [f"        - '{path}'\n" for path in sampled]
        lines.append("    scanDirectories:\n")
        lines += [f"        - '{root}'\n" for root in roots]
        lines.append("    excludePaths:\n        - '*.blade.php'\n")
        lines += [f"        - '{path}'\n" for path in excluded]
        config.write_text("".join(lines))
        output = pathlib.Path(directory) / "targets.json"
        run = subprocess.run(
            [str(tools / "vendor/bin/phpstan"), "analyse", "-c", str(config), "--autoload-file",
             str(ORACLE / "autoload.php"), "--no-progress", "--error-format=json", "--memory-limit=16G"],
            capture_output=True, text=True, env={**os.environ, "RERAIL_CALL_TARGETS": str(output)},
        )
        report = json.loads(run.stdout)
        assert not report["errors"], report["errors"]
        records = json.loads(output.read_text())
    calls: dict[tuple[str, int, int], list] = {}
    for records_of_file in records.values():
        for start, end, line, kind, targets, file in records_of_file:
            known = calls.get((file, start, end))
            if known is None:
                calls[(file, start, end)] = [start, end, line, kind, targets]
            elif known[4] != targets:
                known[4] = [["unresolved", "context"]]
    found: dict[str, list] = {}
    for (file, _, _), call in sorted(calls.items()):
        found.setdefault(file, []).append(call)
    return found


def main() -> None:
    tools = pathlib.Path(sys.argv[1])
    output = pathlib.Path(sys.argv[2])
    corpora = []
    for argument in sys.argv[3:]:
        name, _, spec = argument.partition("=")
        roots_text, _, sampled_text = spec.partition(":")
        roots = [pathlib.Path(path).resolve() for path in roots_text.split(",")]
        sampled = [pathlib.Path(path).resolve() for path in sampled_text.split(",")]
        excluded = unparsable(roots)
        records = phpstan(tools, roots, sampled, excluded)
        corpora.append({"name": name, "roots": [str(root) for root in roots],
                        "sampled": [str(path) for path in sampled], "excluded": excluded,
                        "records": records})
    version = subprocess.run([str(tools / "vendor/bin/phpstan"), "--version", "--no-ansi"],
                             capture_output=True, text=True).stdout.strip()
    php = subprocess.run(["php", "-r", "echo PHP_VERSION;"], capture_output=True, text=True).stdout
    output.write_text(json.dumps({"phpstan": version, "php": php, "corpora": corpora}) + "\n")


if __name__ == "__main__":
    main()
