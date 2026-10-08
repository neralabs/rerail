"""Records stillat/blade-parser 2.1.0 for the Blade pack's ported validators (rerail/conformance/languages/blade/pack.json).
Run offline, never in CI, with PHP 8.5.11 (laravel/framework 13.34.0 installed beside it):

    python3 generate.py fixtures <stillat/blade-parser checkout at tag v2.1.0> <composer project with stillat/blade-parser 2.1.0>
    python3 generate.py corpus <composer project> <name>=<corpus checkout> [...]

`fixtures` runs each ported validator's own test (tests/Validation/<X>Test.php, a Pest file) with read_tests.php:
every template the test hands Document::fromText() is captured, for every row of the test's dataset (`->with(...)`,
read from the running parser, e.g. CoreDirectiveRetriever's debug directive names), and every expectation the test
makes is checked against the validator's output on it. Each distinct template of a test file is written to
fixtures/<X>Test/<n>.blade.php (n counts them in Pest's order; licenses/blade-parser/UPSTREAM.md lists them, and
license.md is copied beside it). run.php then runs every ported validator over every copy, as the tests run one, and
the recording on a validator's own copy is required to be the test's expectation: exactly the count of errors its
`toHaveCount()` expects, with every message its `toBe()` expects. `corpus` runs the same validators over a corpus's
pack paths, in parallel batches. Each finding is the validator's class name, the path and the start line of the node
the validation error reports (its BladeError position). Validators are constructed as the tests construct them (no
custom or ignored directives), and the parser runs on a bare container (no Laravel application booted): it knows
Blade's core directives and no custom one."""

from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import sys
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

VERSION = "2.1.0"
COMMIT = "23a962f99bca451f8738ed2475a70129fb33f07c"
ORIGIN = f"https://github.com/stillat/blade-parser@{COMMIT}"
# Ported validator -> its test (tests/Validation/<test>.php).
VALIDATORS = {
    "ComponentShorthandVariableParameterValidator": "ComponentShorthandVariableParameterTest",
    "DebugDirectiveValidator": "DebugDirectiveTest",
    "DuplicateConditionExpressionsValidator": "DuplicateConditionExpressionsTest",
    "ForElseStructureValidator": "ForElseStructureTest",
    "NoArgumentsValidator": "NoArgumentsTest",
    "RequiredArgumentsValidator": "RequiredArgumentsTest",
}
LICENSES = HERE.parents[1] / "licenses" / "blade-parser"


def php(*arguments: str, stdin: str | None = None) -> dict | list:
    """Runs a PHP driver beside this script without the agent variables PHP tools print a banner for."""
    environment = {key: os.environ[key] for key in ("PATH", "HOME", "TMPDIR", "LANG") if key in os.environ}
    result = subprocess.run(["php", "-d", "memory_limit=-1", *arguments], input=stdin, capture_output=True, text=True,
                            env=environment)
    if result.returncode != 0:
        raise SystemExit(f"php {' '.join(arguments)} failed:\n{result.stdout}\n{result.stderr}")
    return json.loads(result.stdout)


def run(project: pathlib.Path, root: pathlib.Path, files: list[str]) -> tuple[list[dict], list[str]]:
    """The ported validators' findings (with their messages) on `files`, and the files the parser throws on."""
    batches = [files[start:start + 25] for start in range(0, len(files), 25)]
    with ThreadPoolExecutor(max_workers=8) as pool:
        outputs = list(pool.map(lambda batch: php(str(HERE / "run.php"), str(project), str(root), *VALIDATORS,
                                                  stdin="\n".join(batch)), batches))
    findings, skipped = [], []
    for output in outputs:
        if output["version"] != VERSION:
            raise SystemExit(f"{project} has stillat/blade-parser {output['version']}, not {VERSION}")
        findings += output["findings"]
        skipped += output["skipped"]
    return findings, skipped


