"""Records PHPStan 2.2.16 for the PHP pack's ported rules (rerail/packs/php/conformance.json). Run by
`npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request CI), with
PHP 8.5.11:

    python3 generate.py fixtures <phpstan-src checkout at tag 2.2.16> <composer project with phpstan/phpstan 2.2.16>

A finding belongs to a ported rule class by its identifier and, where another class reports the same identifier
(`argument.type`, `missingType.return`, `variable.undefined`), by the message the ported class writes. Only the
ported identifiers are recorded (`IDENTIFIERS`).

`fixtures` reads each ported class's RuleTestCase statically (read_tests.php) and copies the files its test
methods analyse to fixtures/<path under tests/PHPStan> (the pack's licenses/phpstan/UPSTREAM.md lists them). A file several
methods analyse with different rule options takes the method whose options are PHPStan's defaults at level max.
Each analysis unit (a method's files, analysed together) is recorded as:
  - for the identifiers of the class whose test analyses it, the test's own expectation (message, line). A
    multi-file unit's expectation names no file: PHPStan's own run over the unit places each error, and must report
    exactly the expected (message, line) multiset;
  - for the other ported identifiers, PHPStan's output at level max over the unit.
Methods read_tests.php cannot read statically (data providers, helpers, skipped on this PHP) are left out.

Each finding is the identifier, path and line (PHPStan reports no columns). PHPStan is run with the agent environment
variables unset: it prints a banner before its JSON when it detects an agent."""

from __future__ import annotations

import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

ORIGIN = "https://github.com/phpstan/phpstan-src@08e3736efd29631ab8f32596c5dd2362dec8dc26"
AGENT_VARIABLES = ("AGENT", "CLAUDECODE", "OMPCODE", "ORCA_OPENCODE_AGENT")

# Ported identifier -> the message pattern of the ported class (None: no other class reports the identifier).
IDENTIFIERS = {
    "variable.undefined": r"^(Undefined variable: \$\S+|Variable \$\S+ might not be defined\.)$",
    "argument.sprintf": None,
    "argument.sscanf": None,
    "array.duplicateKey": None,
    "argument.type": r"^Parameter #1 \$min \(.*\) of function random_int expects lower number than parameter #2 \$max \(.*\)\.$",
    "missingType.return": r"^Function \S+\(\) has no return type specified\.$",
    "if.alwaysTrue": None,
    "if.alwaysFalse": None,
}


def _printf(message: str) -> str | None:
    called = re.match(r"^Call to (\w+) contains ", message)
    return f"argument.{called.group(1)}" if called else None


def _if(message: str) -> str | None:
    constant = re.match(r"^If condition is always (true|false)\.$", message)
    return f"if.always{constant.group(1).capitalize()}" if constant else None


# Ported class's test (under tests/PHPStan) -> its identifiers, and the identifier of one of its expected messages.
TESTS = {
    "Rules/Variables/DefinedVariableRuleTest.php": (
        {"variable.undefined"},
        lambda message: "variable.undefined" if re.match(IDENTIFIERS["variable.undefined"], message) else None),
    "Rules/Functions/PrintfParametersRuleTest.php": ({"argument.sprintf", "argument.sscanf"}, _printf),
    "Rules/Arrays/DuplicateKeysInLiteralArraysRuleTest.php": ({"array.duplicateKey"}, lambda message: "array.duplicateKey"),
    "Rules/Functions/RandomIntParametersRuleTest.php": (
        {"argument.type"},
        lambda message: "argument.type" if re.match(IDENTIFIERS["argument.type"], message) else None),
    "Rules/Functions/MissingFunctionReturnTypehintRuleTest.php": (
        {"missingType.return"},
        lambda message: "missingType.return" if re.match(IDENTIFIERS["missingType.return"], message) else None),
    "Rules/Comparison/IfConstantConditionRuleTest.php": ({"if.alwaysTrue", "if.alwaysFalse"}, _if),
}

# The rule options a test method sets, as PHPStan 2.2.16 configures them at level max (conf/config*.neon).
DEFAULTS = {
    "cliArgumentsVariablesRegistered": True,
    "checkMaybeUndefinedVariables": True,
    "polluteScopeWithLoopInitialAssignments": True,
    "polluteScopeWithAlwaysIterableForeach": True,
    "treatPhpDocTypesAsCertain": True,
    "checkNonStringableDynamicAccess": False,
}


def environment() -> dict[str, str]:
    return {key: value for key, value in os.environ.items() if key not in AGENT_VARIABLES}


def ported(identifier: str | None, message: str) -> bool:
    if identifier not in IDENTIFIERS:
        return False
    pattern = IDENTIFIERS[identifier]
    return pattern is None or re.match(pattern, message) is not None


