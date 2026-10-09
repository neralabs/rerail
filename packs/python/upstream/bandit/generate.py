"""Records Bandit 1.8.6 for the Python pack's ported tests (rerail/packs/python/conformance.json).
Run offline, never in CI, under CPython 3.14 (Bandit itself runs from its own install, on CPython 3.13):

    python3.14 -I -B generate.py fixtures <Bandit checkout at tag 1.8.6> <bandit executable, 1.8.6>
    python3.14 -I -B generate.py corpus <bandit executable, 1.8.6> <name>=<corpus checkout> [...]

`fixtures` copies the examples Bandit reports each test on (`examples/<file>`: the file its functional test scans, and any other) to
fixtures/<file> (the pack's licenses/bandit/UPSTREAM.md lists them) and runs Bandit with every ported test over every copy.
`corpus` runs the same tests over a corpus's pack paths, files CPython 3.14 does not parse left out. Both run with
`--ignore-nosec` (Rerail does not model `# nosec`, so the ports keep every finding) and every severity and confidence.
A file Bandit reports an error for is skipped. Each finding is the test id, path and line, and the column and end
column of the node the test reported (`col_offset` and `end_col_offset`, UTF-8 bytes from 0, made 1-based code points).
Bandit reports neither line those offsets lie on: its line is the node's first unless the test names another (a call
check that reports a keyword argument's line keeps the call's columns), and its `line_range` is the node's lines, or
for a check on a string (`Str`, `Bytes`) the lines of the string's parent. So the recorder parses the file and finds
the node: the one inside `line_range` with both offsets, whose first and last lines are where the column and end
column are counted, and it refuses a finding whose offsets fit no node or nodes on different lines. The node's last
line is not recorded: Bandit does not report it as the finding's end, and the end column is compared without a line.

The facts checks 7 to 9 read about Bandit, upstream/bandit/source.json, are recorded from the same checkout, from the repository root, by
`node --import tsx conformance/recorders/python-source.ts bandit <Bandit checkout> <python of Bandit 1.8.6's environment>`."""

from __future__ import annotations

import ast
import json
import pathlib
import shutil
import subprocess
import sys
import warnings

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

VERSION = "1.8.6"
ORIGIN = "https://github.com/PyCQA/bandit@2d0b675b04c80ae42277e10500db06a0a37bae17"

# Each ported test and the examples Bandit reports it on (source.json `tests`): its own example
# (tests/functional/test_functional.py) and any other.
TESTS: dict[str, list[str]] = {
    "B101": ["assert.py"],
    "B113": ["requests-missing-timeout.py"],
    "B505": ["weak_cryptographic_key_sizes.py"],
    "B506": ["yaml_load.py", "new_candidates-all.py"],
    "B610": ["django_sql_injection_extra.py"],
}


def node_lines(tree: ast.AST, result: dict) -> tuple[int, int]:
    """The first and last lines of the node a Bandit result reports: the node inside its `line_range` with its
    `col_offset` and `end_col_offset`. Nodes that share both offsets and lines (an expression statement and its call)
    are one answer; anything else is refused rather than guessed."""
    first, last = result["line_range"][0], result["line_range"][-1]
    found = {
        (node.lineno, node.end_lineno)
        for node in ast.walk(tree)
        if isinstance(node, (ast.expr, ast.stmt)) and node.end_lineno is not None
        and first <= node.lineno <= node.end_lineno <= last
        and node.col_offset == result["col_offset"] and node.end_col_offset == result["end_col_offset"]
    }
    if len(found) != 1:
        raise SystemExit(f"{result['filename']}:{result['line_number']}: {result['test_id']}'s offsets "
                         f"{result['col_offset']}-{result['end_col_offset']} fit {len(found)} node position(s) in lines "
                         f"{first}-{last}: {sorted(found)}")
    return found.pop()


def bandit(executable: str, paths: list[str], cwd: pathlib.Path) -> tuple[list[dict], set[str]]:
    """The ported tests' findings over `paths` (relative to `cwd`), and the files Bandit could not analyse."""
    findings = []
    errors: set[str] = set()
    for start in range(0, len(paths), 500):
        run = subprocess.run(
            [executable, "-q", "-f", "json", "--ignore-nosec", "-t", ",".join(sorted(TESTS)),
             *paths[start:start + 500]],
            capture_output=True, text=True, cwd=cwd,
        )
        report = json.loads(run.stdout)
        errors |= {pathlib.Path(error["filename"]).as_posix() for error in report["errors"]}
        trees: dict[str, tuple[list[str], ast.AST]] = {}
        for result in report["results"]:
            assert result["test_id"] in TESTS, result
            assert result["line_range"][0] <= result["line_number"] <= result["line_range"][-1], result
            if result["filename"] not in trees:
                path = cwd / result["filename"]
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore")
                    trees[result["filename"]] = (common.source_lines(path), ast.parse(path.read_bytes()))
            lines, tree = trees[result["filename"]]
            start, end = node_lines(tree, result)
            findings.append({"rule": result["test_id"], "path": pathlib.Path(result["filename"]).as_posix(),
                             "line": result["line_number"],
                             "column": common.code_point_column(lines, start, result["col_offset"]),
                             "endColumn": common.code_point_column(lines, end, result["end_col_offset"])})
    return [finding for finding in findings if finding["path"] not in errors], errors


def fixtures(checkout: pathlib.Path, executable: str) -> None:
    version = common.tool_version(executable, VERSION)
    head = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != ORIGIN.rsplit("@", 1)[1]:
        raise SystemExit(f"{checkout} is at {head}, not Bandit {VERSION}")
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    target.mkdir()
    files = sorted({file for paths in TESTS.values() for file in paths})
    for file in files:
        shutil.copyfile(checkout / "examples" / file, target / file)
    findings, errors = bandit(executable, files, target)
    assert not errors, errors
    common.write(HERE / "fixtures.expected.json", "bandit", version,
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(TESTS), "files": files}, findings)


def corpus(executable: str, name: str, root: pathlib.Path) -> None:
    version = common.tool_version(executable, VERSION)
    files = common.corpus_files(root, name)
    skipped = set(common.unparsable(root, files))
    findings, errors = bandit(executable, sorted(set(files) - skipped), root)
    common.write(HERE / f"corpus-{name}.expected.json", "bandit", version,
                 common.corpus_source(name, sorted(TESTS), sorted(skipped | errors)), findings)


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
