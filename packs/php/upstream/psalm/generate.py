"""Records Psalm 6.19.1 for the PHP pack's ported issues (rerail/packs/php/conformance.json). Run by
`npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request CI), with
PHP 8.5.11:

    python3 generate.py fixtures <Psalm checkout at tag 6.19.1> <composer project with vimeo/psalm 6.19.1>

`fixtures` reads every case of the issues' own tests (tests/UnusedVariableTest.php, tests/ForbiddenCodeTest.php):
providerValidCodeParse() and providerInvalidCodeParse(), read statically by read_tests.php. Every case of both
providers is read (all are constant expressions, none asks for a PHP version above 8.5), so none is left out. Each
case is copied to fixtures/<Test>/<provider>/<case>.php (the pack's licenses/psalm/UPSTREAM.md lists them) and analysed alone, as
the upstream test analyses it: as src/somefile.php of its own project, at error level 1 with unused variables found
(UnusedVariableTest's reportUnusedVariables(); not unused code), the case's ignored_issues suppressed, for its
php_version (the tests' default 7.4; 8.0 for a `PHP80-` case, 8.1 for a valid `PHP81-` one). The recording is
Psalm's UnusedVariable and ForbiddenCode reports. As the upstream tests do, an invalid case must report its
error_message (an issue type, optionally ` - src/somefile.php:<line>:<column> - <message>`), and a valid case must
report neither issue; `SKIPPED-` cases, which the tests skip as known Psalm bugs, are recorded without that check.

Each finding is the issue type, path, line, and column: Psalm's column counts bytes, so the column is recounted in
code points from Psalm's byte offset (`from`)."""

from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from xml.sax.saxutils import quoteattr

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

ISSUES = ["ForbiddenCode", "UnusedVariable"]
TESTS = ["UnusedVariableTest", "ForbiddenCodeTest"]
ORIGIN = "https://github.com/vimeo/psalm@e2ca44251c1f1aa2e35452d426895746b47de64e"
SCRATCH = HERE.parents[3] / ".rerail" / "scratch" / "psalm"
# Psalm reads these; an agent's environment changes its output.
ENV = {key: value for key, value in os.environ.items()
       if key not in ("AGENT", "CLAUDECODE", "OMPCODE", "ORCA_OPENCODE_AGENT")}


def config(directory: pathlib.Path, files: list[str], ignored: list[str]) -> None:
    """psalm.xml as tests/TestConfig.php configures the tests: level 1, unused variables (not unused code)."""
    handlers = "".join(f'<{issue} errorLevel="suppress"/>' for issue in ignored)
    project = "".join(f"<directory name={quoteattr(file)}/>" if not file.endswith(".php")
                      else f"<file name={quoteattr(file)}/>" for file in files)
    (directory / "psalm.xml").write_text(
        '<?xml version="1.0"?>\n'
        '<psalm errorLevel="1" findUnusedCode="false" findUnusedVariablesAndParams="true"'
        ' findUnusedBaselineEntry="false" findUnusedPsalmSuppress="false" ensureOverrideAttribute="false"'
        ' strictBinaryOperands="false" ignoreInternalFunctionFalseReturn="true"'
        ' ignoreInternalFunctionNullReturn="true" useDocblockTypes="true">\n'
        f"<projectFiles>{project}</projectFiles>\n"
        + (f"<issueHandlers>{handlers}</issueHandlers>\n" if handlers else "") + "</psalm>\n")


def psalm(project: pathlib.Path, directory: pathlib.Path, php_version: str, *flags: str) -> list[dict]:
    """Every issue Psalm reports for the project configured in `directory`."""
    run = subprocess.run(
        [str(project / "vendor/bin/psalm"), f"--config={directory / 'psalm.xml'}", f"--root={directory}",
         "--no-cache", "--output-format=json", "--no-progress", "--monochrome", "--find-unused-variables",
         f"--php-version={php_version}", "--memory-limit=-1", *flags],
        capture_output=True, text=True, cwd=directory, env=ENV,
    )
    try:
        return json.loads(run.stdout)
    except json.JSONDecodeError:
        raise SystemExit(f"psalm failed in {directory}:\n{run.stdout[:2000]}\n{run.stderr[:2000]}") from None


