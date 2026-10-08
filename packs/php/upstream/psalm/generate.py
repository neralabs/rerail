"""Records Psalm 6.19.1 for the PHP pack's ported issues (rerail/conformance/languages/php/pack.json). Run offline,
never in CI, with PHP 8.5.11:

    python3 generate.py fixtures <Psalm checkout at tag 6.19.1> <composer project with vimeo/psalm 6.19.1>
    python3 generate.py corpus <composer project> <name>=<corpus checkout> [...]

`fixtures` reads every case of the issues' own tests (tests/UnusedVariableTest.php, tests/ForbiddenCodeTest.php):
providerValidCodeParse() and providerInvalidCodeParse(), read statically by read_tests.php. Every case of both
providers is read (all are constant expressions, none asks for a PHP version above 8.5), so none is left out. Each
case is copied to fixtures/<Test>/<provider>/<case>.php (licenses/psalm/UPSTREAM.md lists them) and analysed alone, as
the upstream test analyses it: as src/somefile.php of its own project, at error level 1 with unused variables found
(UnusedVariableTest's reportUnusedVariables(); not unused code), the case's ignored_issues suppressed, for its
php_version (the tests' default 7.4; 8.0 for a `PHP80-` case, 8.1 for a valid `PHP81-` one). The recording is
Psalm's UnusedVariable and ForbiddenCode reports. As the upstream tests do, an invalid case must report its
error_message (an issue type, optionally ` - src/somefile.php:<line>:<column> - <message>`), and a valid case must
report neither issue; `SKIPPED-` cases, which the tests skip as known Psalm bugs, are recorded without that check.

`corpus` runs Psalm once over a corpus's pack paths (Blade templates and files `php -l` rejects excluded; files
Psalm reports a ParseError or DuplicateClass for, or crashes on (CRASHES), are skipped too) with the same configuration, for PHP 8.5,
from a project in .rerail/scratch/ whose psalm.xml names the corpus's files (the corpus's own psalm.xml is not read).
Psalm does not analyse a class whose parent or interface it cannot load (PHPUnit's TestCase, for every test), so
that project installs the corpus's dependencies into deps/: the corpus's composer.json without its autoload, scripts
and the packages the checkout itself holds (path and package repositories, which become `replace`d), plus EXTRA,
resolved once by `composer update` and pinned by composer/<name>.lock, which later runs `composer install`. As
`benchmark_php_usage_corpus` reads them (`rerail-fixtures/php/scope/php/generate_usage_corpus.py`), they are Psalm's
`extraFiles`, scanned for declarations and not analysed, less the directories that would shadow the corpus's own
declarations (`shadowing`). The recording's `unanalysed` lists, by file, the lines of the functions Psalm stops
analysing and of the classes it analyses none of, read from its own reports (`unanalysed`): no finding there, Psalm's
or Rerail's, is scored.

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
sys.path.insert(0, str(HERE.parents[1] / "scope" / "php"))
import common  # noqa: E402
from generate_usage_corpus import shadowing, unanalysed  # noqa: E402

ISSUES = ["ForbiddenCode", "UnusedVariable"]
TESTS = ["UnusedVariableTest", "ForbiddenCodeTest"]
ORIGIN = "https://github.com/vimeo/psalm@e2ca44251c1f1aa2e35452d426895746b47de64e"
SCRATCH = HERE.parents[3] / ".rerail" / "scratch" / "psalm"
# Psalm reads these; an agent's environment changes its output.
ENV = {key: value for key, value in os.environ.items()
       if key not in ("AGENT", "CLAUDECODE", "OMPCODE", "ORCA_OPENCODE_AGENT")}
# Files whose analysis throws inside Psalm 6.19.1 and aborts the whole run; they are skipped.
CRASHES = {
    # `@phpstan-import-type Invalid from int`: FileAnalyzer asks for the class storage of `...\Fixtures\int`
    # ("Could not get class storage for symfony\component\typeinfo\tests\fixtures\int").
    "symfony": [
        "src/Symfony/Component/TypeInfo/Tests/Fixtures/DummyWithTypeAliases.php",
        # `(int) $var['res']` of the array Tests/Fixtures/dumb-var.php returns, which holds \NAN: CastAnalyzer casts
        # the NAN literal ("The float NAN is not representable as an int").
        "src/Symfony/Component/VarDumper/Tests/Dumper/CliDumperTest.php",
        "src/Symfony/Component/VarDumper/Tests/Dumper/HtmlDumperTest.php",
    ],
}
# Dependencies a corpus installs outside composer.json: symfony's ./phpunit script installs PHPUnit 13
# (SYMFONY_PHPUNIT_VERSION=13) through symfony/phpunit-bridge's simple-phpunit.
EXTRA = {"symfony": {"phpunit/phpunit": "^13"}}
# The version the checkout's `replace`d packages (`self.version`) have: laravel's Application::VERSION is 13.34.0,
# symfony's Kernel::VERSION 8.2.0-DEV.
ROOT_VERSION = {"laravel": "13.34.0", "symfony": "8.2.x-dev"}


def dependencies(name: str, root: pathlib.Path, directory: pathlib.Path) -> None:
    """Installs the corpus's dependencies into `directory`'s vendor/, from composer/<name>.lock when it exists, with
    no autoloader: Psalm reads them as `extraFiles`."""
    manifest = json.loads((root / "composer.json").read_text())
    for key in ("autoload", "autoload-dev", "scripts"):
        manifest.pop(key, None)
    local = set()
    repositories = []
    for repository in manifest.get("repositories", []):
        if repository["type"] == "path":
            package = json.loads((root / repository["url"] / "composer.json").read_text())
            local |= {package["name"], *package.get("replace", {})}
        elif repository["type"] == "package":
            local.add(repository["package"]["name"])
        else:
            repositories.append(repository)
    manifest["repositories"] = repositories
    for key in ("require", "require-dev"):
        manifest[key] = {package: version for package, version in manifest.get(key, {}).items()
                         if package not in local}
    manifest["require-dev"].update(EXTRA.get(name, {}))
    manifest.setdefault("replace", {}).update({package: "*" for package in sorted(local) if "/" in package})
    (directory / "composer.json").write_text(json.dumps(manifest, indent=1) + "\n")
    lock = HERE / "composer" / f"{name}.lock"
    env = {**ENV, "COMPOSER_ROOT_VERSION": ROOT_VERSION[name], "COMPOSER_NO_INTERACTION": "1"}
    if lock.exists():
        shutil.copyfile(lock, directory / "composer.lock")
    command = "install" if lock.exists() else "update"
    subprocess.run(["composer", command, "--no-scripts", "--no-plugins", "--ignore-platform-reqs", "--no-progress",
                    "--prefer-dist", "--no-autoloader", "--quiet"], cwd=directory, env=env, check=True)
    if not lock.exists():
        lock.parent.mkdir(exist_ok=True)
        shutil.copyfile(directory / "composer.lock", lock)


def config(directory: pathlib.Path, files: list[str], ignored: list[str], skipped: list[str] = (),
           vendor: pathlib.Path | None = None, shadowed: list[pathlib.Path] = ()) -> None:
    """psalm.xml as tests/TestConfig.php configures the tests: level 1, unused variables (not unused code); `vendor`,
    less `shadowed`, as `extraFiles`."""
    handlers = "".join(f'<{issue} errorLevel="suppress"/>' for issue in ignored)
    project = "".join(f"<directory name={quoteattr(file)}/>" if not file.endswith(".php")
                      else f"<file name={quoteattr(file)}/>" for file in files)
    ignore = "".join(f"<file name={quoteattr(file)}/>" for file in skipped)
    left_out = "".join(f"<directory name={quoteattr(str(path))}/>" for path in shadowed)
    extra = "" if vendor is None else (
        f"<extraFiles><directory name={quoteattr(str(vendor))}/>"
        + (f"<ignoreFiles>{left_out}</ignoreFiles>" if left_out else "") + "</extraFiles>\n")
    (directory / "psalm.xml").write_text(
        '<?xml version="1.0"?>\n'
        '<psalm errorLevel="1" findUnusedCode="false" findUnusedVariablesAndParams="true"'
        ' findUnusedBaselineEntry="false" findUnusedPsalmSuppress="false" ensureOverrideAttribute="false"'
        ' strictBinaryOperands="false" ignoreInternalFunctionFalseReturn="true"'
        ' ignoreInternalFunctionNullReturn="true" useDocblockTypes="true">\n'
        f"<projectFiles>{project}" + (f"<ignoreFiles>{ignore}</ignoreFiles>" if ignore else "") + "</projectFiles>\n"
        + extra + (f"<issueHandlers>{handlers}</issueHandlers>\n" if handlers else "") + "</psalm>\n")


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


def corpus(project: pathlib.Path, name: str, root: pathlib.Path) -> None:
    files = common.corpus_files(root, name)
    skipped = set(common.unparsable(root, files)) | set(CRASHES.get(name, []))
    directory = SCRATCH / f"corpus-{name}"
    shutil.rmtree(directory, ignore_errors=True)
    # psalm.xml's directory holds no composer.json, so Psalm looks for no autoloader of the project.
    (directory / "deps").mkdir(parents=True)
    dependencies(name, root, directory / "deps")
    vendor = directory / "deps" / "vendor"
    paths = [root / path for path in common.corpus(name)["paths"]]
    blade = [file for path in paths for file in sorted(path.rglob("*.blade.php"))]
    config(directory, [str(path) for path in paths], [],
           [str(root / file) for file in sorted(skipped)] + [str(file) for file in blade], vendor,
           shadowing(root, vendor))
    issues = psalm(project, directory, "8.5", "--threads=8")
    (directory / "issues.json").write_text(json.dumps(issues))
    by_file: dict[str, list[dict]] = {}
    for issue in issues:
        path = pathlib.Path(issue["file_path"]).resolve()
        # A dependency's file (vendor/) or a class Psalm's own autoloader holds: no corpus file.
        if path.is_relative_to(root):
            by_file.setdefault(path.relative_to(root).as_posix(), []).append(issue)
    analysed = set(files) - skipped
    findings = []
    for path, reports in by_file.items():
        if path not in analysed:
            # Not scored: a file a corpus file includes (`require 'common.inc'`), which Psalm analyses too, or a
            # skipped one.
            continue
        # A ParseError, or a class a dependency declares too (vendor/'s symfony/phpunit-bridge is the checkout's
        # src/Symfony/Bridge/PhpUnit): Psalm does not analyse the file.
        if any(issue["type"] in ("ParseError", "DuplicateClass") for issue in reports):
            skipped.add(path)
            continue
        source = (root / path).read_bytes()
        findings += [finding(issue, path, source) for issue in reports if issue["type"] in ISSUES]
    regions = []
    for file, ranges in unanalysed(project, issues, [root / path for path in sorted(analysed - skipped)]).items():
        path = pathlib.Path(file).relative_to(root).as_posix()
        regions += [{"path": path, "startLine": first, "endLine": last} for _, _, first, last in ranges]
    common.write(HERE / f"corpus-{name}.expected.json", "psalm", version(project),
                 common.corpus_source(name, ISSUES, sorted(skipped), regions), findings)


def main() -> None:
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), pathlib.Path(sys.argv[3]).resolve())
    else:
        project = pathlib.Path(sys.argv[2]).resolve()
        for argument in sys.argv[3:]:
            name, _, path = argument.partition("=")
            corpus(project, name, pathlib.Path(path).resolve())


if __name__ == "__main__":
    main()
