"""Records Ruff 0.16.9 for the Python pack's ported rules (rerail/packs/python/conformance.json).
Run offline, never in CI, under CPython 3.14:

    python3.14 -I -B generate.py fixtures <Ruff checkout at tag 0.16.9> <ruff executable, 0.16.9>
    python3.14 -I -B generate.py corpus <ruff executable, 0.16.9> <name>=<corpus checkout> [...]

`fixtures` copies each rule's own test fixtures (the files its tests lint with default settings and only rules
selected, under `crates/ruff_linter/resources/test/fixtures/<linter>/`) to fixtures/<linter>/<file>
(the pack's licenses/ruff/UPSTREAM.md lists them) and runs Ruff with every ported rule selected over every copy. `corpus` runs the
same rules over a corpus's pack paths, files CPython 3.14 does not parse left out. Both run Ruff isolated from any
configuration, at target version py310 (Ruff's CLI default and the engine's default `pythonVersion`, which the ports
keep), with `# noqa` honoured: the ports honour it too (`suppress.noqa`). Each finding is the rule's code, path, line
and column, and its end's line and column (Ruff's `location` and `end_location`: 1-based, in code points, the end
exclusive).

The facts checks 7 to 9 read about Ruff, upstream/ruff/source.json, are recorded from the same checkout, from the repository root, by
`node --import tsx conformance/recorders/python-source.ts ruff <Ruff checkout> <ruff executable, 0.16.9>`."""

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

# Each ported rule and its default-settings fixtures: the files its tests (`crates/ruff_linter/src/rules/<linter>/mod.rs`,
# and the `ruff` linter's `noqa` tests) lint with only rules selected. The rule's other tests are source.json's
# `unscored`: notebooks the host does not route to Python (F821_22.ipynb), and files linted only with other settings
# (options, preview).
RULES: dict[str, list[str]] = {
    "B011": ["flake8_bugbear/B011.py"],
    "B028": ["flake8_bugbear/B028.py"],
    "B905": ["flake8_bugbear/B905.py"],
    "E711": ["pycodestyle/E711.py", "pycodestyle/constant_literals.py"],
    "E722": ["pycodestyle/E722.py"],
    "F507": ["pyflakes/F50x.py"],
    "F523": ["pyflakes/F523.py"],
    "F524": ["pyflakes/F524.py"],
    "F601": ["pyflakes/F601.py"],
    "F632": ["pyflakes/F632.py", "pycodestyle/constant_literals.py"],
    "F722": ["pyflakes/F722.py", "pyflakes/F722_1.py"],
    "F811": [f"pyflakes/F811_{n}.py" for n in [*range(0, 29), 30, 31, 32, 35, 36]]
    + ["pyflakes/F811_29.pyi", "pyflakes/F811_33.pyi"],
    "F821": [f"pyflakes/F821_{n}.py" for n in [*range(0, 8), *range(9, 22), *range(23, 32), 33, 34]]
    + [f"pyflakes/F821_{stub}.pyi" for stub in ["5", "8", "11", "26", "32", "34"]]
    + ["pyflakes/__init__.py", "pyflakes/builtins.py", "pyflakes/future_annotations.py", "pyflakes/typing_modules.py", "ruff/RUF100_3.py"],
    "F823": ["pyflakes/F823.py"],
    "F841": ["pyflakes/F841_0.py", "pyflakes/F841_1.py", "pyflakes/F841_2.py", "pyflakes/F841_3.py", "ruff/RUF100_7.py", "ruff/flake8_noqa.py",
             "ruff/noqa.py", "ruff/ruff_noqa_all.py", "ruff/ruff_noqa_codes.py", "ruff/ruff_noqa_invalid.py"],
    "PIE796": ["flake8_pie/PIE796.py", "flake8_pie/PIE796.pyi"],
    "SIM222": ["flake8_simplify/SIM222.py"],
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
            assert diagnostic["end_location"] is not None, diagnostic
            findings.append({
                "rule": diagnostic["code"],
                "path": pathlib.Path(diagnostic["filename"]).resolve().relative_to(cwd.resolve()).as_posix(),
                "line": diagnostic["location"]["row"],
                "column": diagnostic["location"]["column"],
                "endLine": diagnostic["end_location"]["row"],
                "endColumn": diagnostic["end_location"]["column"],
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
