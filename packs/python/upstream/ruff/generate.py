"""Records Ruff 0.16.9 for the Python pack's ported rules (rerail/packs/python/conformance.json).
Run offline, never in CI, under CPython 3.14:

    python3.14 generate.py fixtures <Ruff checkout at tag 0.16.9> <ruff executable, 0.16.9>
    python3.14 generate.py corpus <ruff executable, 0.16.9> <name>=<corpus checkout> [...]

`fixtures` copies each rule's own test fixtures (the files its tests lint with default settings, under
`crates/ruff_linter/resources/test/fixtures/<linter>/`; see RULES) to fixtures/<linter>/<file> (the pack's
licenses/ruff/UPSTREAM.md lists them) and runs Ruff with every ported rule selected over every copy. `corpus` runs the
same rules over a corpus's pack paths, files CPython 3.14 does not parse left out. Both run Ruff isolated from any
configuration, at target version py310 (Ruff's CLI default and the engine's default `pythonVersion`, which the ports
keep), with `# noqa` honoured, as Ruff runs by default. Each finding is the rule's code, path, line and column
(Ruff's, 1-based, in characters)."""

from __future__ import annotations

import json
import pathlib
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

VERSION = "0.16.9"
ORIGIN = "https://github.com/astral-sh/ruff@0be08a206f9c3180afd3e93bcc792ed5cb1f4db1"
FIXTURES = "crates/ruff_linter/resources/test/fixtures"