def column(source: bytes, offset: int) -> int:
    """The 1-based code-point column of byte `offset`."""
    start = source.rfind(b"\n", 0, offset) + 1
    return len(source[start:offset].decode("utf-8", errors="replace")) + 1


def finding(issue: dict, path: str, source: bytes) -> dict:
    return {"rule": issue["type"], "path": path, "line": issue["line_from"], "column": column(source, issue["from"])}


def version(project: pathlib.Path) -> str:
    text = subprocess.run([str(project / "vendor/bin/psalm"), "--version"], capture_output=True, text=True,
                          check=True, env=ENV).stdout
    return text.split("Psalm ")[1].split("@")[0]


def reported(issue: dict, expected: str) -> bool:
    """Whether the upstream test's `\\b<error_message>\\b` matches the exception Psalm throws for `issue`: the type,
    the location, then the message (to which a Mixed* issue's exception appends where the mixed value came from)."""
    thrown = f"{issue['type']} - src/somefile.php:{issue['line_from']}:{issue['column_from']} - {issue['message']}"
    return expected in thrown or (issue["type"].startswith("Mixed") and thrown in expected)


def analyse(project: pathlib.Path, case: dict, directory: pathlib.Path) -> list[dict]:
    shutil.rmtree(directory, ignore_errors=True)
    (directory / "src").mkdir(parents=True)
    (directory / "src/somefile.php").write_text(case["code"])
    config(directory, ["src"], case["ignored_issues"])
    name = case["name"]
    php_version = case["php_version"] or ("8.0" if name.startswith("PHP80-") else
                                          "8.1" if name.startswith("PHP81-") and case["provider"] == "valid" else
                                          "7.4")
    return psalm(project, directory, php_version, "--threads=1")


def fixtures(checkout: pathlib.Path, project: pathlib.Path) -> None:
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    cases = []
    for test in TESTS:
        read = subprocess.run(["php", str(HERE / "read_tests.php"), str(project / "vendor/autoload.php"),
                               str(checkout / "tests" / f"{test}.php")], capture_output=True, text=True, check=True)
        for case in json.loads(read.stdout):
            assert case["php_version"] is None or float(case["php_version"]) <= 8.5, case
            case["path"] = f"{test}/{case['provider']}/{case['name']}.php"
            cases.append(case)
    assert len({case["path"] for case in cases}) == len(cases)
    with ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(lambda pair: analyse(project, pair[1], SCRATCH / "fixtures" / str(pair[0])),
                                enumerate(cases)))
    findings = []
    failures = []
    for case, issues in zip(cases, results):
        (target / case["path"]).parent.mkdir(parents=True, exist_ok=True)
        (target / case["path"]).write_text(case["code"])
        source = case["code"].encode()
        kept = [issue for issue in issues if issue["type"] in ISSUES]
        findings += [finding(issue, case["path"], source) for issue in kept]
        if case["name"].startswith("SKIPPED-"):
            continue
        if case["provider"] == "invalid" and not any(reported(issue, case["error_message"]) for issue in issues):
            failures.append(f"{case['path']}: expected {case['error_message']!r}, Psalm reports "
                            f"{[(i['type'], i['line_from'], i['column_from'], i['message']) for i in issues]}")
        if case["provider"] == "valid" and kept:
            failures.append(f"{case['path']}: valid, Psalm reports {[(i['type'], i['line_from']) for i in kept]}")
    assert not failures, "\n".join(failures)
    common.write(HERE / "fixtures.expected.json", "psalm", version(project),
                 {"kind": "fixtures", "origin": ORIGIN, "rules": ISSUES, "files": sorted(c["path"] for c in cases)},
                 findings)


def main() -> None:
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), pathlib.Path(sys.argv[3]).resolve())
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
