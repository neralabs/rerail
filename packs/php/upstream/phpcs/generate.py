"""Records PHP_CodeSniffer 4.0.4 for the PHP pack's ported sniffs (rerail/packs/php/conformance.json).
Run by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request CI),
with PHP 8.5.11:

    python3 generate.py fixtures <PHP_CodeSniffer checkout at tag 4.0.4> <composer project with squizlabs/php_codesniffer 4.0.4>

`fixtures` copies each sniff's own test files (`src/Standards/<Standard>/Tests/<Category>/<Sniff>UnitTest*.inc`) to
fixtures/<Standard>/<Category>/<file>.php (the runner reads `.php`; the pack's licenses/php_codesniffer/UPSTREAM.md lists them),
runs PHP_CodeSniffer with every ported sniff (ruleset.xml: the sniffs alone, with their default properties) over every
copy, and requires its reports on a sniff's own files to be,
line by line, the counts its test's getErrorList() and getWarningList() expect: so the recording on a sniff's own
files is the upstream test's expectation, and on the other sniffs' files the tool's output. Each finding is the
sniff (`Standard.Category.Sniff`), path, line and column (PHP_CodeSniffer's, in characters, tab width 0).
`source.units` names each copy's origin: the sniff whose test it is."""

from __future__ import annotations

import json
import pathlib
import shutil
import subprocess
import sys
from collections import Counter

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

SNIFFS = [
    "Generic.CodeAnalysis.EmptyStatement",
    "Generic.PHP.BacktickOperator",
    "Generic.PHP.DeprecatedFunctions",
    "Generic.PHP.ForbiddenFunctions",
    "Generic.PHP.LowerCaseConstant",
    "Generic.PHP.NoSilencedErrors",
    "Squiz.PHP.DisallowSizeFunctionsInLoops",
    "Squiz.PHP.Eval",
]
ORIGIN = "https://github.com/PHPCSStandards/PHP_CodeSniffer@bbdc3d0532623e21838b7041a4364383a8126f96"


def phpcs(project: pathlib.Path, paths: list[str], cwd: pathlib.Path) -> tuple[list[dict], set[str]]:
    """The ported sniffs' findings, and the files PHP_CodeSniffer could not analyse (an `Internal.*` message, such as
    `Internal.NoCodeFound` for a file without a PHP open tag the install accepts)."""
    run = subprocess.run(
        [str(project / "vendor/bin/phpcs"), "--report=json", "--no-cache", "-q", f"--standard={HERE / 'ruleset.xml'}",
         "--tab-width=0", "--extensions=php", "--runtime-set", "ignore_warnings_on_exit",
         "1", "-d", "memory_limit=-1", *paths],
        capture_output=True, text=True, cwd=cwd,
    )
    report = json.loads(run.stdout)
    findings = []
    unanalysed = set()
    for path, file in report["files"].items():
        relative = pathlib.Path(path).resolve().relative_to(cwd.resolve()).as_posix()
        for message in file["messages"]:
            if message["source"].startswith("Internal."):
                unanalysed.add(relative)
                continue
            sniff = ".".join(message["source"].split(".")[:3])
            assert sniff in SNIFFS, message
            findings.append({"rule": sniff, "path": relative, "line": message["line"], "column": message["column"]})
    return findings, unanalysed


def version(project: pathlib.Path) -> str:
    text = subprocess.run([str(project / "vendor/bin/phpcs"), "--version"], capture_output=True, text=True,
                          check=True).stdout
    return text.split("version ")[1].split()[0]


def fixtures(checkout: pathlib.Path, project: pathlib.Path) -> None:
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    owned: dict[str, str] = {}
    tests: dict[str, pathlib.Path] = {}
    for sniff in SNIFFS:
        standard, category, name = sniff.split(".")
        test = checkout / "src/Standards" / standard / "Tests" / category / f"{name}UnitTest.php"
        for source in sorted(test.parent.glob(f"{name}UnitTest*.inc")):
            copy = pathlib.Path(standard) / category / f"{source.name}.php"
            (target / copy).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target / copy)
            owned[copy.as_posix()] = sniff
            tests[copy.as_posix()] = test
    files = sorted(owned)
    findings, unanalysed = phpcs(project, files, target)
    assert not unanalysed, unanalysed
    for file, sniff in owned.items():
        lists = json.loads(subprocess.run(["php", str(HERE / "error_lists.php"), str(tests[file]),
                                           pathlib.Path(file).name.removesuffix(".php")],
                                          capture_output=True, text=True, check=True).stdout)
        expected = Counter({int(line): count for kind in ("errors", "warnings")
                            for line, count in lists[kind].items()})
        reported = Counter(f["line"] for f in findings if f["path"] == file and f["rule"] == sniff)
        assert +expected == +reported, (file, dict(expected), dict(reported))
    common.write(HERE / "fixtures.expected.json", "phpcs", version(project),
                 {"kind": "fixtures", "origin": ORIGIN, "rules": SNIFFS, "files": files,
                  "units": {file: {"rules": [owned[file]]} for file in files}}, findings)


def main() -> None:
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), pathlib.Path(sys.argv[3]).resolve())
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