# Each ported rule, shipped (rules/) or candidate (candidates/rules/), and its own default-settings fixtures: every
# .py and .pyi file the tests in `crates/ruff_linter/src/rules/<linter>/mod.rs` (and `src/linter.rs`'s syntax-error
# tests) lint with settings that enable the rule and change no option, preview mode or target version. Notebooks
# (.ipynb), which the host does not route to Python, are left out. Fixtures of pylint-derived (PL*) and flake8-builtins
# (A*) rules are GPL-derived and are never copied, so the candidate A005 is not recorded; nor is the candidate INP001,
# which Ruff's tests lint only with a `namespace-packages` option.
RULES: dict[str, list[str]] = {
    "ASYNC100": ["flake8_async/ASYNC100.py"],
    "ASYNC105": ["flake8_async/ASYNC105.py"],
    "ASYNC109": ["flake8_async/ASYNC109_0.py", "flake8_async/ASYNC109_1.py"],
    "ASYNC110": ["flake8_async/ASYNC110.py"],
    "ASYNC115": ["flake8_async/ASYNC115.py"],
    "ASYNC116": ["flake8_async/ASYNC116.py"],
    "ASYNC210": ["flake8_async/ASYNC210.py"],
    "ASYNC212": ["flake8_async/ASYNC212.py"],
    "ASYNC220": ["flake8_async/ASYNC22x.py"],
    "ASYNC221": ["flake8_async/ASYNC22x.py"],
    "ASYNC222": ["flake8_async/ASYNC22x.py"],
    "ASYNC230": ["flake8_async/ASYNC230.py"],
    "ASYNC240": ["flake8_async/ASYNC240.py"],
    "ASYNC250": ["flake8_async/ASYNC250.py"],
    "ASYNC251": ["flake8_async/ASYNC251.py"],
    "B002": ["flake8_bugbear/B002.py"],
    "B003": ["flake8_bugbear/B003.py"],
    "B004": ["flake8_bugbear/B004.py"],
    "B005": ["flake8_bugbear/B005.py"],
    "B006": [
        "flake8_bugbear/B006_1.py",
        "flake8_bugbear/B006_1.pyi",
        "flake8_bugbear/B006_2.py",
        "flake8_bugbear/B006_3.py",
        "flake8_bugbear/B006_4.py",
        "flake8_bugbear/B006_5.py",
        "flake8_bugbear/B006_6.py",
        "flake8_bugbear/B006_7.py",
        "flake8_bugbear/B006_8.py",
        "flake8_bugbear/B006_9.py",
        "flake8_bugbear/B006_B008.py",
    ],
    "B007": ["flake8_bugbear/B007.py"],
    "B008": ["flake8_bugbear/B006_B008.py"],
    "B009": ["flake8_bugbear/B009_B010.py"],
    "B010": ["flake8_bugbear/B009_B010.py"],
    "B011": ["flake8_bugbear/B011.py"],
    "B012": ["flake8_bugbear/B012.py"],
    "B013": ["flake8_bugbear/B013.py"],
    "B014": ["flake8_bugbear/B014.py"],
    "B015": ["flake8_bugbear/B015.py"],
    "B016": ["flake8_bugbear/B016.py"],
    "B017": ["flake8_bugbear/B017_0.py", "flake8_bugbear/B017_1.py"],
    "B018": ["flake8_bugbear/B018.py"],
    "B019": ["flake8_bugbear/B019.py"],
    "B020": ["flake8_bugbear/B020.py"],
    "B021": ["flake8_bugbear/B021.py"],
    "B022": ["flake8_bugbear/B022.py"],
    "B023": ["flake8_bugbear/B023.py"],
    "B024": ["flake8_bugbear/B024.py"],
    "B025": ["flake8_bugbear/B025.py"],
    "B026": ["flake8_bugbear/B026.py"],
    "B027": ["flake8_bugbear/B027.py", "flake8_bugbear/B027.pyi"],
    "B028": ["flake8_bugbear/B028.py"],
    "B029": ["flake8_bugbear/B029.py"],
    "B030": ["flake8_bugbear/B030.py"],
    "B031": ["flake8_bugbear/B031.py"],
    "B032": ["flake8_bugbear/B032.py"],
    "B033": ["flake8_bugbear/B033.py"],
    "B034": ["flake8_bugbear/B034.py"],
    "B035": ["flake8_bugbear/B035.py"],
    "B039": ["flake8_bugbear/B039.py"],
    "B904": ["flake8_bugbear/B904.py"],
    "B905": ["flake8_bugbear/B905.py"],
    "B911": ["flake8_bugbear/B911.py"],
    "B912": ["flake8_bugbear/B912.py"],
    "BLE001": ["flake8_blind_except/BLE.py"],
    "E402": [
        "pycodestyle/E40.py",
        "pycodestyle/E402_0.py",
        "pycodestyle/E402_1.py",
        "pycodestyle/E402_2.py",
        "pycodestyle/E402_3.py",
        "pycodestyle/E402_4.py",
        "pycodestyle/E402_5.py",
    ],
    "E711": ["pycodestyle/E711.py", "pycodestyle/constant_literals.py"],
    "E712": ["pycodestyle/E712.py", "pycodestyle/constant_literals.py"],
    "E713": ["pycodestyle/E713.py"],
    "E714": ["pycodestyle/E714.py"],
    "E721": ["pycodestyle/E721.py"],
    "E722": ["pycodestyle/E722.py"],
    "E731": ["pycodestyle/E731.py"],
    "E741": ["pycodestyle/E741.py", "pycodestyle/E741.pyi", "ruff/noqa.py"],
    "E742": ["pycodestyle/E742.py"],
    "E743": ["pycodestyle/E743.py"],
    "F401": [
        "pyflakes/F401_0.py",
        "pyflakes/F401_1.py",
        "pyflakes/F401_10.py",
        "pyflakes/F401_11.py",
        "pyflakes/F401_12.py",
        "pyflakes/F401_13.py",
        "pyflakes/F401_14.py",
        "pyflakes/F401_15.py",
        "pyflakes/F401_16.py",
        "pyflakes/F401_17.py",
        "pyflakes/F401_18.py",
        "pyflakes/F401_19.py",
        "pyflakes/F401_2.py",
        "pyflakes/F401_20.py",
        "pyflakes/F401_21.py",
        "pyflakes/F401_22.py",
        "pyflakes/F401_23.py",
        "pyflakes/F401_24/__init__.py",
        "pyflakes/F401_25__all_nonempty/__init__.py",
        "pyflakes/F401_26__all_empty/__init__.py",
        "pyflakes/F401_27__all_mistyped/__init__.py",
        "pyflakes/F401_28__all_multiple/__init__.py",
        "pyflakes/F401_29__all_conditional/__init__.py",
        "pyflakes/F401_3.py",
        "pyflakes/F401_32.py",
        "pyflakes/F401_34.py",
        "pyflakes/F401_35.py",
        "pyflakes/F401_4.py",
        "pyflakes/F401_5.py",
        "pyflakes/F401_6.py",
        "pyflakes/F401_7.py",
        "pyflakes/F401_8.py",
        "pyflakes/F401_9.py",
        "pyflakes/__init__.py",
        "pyflakes/future_annotations.py",
        "pyflakes/multi_statement_lines.py",
        "ruff/RUF050_F401.py",
        "ruff/RUF100_1.py",
        "ruff/RUF100_2.py",
        "ruff/RUF100_4.py",
        "ruff/flake8_noqa.py",
        "ruff/ruff_noqa_all.py",
        "ruff/ruff_noqa_codes.py",
        "ruff/ruff_noqa_invalid.py",
    ],
    "F402": ["pyflakes/F402.py"],
    "F403": ["pyflakes/F403.py"],
    "F404": ["pyflakes/F404_0.py", "pyflakes/F404_1.py", "syntax_errors/late_future_import.py"],
    "F405": ["pyflakes/F405.py"],
    "F406": ["pyflakes/F406.py"],
    "F407": ["pyflakes/F407.py"],
    "F501": ["pyflakes/F50x.py"],
    "F502": ["pyflakes/F502.py", "pyflakes/F50x.py"],
    "F503": ["pyflakes/F503.py", "pyflakes/F50x.py"],
    "F504": ["pyflakes/F504.py", "pyflakes/F50x.py"],
    "F505": ["pyflakes/F504.py", "pyflakes/F50x.py"],
    "F506": ["pyflakes/F50x.py"],
    "F507": ["pyflakes/F50x.py"],
    "F508": ["pyflakes/F50x.py"],
    "F509": ["pyflakes/F50x.py"],
    "F521": ["pyflakes/F521.py"],
    "F522": ["pyflakes/F522.py"],
    "F523": ["pyflakes/F523.py"],
    "F524": ["pyflakes/F524.py"],
    "F525": ["pyflakes/F525.py"],
    "F541": ["pyflakes/F541.py", "ruff/RUF100_7.py"],
    "F601": ["pyflakes/F601.py"],
    "F602": ["pyflakes/F602.py"],
    "F622": ["pyflakes/F622.py"],
    "F631": ["pyflakes/F631.py"],
    "F632": ["pycodestyle/constant_literals.py", "pyflakes/F632.py"],
    "F633": ["pyflakes/F633.py"],
    "F634": ["pyflakes/F634.py"],
    "F701": ["pyflakes/F701.py"],
    "F702": ["pyflakes/F702.py"],
    "F704": ["pyflakes/F704.py", "syntax_errors/yield_scope.py"],
    "F706": ["pyflakes/F706.py", "syntax_errors/return_outside_function.py"],
    "F707": ["pyflakes/F707.py"],
    "F722": ["pyflakes/F722.py", "pyflakes/F722_1.py"],
    "F811": [f"pyflakes/F811_{n}.py" for n in [*range(0, 29), 30, 31, 32, 35, 36]]
    + ["pyflakes/F811_29.pyi", "pyflakes/F811_33.pyi"],
    "F821": [
        "pyflakes/F821_0.py",
        "pyflakes/F821_1.py",
        "pyflakes/F821_10.py",
        "pyflakes/F821_11.py",
        "pyflakes/F821_11.pyi",
        "pyflakes/F821_12.py",
        "pyflakes/F821_13.py",
        "pyflakes/F821_14.py",
        "pyflakes/F821_15.py",
        "pyflakes/F821_16.py",
        "pyflakes/F821_17.py",
        "pyflakes/F821_18.py",
        "pyflakes/F821_19.py",
        "pyflakes/F821_2.py",
        "pyflakes/F821_20.py",
        "pyflakes/F821_21.py",
        "pyflakes/F821_23.py",
        "pyflakes/F821_24.py",
        "pyflakes/F821_25.py",
        "pyflakes/F821_26.py",
        "pyflakes/F821_26.pyi",
        "pyflakes/F821_27.py",
        "pyflakes/F821_28.py",
        "pyflakes/F821_3.py",
        "pyflakes/F821_30.py",
        "pyflakes/F821_31.py",
        "pyflakes/F821_32.pyi",
        "pyflakes/F821_33.py",
        "pyflakes/F821_34.py",
        "pyflakes/F821_34.pyi",
        "pyflakes/F821_4.py",
        "pyflakes/F821_5.py",
        "pyflakes/F821_5.pyi",
        "pyflakes/F821_6.py",
        "pyflakes/F821_7.py",
        "pyflakes/F821_8.pyi",
        "pyflakes/F821_9.py",
        "pyflakes/__init__.py",
        "pyflakes/builtins.py",
        "pyflakes/future_annotations.py",
        "pyflakes/typing_modules.py",
        "ruff/RUF100_3.py",
    ],
    "F822": [
        "pyflakes/F822_0.py",
        "pyflakes/F822_0.pyi",
        "pyflakes/F822_1.py",
        "pyflakes/F822_1b.py",
        "pyflakes/F822_2.py",
        "pyflakes/F822_3.py",
        "pyflakes/__init__.py",
    ],
    "F823": ["pyflakes/F823.py"],
    "F841": [
        "pyflakes/F841_0.py",
        "pyflakes/F841_1.py",
        "pyflakes/F841_2.py",
        "pyflakes/F841_3.py",
        "ruff/RUF100_7.py",
        "ruff/flake8_noqa.py",
        "ruff/noqa.py",
        "ruff/ruff_noqa_all.py",
        "ruff/ruff_noqa_codes.py",
        "ruff/ruff_noqa_invalid.py",
    ],
    "F842": ["pyflakes/F842.py"],
    "F901": ["pyflakes/F901.py"],
    "N999": [
        "pep8_naming/N999/module/MODULE/__init__.py",
        "pep8_naming/N999/module/MODULE/file.py",
        "pep8_naming/N999/module/flake9/__init__.py",
        "pep8_naming/N999/module/invalid_name/0001_initial.py",
        "pep8_naming/N999/module/invalid_name/import.py",
        "pep8_naming/N999/module/mod with spaces/__init__.py",
        "pep8_naming/N999/module/mod with spaces/file.py",
        "pep8_naming/N999/module/mod-with-dashes/__init__.py",
        "pep8_naming/N999/module/valid_name/__init__.py",
        "pep8_naming/N999/module/valid_name/__main__.py",
        "pep8_naming/N999/module/valid_name/__setup__.py",
        "pep8_naming/N999/module/valid_name/file-with-dashes.py",
    ],
    "PIE790": ["flake8_pie/PIE790.py", "flake8_pyi/PYI021_1.pyi"],
    "PIE794": ["flake8_pie/PIE794.py"],
    "PIE796": ["flake8_pie/PIE796.py", "flake8_pie/PIE796.pyi"],
    "PIE800": ["flake8_pie/PIE800.py"],
    "PIE804": ["flake8_pie/PIE804.py"],
    "PIE807": ["flake8_pie/PIE807.py"],
    "PIE808": ["flake8_pie/PIE808.py"],
    "PIE810": ["flake8_pie/PIE810.py"],
    "RET501": ["flake8_return/RET501.py"],
    "RET502": ["flake8_return/RET502.py"],
    "RET503": ["flake8_return/RET503.py"],
    "RET504": ["flake8_return/RET504.py"],
    "RET505": ["flake8_return/RET505.py"],
    "RET506": ["flake8_return/RET506.py"],
    "RET507": ["flake8_return/RET507.py"],
    "RET508": ["flake8_return/RET508.py"],
    "SIM101": ["flake8_simplify/SIM101.py"],
    "SIM102": ["flake8_simplify/SIM102.py"],
    "SIM103": ["flake8_simplify/SIM103.py"],
    "SIM105": [
        "flake8_simplify/SIM105_0.py",
        "flake8_simplify/SIM105_1.py",
        "flake8_simplify/SIM105_2.py",
        "flake8_simplify/SIM105_3.py",
        "flake8_simplify/SIM105_4.py",
        "ruff/RUF072_RUF047_SIM105.py",
        "ruff/RUF072_SIM105.py",
    ],
    "SIM107": ["flake8_simplify/SIM107.py"],
    "SIM108": ["flake8_simplify/SIM108.py"],
    "SIM109": ["flake8_simplify/SIM109.py"],
    "SIM110": ["flake8_simplify/SIM110.py", "flake8_simplify/SIM111.py"],
    "SIM112": ["flake8_simplify/SIM112.py"],
    "SIM113": ["flake8_simplify/SIM113.py"],
    "SIM114": ["flake8_simplify/SIM114.py"],
    "SIM115": ["flake8_simplify/SIM115.py"],
    "SIM116": ["flake8_simplify/SIM116.py"],
    "SIM117": ["flake8_simplify/SIM117.py"],
    "SIM118": ["flake8_simplify/SIM118.py"],
    "SIM201": ["flake8_simplify/SIM201.py"],
    "SIM202": ["flake8_simplify/SIM202.py"],
    "SIM208": ["flake8_simplify/SIM208.py"],
    "SIM210": ["flake8_simplify/SIM210.py"],
    "SIM211": ["flake8_simplify/SIM211.py"],
    "SIM212": ["flake8_simplify/SIM212.py"],
    "SIM220": ["flake8_simplify/SIM220.py"],
    "SIM221": ["flake8_simplify/SIM221.py"],
    "SIM222": ["flake8_simplify/SIM222.py"],
    "SIM223": ["flake8_simplify/SIM223.py"],
    "SIM300": ["flake8_simplify/SIM300.py"],
    "SIM401": ["flake8_simplify/SIM401.py"],
    "SIM905": ["flake8_simplify/SIM905.py"],
    "SIM910": ["flake8_simplify/SIM910.py"],
    "SIM911": ["flake8_simplify/SIM911.py"],
    "T100": ["flake8_debugger/T100.py"],
    "W605": ["pycodestyle/W605_0.py", "pycodestyle/W605_1.py"],
}


