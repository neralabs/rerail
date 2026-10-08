"""Writes php.expected.json: for the PHP workspace in project/, what each call reaches.

- `phpstan`: PHPStan 2.2.16's reflection of every call (phpstan/CallTargetCollector.php, at level 0
  over the whole workspace): per file, each call's byte range, line, kind and resolved targets
  (a user function by name and file, a user method by the file and line of its declaration, a
  trait's where a trait declares it, a builtin, no constructor, a first-class callable, or
  unresolved). A call in a trait PHPStan reads once per class using it; where those readings
  differ, the call is recorded unresolved (`context`).
- `runtime`: PHP 8.5.11 running project/main.php with every function-like instrumented
  (trace.php): per file, each call site's line and the declarations (file, start line) PHP
  dispatched it to.

Paths are relative to project/. Run offline, not in CI, with the tools a Composer project
provides:
    python3 generate_php.py /path/to/composer/project
where the project requires phpstan/phpstan:2.2.16 and nikic/php-parser:5.9.0, under PHP 8.5.11."""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).parent
WORKSPACE = HERE / "project"
PHPSTAN = HERE / "phpstan"


def relative(path: str) -> str:
    return str(pathlib.Path(path).resolve().relative_to(WORKSPACE.resolve()))


def phpstan(tools: pathlib.Path) -> dict[str, list[dict[str, object]]]:
    with tempfile.TemporaryDirectory() as directory:
        config = pathlib.Path(directory) / "phpstan.neon"
        config.write_text(
            f"includes:\n    - {PHPSTAN / 'phpstan.neon'}\n"
            f"parameters:\n    level: 0\n    tmpDir: {directory}/cache\n    paths:\n        - {WORKSPACE}\n")
        output = pathlib.Path(directory) / "targets.json"
        run = subprocess.run(
            [str(tools / "vendor/bin/phpstan"), "analyse", "-c", str(config), "--autoload-file",
             str(PHPSTAN / "autoload.php"), "--no-progress", "--error-format=json", "--memory-limit=1G"],
            capture_output=True, text=True, env={**os.environ, "RERAIL_CALL_TARGETS": str(output)},
        )
        report = json.loads(run.stdout)
        assert not report["errors"], report["errors"]
        records = json.loads(output.read_text())
    calls: dict[tuple[str, int, int], dict[str, object]] = {}
    for records_of_file in records.values():
        for start, end, line, kind, targets, file in records_of_file:
            for target in targets:
                if target[0] == "function":
                    target[2] = relative(target[2])
                elif target[0] == "method":
                    target[3] = relative(target[3])
            key = (relative(file), start, end)
            known = calls.get(key)
            # PHPStan reads a nullsafe call twice (as itself and as the call it guards), and a
            # trait's call once per class using it.
            if known is None:
                calls[key] = {"start": start, "end": end, "line": line, "kind": kind, "targets": targets}
            elif known["targets"] != targets:
                known["targets"] = [["unresolved", "context"]]
    found: dict[str, list[dict[str, object]]] = {}
    for (file, _, _), call in sorted(calls.items()):
        found.setdefault(file, []).append(call)
    return found


def runtime(tools: pathlib.Path) -> dict[str, list[dict[str, object]]]:
    run = subprocess.run(
        ["php", str(HERE / "trace.php"), str(tools / "vendor/autoload.php"), str(WORKSPACE)],
        capture_output=True, text=True, check=True,
    )
    return json.loads(run.stdout)


def main() -> None:
    tools = pathlib.Path(sys.argv[1])
    version = subprocess.run([str(tools / "vendor/bin/phpstan"), "--version", "--no-ansi"],
                             capture_output=True, text=True, check=True).stdout.strip()
    php = subprocess.run(["php", "-r", "echo PHP_VERSION;"], capture_output=True, text=True,
                         check=True).stdout
    expected = {"phpstanVersion": version, "php": php, "phpstan": phpstan(tools), "runtime": runtime(tools)}
    (HERE / "php.expected.json").write_text(json.dumps(expected, indent=1) + "\n")


if __name__ == "__main__":
    main()