def phpstan(project: pathlib.Path, paths: list[pathlib.Path], excluded: list[str], *flags: str) -> dict:
    """PHPStan's JSON report at level max over `paths`, with its own temporary directory (runs may be parallel)."""
    scratch = tempfile.mkdtemp(prefix="phpstan-")
    config = pathlib.Path(scratch) / "phpstan.neon"
    config.write_text("parameters:\n  level: max\n  tmpDir: '" + scratch + "/tmp'\n  paths:\n"
                      + "".join(f"    - '{path}'\n" for path in paths)
                      + "  excludePaths:\n    analyseAndScan:\n      - '*.blade.php'\n"
                      + "".join(f"      - '{path}'\n" for path in excluded))
    try:
        run = subprocess.run(
            [str(project / "vendor/bin/phpstan"), "analyse", "-c", str(config), "--no-progress", "--error-format=json",
             "--memory-limit=16G", *flags],
            capture_output=True, text=True, env=environment(),
        )
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    try:
        report = json.loads(run.stdout)
    except json.JSONDecodeError:
        raise SystemExit(f"PHPStan over {[str(path) for path in paths][:3]} printed no report:\n{run.stdout[-2000:]}\n"
                         f"{run.stderr[-2000:]}") from None
    assert not report["errors"], report["errors"]
    return report


def messages(report: dict, root: pathlib.Path) -> list[dict]:
    """The ported findings of a report: identifier, path relative to `root`, line, message."""
    found = []
    for path, file in report["files"].items():
        relative = pathlib.Path(path).resolve().relative_to(root.resolve()).as_posix()
        for message in file["messages"]:
            if ported(message.get("identifier"), message["message"]):
                found.append({"rule": message["identifier"], "path": relative, "line": message["line"],
                              "message": message["message"]})
    return found


def version(project: pathlib.Path) -> str:
    text = subprocess.run([str(project / "vendor/bin/phpstan"), "--version"], capture_output=True, text=True,
                          check=True, env=environment()).stdout
    return text.strip().split()[-1]


def choose(methods: list[dict]) -> list[dict]:
    """One method per analysed file set: the one whose options are PHPStan's defaults, else the first."""
    chosen: dict[tuple[str, ...], dict] = {}
    for method in methods:
        key = tuple(method["files"])
        score = sum(DEFAULTS.get(name) == value for name, value in method["properties"].items())
        current = chosen.get(key)
        if current is None or score > current["score"]:
            chosen[key] = {**method, "score": score}
    return list(chosen.values())


def fixtures(checkout: pathlib.Path, project: pathlib.Path, dump: pathlib.Path | None) -> None:
    tests_root = checkout / "tests/PHPStan"
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    units: list[dict] = []
    for test, (identifiers, identify) in TESTS.items():
        read = subprocess.run(["php", str(HERE / "read_tests.php"), str(project / "vendor/autoload.php"),
                               str(tests_root / test)], capture_output=True, text=True, check=True)
        for method in choose(json.loads(read.stdout)):
            files = []
            for file in method["files"]:
                source = (tests_root / test).parent.joinpath(file).resolve()
                copy = source.relative_to(tests_root.resolve()).as_posix()
                (target / copy).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, target / copy)
                files.append(copy)
            expected = []
            for message, line in method["errors"]:
                identifier = identify(message)
                if identifier in identifiers:
                    expected.append((identifier, message, line))
            units.append({"test": test, "method": method["method"], "files": files, "identifiers": identifiers,
                          "expected": expected})

    def analyse(unit: dict) -> list[dict]:
        return messages(phpstan(project, [target / file for file in unit["files"]], []), target)

    with ThreadPoolExecutor(max_workers=6) as pool:
        reports = list(pool.map(analyse, units))

    findings: list[dict] = []
    seen: set[str] = set()
    for unit, reported in zip(units, reports):
        assert not seen.intersection(unit["files"]), (unit["method"], seen.intersection(unit["files"]))
        seen.update(unit["files"])
        for finding in reported:
            if finding["rule"] not in unit["identifiers"]:
                findings.append(finding)
        own = [finding for finding in reported if finding["rule"] in unit["identifiers"]]
        if len(unit["files"]) == 1:
            findings += [{"rule": identifier, "path": unit["files"][0], "line": line, "message": message}
                         for identifier, message, line in unit["expected"]]
        else:
            expected = Counter((identifier, message, line) for identifier, message, line in unit["expected"])
            placed = Counter((finding["rule"], finding["message"], finding["line"]) for finding in own)
            assert expected == placed, (unit["method"], expected, placed)
            findings += own
    files = [unit["files"][0] if len(unit["files"]) == 1 else unit["files"] for unit in units]
    if dump is not None:
        dump.write_text(json.dumps(findings, indent=1) + "\n")
    common.write(HERE / "fixtures.expected.json", "phpstan", version(project),
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(IDENTIFIERS), "files": files},
                 [{key: finding[key] for key in ("rule", "path", "line")} for finding in findings])


def main() -> None:
    # --messages <directory>: also write each run's findings with PHPStan's messages there (for divergence review).
    arguments = sys.argv[1:]
    dump_directory = None
    if "--messages" in arguments:
        index = arguments.index("--messages")
        dump_directory = pathlib.Path(arguments[index + 1]).resolve()
        del arguments[index:index + 2]
    if arguments[0] == "fixtures":
        fixtures(pathlib.Path(arguments[1]).resolve(), pathlib.Path(arguments[2]).resolve(),
                 dump_directory / "phpstan-fixtures.json" if dump_directory else None)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
