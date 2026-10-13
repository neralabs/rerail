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

# Each ported rule and its own default-settings fixtures: every file the tests in
# `crates/ruff_linter/src/rules/<linter>/mod.rs` (and `src/linter.rs`'s syntax-error tests) lint with settings that
# enable the rule and change no option, preview mode or target version. Notebooks (.ipynb), which the host does not
# route to Python, are left out. Fixtures of pylint-derived (PL*) and flake8-builtins (A*) rules are GPL-derived and are
# never copied; no ported rule here needs one.
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
    "B007": ["flake8_bugbear/B007.py"],
    "B009": ["flake8_bugbear/B009_B010.py"],
    "B010": ["flake8_bugbear/B009_B010.py"],
    "B011": ["flake8_bugbear/B011.py"],
    "B013": ["flake8_bugbear/B013.py"],
    "B014": ["flake8_bugbear/B014.py"],
    "B016": ["flake8_bugbear/B016.py"],
    "B017": ["flake8_bugbear/B017_0.py", "flake8_bugbear/B017_1.py"],
    "B019": ["flake8_bugbear/B019.py"],
    "B020": ["flake8_bugbear/B020.py"],
    "B021": ["flake8_bugbear/B021.py"],
    "B022": ["flake8_bugbear/B022.py"],
    "B023": ["flake8_bugbear/B023.py"],
    "B025": ["flake8_bugbear/B025.py"],
    "B026": ["flake8_bugbear/B026.py"],
    "B028": ["flake8_bugbear/B028.py"],
    "B029": ["flake8_bugbear/B029.py"],
    "B032": ["flake8_bugbear/B032.py"],
    "B033": ["flake8_bugbear/B033.py"],
    "B034": ["flake8_bugbear/B034.py"],
    "B039": ["flake8_bugbear/B039.py"],
    "B904": ["flake8_bugbear/B904.py"],
    "B905": ["flake8_bugbear/B905.py"],
    "E711": ["pycodestyle/E711.py", "pycodestyle/constant_literals.py"],
    "E712": ["pycodestyle/E712.py", "pycodestyle/constant_literals.py"],
    "E713": ["pycodestyle/E713.py"],
    "E714": ["pycodestyle/E714.py"],
    "E721": ["pycodestyle/E721.py"],
    "E722": ["pycodestyle/E722.py"],
    "E731": ["pycodestyle/E731.py"],
    "E742": ["pycodestyle/E742.py"],
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
    "F601": ["pyflakes/F601.py"],
    "F602": ["pyflakes/F602.py"],
    "F622": ["pyflakes/F622.py"],
    "F631": ["pyflakes/F631.py"],
    "F632": ["pycodestyle/constant_literals.py", "pyflakes/F632.py"],
    "F633": ["pyflakes/F633.py"],
    "F634": ["pyflakes/F634.py"],
    "F701": ["pyflakes/F701.py"],
    "F702": ["pyflakes/F702.py"],
    "F706": ["pyflakes/F706.py", "syntax_errors/return_outside_function.py"],
    "F707": ["pyflakes/F707.py"],
    "F722": ["pyflakes/F722.py", "pyflakes/F722_1.py"],
    "F811": [f"pyflakes/F811_{n}.py" for n in [*range(0, 29), 30, 31, 32, 35, 36]]
    + ["pyflakes/F811_29.pyi", "pyflakes/F811_33.pyi"],
    "F823": ["pyflakes/F823.py"],
    "F842": ["pyflakes/F842.py"],
    "F901": ["pyflakes/F901.py"],
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
    "RET504": ["flake8_return/RET504.py"],
    "RET505": ["flake8_return/RET505.py"],
    "RET506": ["flake8_return/RET506.py"],
    "RET507": ["flake8_return/RET507.py"],
    "RET508": ["flake8_return/RET508.py"],
    "SIM101": ["flake8_simplify/SIM101.py"],
    "SIM107": ["flake8_simplify/SIM107.py"],
    "SIM109": ["flake8_simplify/SIM109.py"],
    "SIM112": ["flake8_simplify/SIM112.py"],
    "SIM113": ["flake8_simplify/SIM113.py"],
    "SIM115": ["flake8_simplify/SIM115.py"],
    "SIM116": ["flake8_simplify/SIM116.py"],
    "SIM201": ["flake8_simplify/SIM201.py"],
    "SIM202": ["flake8_simplify/SIM202.py"],
    "SIM208": ["flake8_simplify/SIM208.py"],
    "SIM210": ["flake8_simplify/SIM210.py"],
    "SIM211": ["flake8_simplify/SIM211.py"],
    "SIM212": ["flake8_simplify/SIM212.py"],
    "SIM220": ["flake8_simplify/SIM220.py"],
    "SIM221": ["flake8_simplify/SIM221.py"],
    "SIM905": ["flake8_simplify/SIM905.py"],
    "SIM910": ["flake8_simplify/SIM910.py"],
    "T100": ["flake8_debugger/T100.py"],
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
