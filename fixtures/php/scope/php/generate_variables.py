"""Writes variables.expected.json: for every variables/*.php fixture here, where PHPStan 2.2.16 reports a variable
undefined (`variable.undefined`: "Undefined variable: $x" is `undefined`, "Variable $x might not be defined." is `maybe`,
"Call to function compact() contains undefined variable $x." is `compact`), and where Psalm 6.19.1 reports an unused
variable, parameter, closure parameter or foreach value (`--find-unused-variables`), each by line and variable name.
Both run on PHP 8.5.11; a run that fails (any exit code but success or issues found) or writes no report fails the
generation, with the tool's stderr. Run offline, not in CI, with the tools a Composer project provides:
    python3 generate_variables.py /path/to/composer/project
where the project requires phpstan/phpstan:2.2.16 and vimeo/psalm:6.19.1."""

from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).parent
FIXTURES = HERE / "variables"
PSALM_XML = """<?xml version="1.0"?>
<psalm errorLevel="1" findUnusedVariablesAndParams="true" findUnusedCode="false" findUnusedBaselineEntry="false"
       xmlns="https://getpsalm.org/schema/config">
    <projectFiles><directory name="{directory}" /></projectFiles>
</psalm>
"""
USAGE_TYPES = {"UnusedVariable", "UnusedParam", "UnusedClosureParam", "UnusedForeachValue"}


def version(command: list[str]) -> str:
    return subprocess.run(command, capture_output=True, text=True, check=True).stdout.strip()


def analysis_report(tool: str, run: subprocess.CompletedProcess[str], codes: set[int]) -> object:
    """The JSON report of an analysis run that ended with one of its success or issues-found exit
    codes; any other run (a crash, a bad configuration, no report) fails the generation, so a
    missing analysis never reads as an empty oracle."""
    def fail(why: str) -> SystemExit:
        return SystemExit(f"{tool} {why} (exit code {run.returncode}); stderr:\n{run.stderr}")
    if run.returncode not in codes:
        raise fail("failed")
    try:
        parsed = json.loads(run.stdout)
    except json.JSONDecodeError as error:
        raise fail(f"wrote no JSON report: {error}") from error
    if not parsed:
        raise fail("reported nothing, though the fixtures hold issues it reports")
    return parsed


def phpstan(tools: pathlib.Path) -> dict[str, list[dict[str, object]]]:
    run = subprocess.run(
        [str(tools / "vendor/bin/phpstan"), "analyse", "--level=max", "--no-progress", "--error-format=json",
         "--memory-limit=1G", str(FIXTURES)],
        capture_output=True, text=True,
    )
    # 0: no errors; 1: errors found.
    report = analysis_report("PHPStan", run, {0, 1})
    assert isinstance(report, dict) and report.get("files"), report
    assert not report["errors"], report["errors"]
    found: dict[str, list[dict[str, object]]] = {}
    for path, file in report["files"].items():
        for message in file["messages"]:
            if message.get("identifier") != "variable.undefined":
                continue
            text = message["message"]
            for pattern, kind in (
                (r"^Undefined variable: \$(\w+)$", "undefined"),
                (r"^Variable \$(\w+) might not be defined\.$", "maybe"),
                (r"^Call to function compact\(\) contains undefined variable \$(\w+)\.$", "compact"),
            ):
                matched = re.match(pattern, text)
                if matched:
                    found.setdefault(pathlib.Path(path).name, []).append(
                        {"line": message["line"], "name": matched.group(1), "kind": kind})
                    break
            else:
                raise AssertionError(f"unexpected message {text!r}")
    return found


def psalm(tools: pathlib.Path) -> dict[str, list[dict[str, object]]]:
    with tempfile.TemporaryDirectory() as directory:
        config = pathlib.Path(directory) / "psalm.xml"
        config.write_text(PSALM_XML.format(directory=FIXTURES))
        run = subprocess.run(
            [str(tools / "vendor/bin/psalm"), "--no-cache", "--no-progress", "--output-format=json",
             "--find-unused-variables", f"--config={config}", f"--root={FIXTURES}"],
            capture_output=True, text=True, cwd=FIXTURES,
        )
    # 0: no issues; 2: issues found.
    issues = analysis_report("Psalm", run, {0, 2})
    assert isinstance(issues, list), issues
    found: dict[str, list[dict[str, object]]] = {}
    for issue in issues:
        if issue["type"] not in USAGE_TYPES:
            continue
        found.setdefault(pathlib.Path(issue["file_path"]).name, []).append(
            {"line": issue["line_from"], "name": issue["selected_text"].lstrip("&$."), "type": issue["type"]})
    return found


def main() -> None:
    tools = pathlib.Path(sys.argv[1])
    definedness = phpstan(tools)
    usage = psalm(tools)
    fixtures = sorted(path.name for path in FIXTURES.glob("*.php"))
    out = {
        "php": version(["php", "-r", "echo PHP_VERSION;"]),
        "phpstan": version([str(tools / "vendor/bin/phpstan"), "--version", "--no-ansi"]),
        "psalm": version([str(tools / "vendor/bin/psalm"), "--version"]),
        "fixtures": {
            name: {
                "definedness": sorted(definedness.get(name, []), key=lambda r: (r["line"], r["name"], r["kind"])),
                "usage": sorted(usage.get(name, []), key=lambda r: (r["line"], r["name"], r["type"])),
            }
            for name in fixtures
        },
    }
    (HERE / "variables.expected.json").write_text(json.dumps(out, indent=1) + "\n")


if __name__ == "__main__":
    main()