def fixtures(checkout: pathlib.Path, project: pathlib.Path) -> None:
    head = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != COMMIT:
        raise SystemExit(f"{checkout} is at {head}, not v{VERSION} ({COMMIT})")
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    files: list[str] = []
    copies: list[tuple] = []  # (file, test file, test, dataset) for UPSTREAM.md: the first run giving each template
    runs: list[tuple] = []  # (file, validator, test file, test, dataset, expectations): every run, for verification
    for validator, name in VALIDATORS.items():
        test = pathlib.Path("tests/Validation") / f"{name}.php"
        written: dict[str, str] = {}
        for entry in php(str(HERE / "read_tests.php"), str(project), str(checkout), str(checkout / test)):
            where = f"{test}: {entry['test']} {entry['dataset'] or ''}"
            if len(entry["templates"]) != 1:
                raise SystemExit(f"{where} hands Document::fromText() {len(entry['templates'])} templates")
            template = entry["templates"][0]
            if template not in written:
                file = written[template] = f"{name}/{len(written) + 1}.blade.php"
                (target / file).parent.mkdir(parents=True, exist_ok=True)
                (target / file).write_bytes(template.encode())
                files.append(file)
                copies.append((file, test.as_posix(), entry["test"], entry["dataset"]))
            runs.append((written[template], validator, where, entry["expectations"]))
    findings, skipped = run(project, target, files)
    if skipped:
        raise SystemExit(f"stillat/blade-parser failed on {skipped}")
    for file, validator, where, expectations in runs:
        reported = [f for f in findings if f["path"] == file and f["rule"] == validator]
        for expectation in expectations:
            if "count" in expectation and expectation["count"] != len(reported):
                raise SystemExit(f"{where} expects {expectation['count']} errors; {validator} reports "
                                 f"{len(reported)} on {file}")
        messages = Counter(expectation["be"] for expectation in expectations if "be" in expectation)
        if messages - Counter(f["message"] for f in reported):
            raise SystemExit(f"{where} expects messages {dict(messages)}; {validator} reports "
                             f"{[f['message'] for f in reported]} on {file}")
    common.write(HERE / "fixtures.expected.json", "blade-parser", VERSION,
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(VALIDATORS), "files": files},
                 [{key: f[key] for key in ("rule", "path", "line")} for f in findings])
    write_licenses(checkout, copies)


def write_licenses(checkout: pathlib.Path, copies: list[tuple]) -> None:
    LICENSES.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(checkout / "license.md", LICENSES / "license.md")
    rows = "".join(f"| `rules/blade-parser/fixtures/{file}` | `{test}` | the template test `{name}` passes to "
                   f"`Document::fromText()`{f' (dataset `{dataset}`)' if dataset else ''}, captured verbatim |\n"
                   for file, test, name, dataset in copies)
    (LICENSES / "UPSTREAM.md").write_text(f"""# stillat/blade-parser {VERSION}

Upstream: [stillat/blade-parser](https://github.com/stillat/blade-parser/tree/{COMMIT}) at tag `v{VERSION}` (commit `{COMMIT}`). License: MIT (Copyright (c) Johnathon Koster); [`license.md`](license.md) is the upstream license file, copied verbatim at that tag.

The files listed below are the templates the tests of the validators the Blade pack ports (Rerail's `conformance/languages/blade/pack.json`) hand the parser, and remain under stillat/blade-parser's license. Every other file under `rules/blade-parser/` is Rerail's, under the Apache License 2.0 (this repository's `LICENSE`): `generate.py`, `read_tests.php`, `run.php` and the recordings they write by running stillat/blade-parser {VERSION}.

Paths are relative to `blade/`. Each fixture is written by `rules/blade-parser/generate.py fixtures`, which runs the test with the template captured as the parser receives it; a template several dataset rows of a test file build alike is written once, under the first.

| File | Upstream path | Copy |
|---|---|---|
{rows}""")


def corpus(project: pathlib.Path, name: str, root: pathlib.Path) -> None:
    findings, skipped = run(project, root, common.corpus_files(root, name))
    common.write(HERE / f"corpus-{name}.expected.json", "blade-parser", VERSION,
                 common.corpus_source(name, list(VALIDATORS), skipped),
                 [{key: f[key] for key in ("rule", "path", "line")} for f in findings])


def main() -> None:
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), pathlib.Path(sys.argv[3]).resolve())
    elif sys.argv[1] == "corpus":
        project = pathlib.Path(sys.argv[2]).resolve()
        for argument in sys.argv[3:]:
            name, _, path = argument.partition("=")
            corpus(project, name, pathlib.Path(path).resolve())
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
