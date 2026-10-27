"""Records tlint 9.7.0 for the Blade pack's ported lints (rerail/packs/blade/conformance.json). Run by
`npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request CI), with
PHP 8.5.11:

    python3 generate.py fixtures <tighten/tlint checkout at tag v9.7.0> <composer project with tightenco/tlint 9.7.0>

`fixtures` runs each ported lint's own test (tests/Linting/Linters/<Lint>Test.php) with read_tests.php: every
template the test hands `new <Lint>(...)` is captured, and every assertion the test makes is checked against tlint's
output on it. Each template is written to fixtures/<Lint>Test/<n>.blade.php (n counts the test class's templates in
method order; the pack's licenses/tlint/UPSTREAM.md lists them, and LICENSE.md is copied beside it). run.php then runs every
ported lint over every copy, as `tlint lint` runs a file, and the recording on a lint's own copy is required to be
exactly the lines its test asserts, in order (`$lints[0]->getNode()->getLine()`, `$lints[1]`, ...; none where it
asserts `assertEmpty($lints)`): so the recording is the upstream test's expectation, and on the other lints' copies
the tool's output. Each finding is the lint's class name, the path and its node's line (the `line` of
`tlint lint --json`). `source.units` names each copy's origin: the lint whose test hands it the template. None of the
lints takes configuration; tlint's `tighten` preset enables all six, its `laravel` preset the last three."""

from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

VERSION = "9.7.0"
COMMIT = "e768bedce2bd84930712f9412ecbf2d4c586735c"
ORIGIN = f"https://github.com/tighten/tlint@{COMMIT}"
LINTS = [
    "NoDumpDirectives",
    "NoJsonDirective",
    "NoRayDirective",
    "NoSpaceAfterBladeDirectives",
    "SpaceAfterBladeDirectives",
    "SpacesAroundBladeRenderContent",
]
LICENSES = HERE.parents[1] / "licenses" / "tlint"


def php(*arguments: str, stdin: str | None = None) -> dict | list:
    """Runs a PHP driver beside this script without the agent variables PHP tools print a banner for."""
    environment = {key: os.environ[key] for key in ("PATH", "HOME", "TMPDIR", "LANG") if key in os.environ}
    result = subprocess.run(["php", "-d", "memory_limit=-1", *arguments], input=stdin, capture_output=True, text=True,
                            env=environment)
    if result.returncode != 0:
        raise SystemExit(f"php {' '.join(arguments)} failed:\n{result.stdout}\n{result.stderr}")
    return json.loads(result.stdout)


def run(project: pathlib.Path, root: pathlib.Path, files: list[str]) -> tuple[list[dict], list[str]]:
    output = php(str(HERE / "run.php"), str(project), str(root), *LINTS, stdin="\n".join(files))
    if output["version"] != VERSION:
        raise SystemExit(f"{project} has tlint {output['version']}, not {VERSION}")
    return output["findings"], output["skipped"]


def fixtures(checkout: pathlib.Path, project: pathlib.Path) -> None:
    head = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != COMMIT:
        raise SystemExit(f"{checkout} is at {head}, not v{VERSION} ({COMMIT})")
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    cases = []  # (file, lint, upstream test file, test method, assertions)
    for lint in LINTS:
        test = pathlib.Path("tests/Linting/Linters") / f"{lint}Test.php"
        for method in php(str(HERE / "read_tests.php"), str(project), str(checkout / test), lint):
            if len(method["templates"]) != 1:
                raise SystemExit(f"{test}::{method['test']} hands {lint} {len(method['templates'])} templates")
            file = f"{lint}Test/{sum(case[1] == lint for case in cases) + 1}.blade.php"
            (target / file).parent.mkdir(parents=True, exist_ok=True)
            (target / file).write_bytes(method["templates"][0].encode())
            cases.append((file, lint, test.as_posix(), method["test"], method["assertions"]))
    files = [case[0] for case in cases]
    findings, skipped = run(project, target, files)
    if skipped:
        raise SystemExit(f"tlint failed on {skipped}")
    for file, lint, test, method, assertions in cases:
        # The tests assert $lints[0], $lints[1], ... in order (or assertEmpty): the lint's lines, in its order.
        reported = [f["line"] for f in findings if f["path"] == file and f["rule"] == lint]
        lines = [assertion["equals"] for assertion in assertions if "equals" in assertion]
        if reported != lines:
            raise SystemExit(f"{test}::{method} asserts lines {lines}; {lint} reports {reported} on {file}")
    common.write(HERE / "fixtures.expected.json", "tlint", VERSION,
                 {"kind": "fixtures", "origin": ORIGIN, "rules": LINTS, "files": files,
                  "units": {file: {"rules": [lint]} for file, lint, *_ in sorted(cases)}}, findings)
    write_licenses(checkout, cases)


def write_licenses(checkout: pathlib.Path, cases: list[tuple]) -> None:
    LICENSES.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(checkout / "LICENSE.md", LICENSES / "LICENSE.md")
    rows = "".join(f"| `upstream/tlint/fixtures/{file}` | `{test}` | the template test `{method}` passes to "
                   f"`new {lint}(...)`, captured verbatim |\n" for file, lint, test, method, _ in cases)
    (LICENSES / "UPSTREAM.md").write_text(f"""# tlint {VERSION}

Upstream: [tighten/tlint](https://github.com/tighten/tlint/tree/{COMMIT}) at tag `v{VERSION}` (commit `{COMMIT}`). License: MIT (Copyright (c) Tighten Co.); [`LICENSE.md`](LICENSE.md) is the upstream license file, copied verbatim at that tag.

The files listed below are the templates the tests of the lints the Blade pack ports (the pack's `conformance.json`) hand those lints, and remain under tlint's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of the validators and lints of stillat/blade-parser, Bladestan and tlint, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the recorders `upstream/common.py` and `upstream/tlint/` (`generate.py`, `read_tests.php`, `run.php`), and the recordings they write by running tlint {VERSION}.

Paths are relative to the pack directory, `packs/blade/`. Each fixture is written by `upstream/tlint/generate.py fixtures`, which runs the test with the template captured as the linter receives it.

| File | Upstream path | Copy |
|---|---|---|
{rows}""")


def main() -> None:
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), pathlib.Path(sys.argv[3]).resolve())
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