def ruff(executable: str, paths: list[str], cwd: pathlib.Path) -> list[dict]:
    """The ported rules' findings over `paths` (relative to `cwd`), in batches."""
    findings = []
    for start in range(0, len(paths), 500):
        run = subprocess.run(
            [executable, "check", "--isolated", "--no-cache", "--exit-zero", "--output-format", "json",
             "--target-version", common.TARGET_VERSION, "--select", ",".join(sorted(RULES)),
             *paths[start:start + 500]],
            capture_output=True, text=True, cwd=cwd, check=True,
        )
        for diagnostic in json.loads(run.stdout):
            if diagnostic["code"] == "invalid-syntax":
                continue
            assert diagnostic["code"] in RULES, diagnostic
            findings.append({
                "rule": diagnostic["code"],
                "path": pathlib.Path(diagnostic["filename"]).resolve().relative_to(cwd.resolve()).as_posix(),
                "line": diagnostic["location"]["row"],
                "column": diagnostic["location"]["column"],
            })
    return findings


def fixtures(checkout: pathlib.Path, executable: str) -> None:
    version = common.tool_version(executable, VERSION)
    head = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != ORIGIN.rsplit("@", 1)[1]:
        raise SystemExit(f"{checkout} is at {head}, not Ruff {VERSION}")
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    files = sorted({file for paths in RULES.values() for file in paths})
    for file in files:
        (target / file).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(checkout / FIXTURES / file, target / file)
    common.write(HERE / "fixtures.expected.json", "ruff", version,
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(RULES), "files": files},
                 ruff(executable, files, target))


def corpus(executable: str, name: str, root: pathlib.Path) -> None:
    version = common.tool_version(executable, VERSION)
    files = common.corpus_files(root, name)
    skipped = common.unparsable(root, files)
    analysed = sorted(set(files) - set(skipped))
    common.write(HERE / f"corpus-{name}.expected.json", "ruff", version,
                 common.corpus_source(name, sorted(RULES), skipped), ruff(executable, analysed, root))


def main() -> None:
    common.require_python()
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), sys.argv[3])
    else:
        for argument in sys.argv[3:]:
            name, _, path = argument.partition("=")
            corpus(sys.argv[2], name, pathlib.Path(path).resolve())


if __name__ == "__main__":
    main()
