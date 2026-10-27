"""Records Bladestan 0.11.7 (PHPStan 2.2.16 over the PHP illuminate/view 13.34.0 compiles a template to) for the
Blade pack's ported PHPStan rules (rerail/packs/blade/conformance.json). Run by `npm run fixtures:derive` (a
maintainer, or the bump workflow for an upstream release; never pull request CI), with PHP 8.5.11, from a checkout of
TomasVotruba/bladestan at tag 0.11.7 with its development dependencies installed and
PHPStan pinned (`composer update && composer require --dev phpstan/phpstan:2.2.16 -W`; laravel/framework 13.34.0,
larastan 3.12.3, livewire 4.4.7, orchestra/testbench 11.3.0, phpunit 12.5.38):

    python3 generate.py fixtures <bladestan checkout>

It runs Bladestan's own rule test harness (RecordTest.php: PHPStan's RuleTestCase at level 8 with
tests/Rules/config/configured_extension.neon, as tests/Rules/BladeRuleTest.php runs it). Bladestan analyses a
template where PHP renders it (`view('name', [...])`) with the data the call passes, and reports each error of the
compiled template at its template file and line.

`fixtures` runs every fixture of BladeRuleTest::provideData() and requires its errors to be the test's expectation
(message and caller line, every one), so the recording is the upstream test's. It copies the test's templates
(tests/skeleton/resources) to fixtures/skeleton/resources (the pack's licenses/bladestan/UPSTREAM.md lists them), one analysis
unit each.
A finding is the identifier, the template and the template line. Where a template is analysed more than once (each
fixture renders foo.blade.php, some several times; an included template is inlined into its includer), each
(identifier, line) counts as often as the render reporting it most often reports it, so a line is neither lost nor
counted once per render. Only the ported identifiers are recorded (`IDENTIFIERS`), with the message of the rule the
pack ports where another reports the same identifier.

BladeRuleTest is one test for every rule, and its templates are shared by its fixtures, so no template belongs to a
rule by where it lies. `source.units` names a template's origin by the test's own assertions instead: the ported
identifiers BladeRuleTest expects on it (every finding is one, as the harness requires). A template the test expects
no ported identifier on has no entry."""

from __future__ import annotations

import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

VERSION = "0.11.7"
ORIGIN = "https://github.com/TomasVotruba/bladestan@ceb0823cd4b4f400b64a008cace0d91e786a2859"
# Ported identifier -> the message pattern of the ported rule (PHPStan 2.2.16).
IDENTIFIERS = {
    # DefinedVariableRule: "Undefined variable: $x"; not "Variable $x might not be defined." (some paths).
    "variable.undefined": r"^Undefined variable: \$",
    # PrintfParametersRule.
    "argument.sprintf": r"^Call to sprintf contains ",
    # DuplicateKeysInLiteralArraysRule.
    "array.duplicateKey": r"^Array has \d+ duplicate keys? with value ",
    # RandomIntParametersRule; argument.type is every rule's parameter type error.
    "argument.type": r"^Parameter #1 \$min \(.*\) of function random_int expects lower number than parameter #2 \$max",
    # IfConstantConditionRule.
    "if.alwaysTrue": r"^If condition is always true\.$",
    "if.alwaysFalse": r"^If condition is always false\.$",
    # MissingFunctionReturnTypehintRule.
    "missingType.return": r"^Function .+ has no return type specified\.$",
    # StrictComparisonOfDifferentTypesRule.
    "identical.alwaysTrue": r"^Strict comparison using === between .+ will always evaluate to true\.$",
}


def ported(error: dict) -> bool:
    pattern = IDENTIFIERS.get(error["identifier"])
    return pattern is not None and re.match(pattern, error["message"]) is not None


def environment() -> dict[str, str]:
    """PHP's environment without the agent variables PHPStan prints a banner for."""
    keep = ("PATH", "HOME", "TMPDIR", "LANG")
    return {key: os.environ[key] for key in keep if key in os.environ}


def record(checkout: pathlib.Path, request: dict) -> list[dict]:
    """Runs RecordTest.php in `checkout` on `request` and returns its errors."""
    with tempfile.TemporaryDirectory() as scratch:
        path = pathlib.Path(scratch) / "request.json"
        path.write_text(json.dumps(request))
        result = subprocess.run(
            ["php", "-d", "memory_limit=-1", "vendor/bin/phpunit", "--no-configuration", "--bootstrap",
             "vendor/autoload.php", str(HERE / "RecordTest.php")],
            cwd=checkout, capture_output=True, text=True, env={**environment(), "BLADESTAN_RECORD": str(path)})
        output = path.with_name("request.json.out.json")
        if result.returncode != 0 or not output.exists():
            raise SystemExit(f"Bladestan's harness failed:\n{result.stdout}\n{result.stderr}")
        return json.loads(output.read_text())["errors"]


def merge(errors: list[dict], relative) -> list[dict]:
    """The ported findings: per template, each (identifier, line) as often as the render reporting it most often
    reports it. A render is a `view(...)` call: the caller and its line, where Bladestan reports the call's errors.
    `relative` maps a template's path to the recording's, or None for a template outside it."""
    per_render: dict[tuple[str, int, str], Counter] = defaultdict(Counter)
    for error in errors:
        if error["template"] is None or not ported(error):
            continue
        path = relative(error["template"])
        if path is not None:
            per_render[(error["caller"], error["line"], path)][(error["identifier"], error["templateLine"])] += 1
    most: dict[tuple[str, str, int], int] = defaultdict(int)
    for (_, _, path), counts in per_render.items():
        for (identifier, line), count in counts.items():
            most[(identifier, path, line)] = max(most[(identifier, path, line)], count)
    return [{"rule": identifier, "path": path, "line": line}
            for (identifier, path, line), count in sorted(most.items()) for _ in range(count)]


def fixtures(checkout: pathlib.Path) -> None:
    tests = checkout / "tests"
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    files = []
    for template in sorted((tests / "skeleton" / "resources").rglob("*.blade.php")):
        relative = template.relative_to(tests).as_posix()
        (target / relative).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(template, target / relative)
        files.append(relative)
    resolved = tests.resolve()

    def relative(path: str) -> str | None:
        template = pathlib.Path(path).resolve()
        return template.relative_to(resolved).as_posix() if template.is_relative_to(resolved) else None

    findings = merge(record(checkout, {"mode": "fixtures"}), relative)
    expected = {path: sorted({f["rule"] for f in findings if f["path"] == path}) for path in files}
    common.write(HERE / "fixtures.expected.json", "bladestan", VERSION,
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(IDENTIFIERS), "files": files,
                  "units": {path: {"rules": rules} for path, rules in sorted(expected.items()) if rules}},
                 findings)


def main() -> None:
    checkout = pathlib.Path(sys.argv[2]).resolve()
    if sys.argv[1] == "fixtures":
        fixtures(checkout)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
