"""Runs Psalm's unused-variable analysis over PHP corpora for `benchmark_php_usage_corpus`
(rerail/native/src/policy_dsl/scope/php/tests.rs). Offline, not in CI:

    python3 generate_usage_corpus.py <composer project with vimeo/psalm 6.19.1> <output.json> \
        <name>=<path>[,<path>...] ...

Psalm 6.19.1 runs at errorLevel 1 with `findUnusedVariablesAndParams` (`--find-unused-variables`),
as `generate_variables.py` runs it on the fixtures, under PHP 8.5.11 with no memory limit. Psalm
does not analyse a class whose parent or interface it cannot find, so each corpus's Composer
dependencies are installed beside the output (`deps-<name>/`, from the corpus's own
`composer.json`, its path repositories aside, `--no-scripts --no-plugins --ignore-platform-reqs`)
and given to Psalm as `extraFiles`: scanned for their declarations, not analysed. Files PHP
cannot parse (`php -l`), files Psalm crashes on (`PSALM_CRASHES`) and Blade templates are
excluded. Writes {"psalm", "php", "corpora": [{"name", "commit", "paths", "excluded",
"dependencies": {package: version}, "shadowed": [directory, ...], "reports": {file: [{"line",
"name", "type"}]}, "unanalysed": {file: [[start, end], ...]}, "suppressed": {file: [line, ...]}}]}:
a report's `type` is UnusedVariable, UnusedParam, UnusedClosureParam or UnusedForeachValue, its
`name` the variable without `$`; `shadowed` lists the vendor directories left out of
`extraFiles` (`shadowing`); `unanalysed` lists, per file, the byte ranges (end exclusive) Psalm
does not analyse, by its own reports (`unanalysed`); `suppressed` lists, per file, the lines
carrying `@psalm-suppress` of one of the usage types (or `all`), where Psalm drops a report.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from xml.sax.saxutils import quoteattr

USAGE_TYPES = {"UnusedVariable", "UnusedParam", "UnusedClosureParam", "UnusedForeachValue"}
# Where Psalm stops analysing a function, its reports there are not comparable. It reports one of
# these where it does, the analyser of the expression returning false; `unanalysed.php` reads
# which of them stop a function, and which function. The same types elsewhere stop nothing.
STOP_TYPES = {"UndefinedVariable", "PossiblyUndefinedVariable", "DuplicateParam", "UndefinedClass",
              "NullReference", "InvalidThrow"}
DUPLICATE = re.compile(r"Class (\S+) has already been defined")
SUPPRESS = re.compile(r"@psalm-suppress\s+([\w, ]+)")
# Files Psalm 6.19.1 crashes on (the whole run aborts), by path suffix: excluded from its run and
# from the comparison.
PSALM_CRASHES = {
    "Symfony/Component/TypeInfo/Tests/Fixtures/DummyWithTypeAliases.php":
        "`@phpstan-import-type Invalid from int`: InvalidArgumentException `Could not get class "
        "storage for symfony\\component\\typeinfo\\tests\\fixtures\\int` (FileAnalyzer.php:254)",
    "Symfony/Component/String/AbstractUnicodeString.php":
        "a private const with a ternary initializer (`EMOJI_REGEX`): `Undefined array key "
        "\"EMOJI_REGEX\"` (ClassConstAnalyzer.php:726)",
    "Symfony/Component/VarDumper/Tests/Dumper/CliDumperTest.php":
        "`(int) $var['res']` of the array `Fixtures/dumb-var.php` (required) builds, which holds "
        "`\\NAN`: `The float NAN is not representable as an int` (CastAnalyzer.php:369)",
    "Symfony/Component/VarDumper/Tests/Dumper/HtmlDumperTest.php":
        "as CliDumperTest.php: `(int) $var['res']` of `dumb-var.php`'s array (CastAnalyzer.php:369)",
}


def php_files(paths: list[pathlib.Path]) -> list[pathlib.Path]:
    return [path for root in paths for path in sorted(root.rglob("*.php"))
            if path.is_file() and not path.name.endswith(".blade.php")]


def unparsable(files: list[pathlib.Path]) -> list[str]:
    def rejects(path: pathlib.Path) -> bool:
        return subprocess.run(["php", "-l", str(path)], capture_output=True).returncode != 0
    with ThreadPoolExecutor(os.cpu_count()) as pool:
        return [str(path) for path, bad in zip(files, pool.map(rejects, files)) if bad]


def suppressions(files: list[pathlib.Path]) -> dict[str, list[int]]:
    found: dict[str, list[int]] = {}
    for path in files:
        text = path.read_text(errors="replace")
        if "@psalm-suppress" not in text:
            continue
        for index, line in enumerate(text.splitlines(), 1):
            for matched in SUPPRESS.finditer(line):
                types = {item.strip() for item in matched.group(1).replace(",", " ").split()}
                if types & (USAGE_TYPES | {"all"}):
                    found.setdefault(str(path), []).append(index)
    return found


# What a corpus's own test setup installs beyond its `composer.json`, by Composer package name:
# symfony/symfony's `./phpunit` runs symfony/phpunit-bridge's simple-phpunit, which installs
# PHPUnit `SYMFONY_PHPUNIT_VERSION=13`; without it Psalm analyses none of its test cases.
EXTRA_REQUIREMENTS = {"symfony/symfony": {"phpunit/phpunit": "^13"}}
# The version each checkout is (`COMPOSER_ROOT_VERSION`), which its `replace: self.version`
# packages take: laravel/framework's main branch (orchestra/testbench-core 11.5 conflicts with
# its `branch-alias` 13.0.x-dev), symfony/symfony's `Kernel::VERSION`.
ROOT_VERSIONS = {"laravel/framework": "13.x-dev", "symfony/symfony": "8.2.x-dev"}


def dependencies(repository: pathlib.Path, work: pathlib.Path) -> dict[str, str]:
    """Installs the corpus's Composer dependencies under `work`; returns what it installed."""
    work.mkdir(parents=True, exist_ok=True)
    manifest = json.loads((repository / "composer.json").read_text())
    # A path repository points into the checkout, whose packages the corpus's `replace` covers.
    manifest["repositories"] = [repo for repo in manifest.get("repositories", [])
                                if repo.get("type") != "path"]
    manifest.setdefault("require-dev", {}).update(EXTRA_REQUIREMENTS.get(manifest["name"], {}))
    (work / "composer.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if not (work / "composer.lock").exists():
        subprocess.run(["composer", "install", "--no-scripts", "--no-plugins",
                        "--ignore-platform-reqs", "--no-interaction", "--no-autoloader"],
                       cwd=work, check=True, capture_output=True,
                       env={**os.environ, "COMPOSER_ROOT_VERSION": ROOT_VERSIONS[manifest["name"]]})
    lock = json.loads((work / "composer.lock").read_text())
    return {package["name"]: package["version"]
            for package in lock["packages"] + lock["packages-dev"]}


def shadowing(repository: pathlib.Path, vendor: pathlib.Path) -> list[pathlib.Path]:
    """The dependencies that would shadow the corpus's own declarations: a package's stub
    directories (rector/rector's `stubs-rector` holds an empty `PHPUnit\\Framework\\TestCase`,
    under which Psalm finds no assertion method of any test case), and a package the checkout
    holds itself (a `composer.json` of its name in the checkout: symfony/contracts and
    symfony/phpunit-bridge are symfony/symfony's `src/Symfony/Contracts` and
    `src/Symfony/Bridge/PhpUnit`). Psalm reports a corpus class a dependency declares too
    `DuplicateClass`, or analyses whichever copy its scan meets first, which varies from run to
    run."""
    listed = subprocess.run(["git", "-C", str(repository), "ls-files", "*composer.json"],
                            capture_output=True, text=True, check=True).stdout.split()
    held = {json.loads((repository / path).read_text()).get("name") for path in listed}
    found = [path for path in sorted(vendor.glob("*/*/stubs*")) if path.is_dir()]
    for manifest in sorted(vendor.glob("*/*/composer.json")):
        if json.loads(manifest.read_text()).get("name") in held:
            found.append(manifest.parent)
    return found


def psalm(tools: pathlib.Path, work: pathlib.Path, paths: list[pathlib.Path],
          excluded: list[str], vendor: pathlib.Path, shadowed: list[pathlib.Path]) -> list[dict]:
    work.mkdir(parents=True, exist_ok=True)
    config = work / "psalm.xml"
    directories = "".join(f"<directory name={quoteattr(str(path))} />" for path in paths)
    ignored = "".join(f"<file name={quoteattr(path)} />" for path in excluded)
    left_out = "".join(f"<directory name={quoteattr(str(path))} />" for path in shadowed)
    config.write_text(
        '<?xml version="1.0"?>\n'
        '<psalm errorLevel="1" findUnusedVariablesAndParams="true" findUnusedCode="false"\n'
        '       findUnusedBaselineEntry="false" xmlns="https://getpsalm.org/schema/config">\n'
        f"    <projectFiles>{directories}<ignoreFiles>{ignored}</ignoreFiles></projectFiles>\n"
        f"    <extraFiles><directory name={quoteattr(str(vendor))} />"
        + (f"<ignoreFiles>{left_out}</ignoreFiles>" if left_out else "") + "</extraFiles>\n"
        "</psalm>\n")
    report = work / "psalm.json"
    run = subprocess.run(
        ["php", "-d", "memory_limit=-1", str(tools / "vendor/bin/psalm"), "--no-cache",
         "--no-progress", "--find-unused-variables", f"--threads={os.cpu_count()}",
         f"--config={config}", f"--root={work}", f"--report={report}", "--output-format=json"],
        capture_output=True, text=True, cwd=work,
    )
    # Psalm exits 2 when it reports issues; anything else is a crash.
    assert run.returncode in (0, 2), (run.stdout + run.stderr)[:2000]
    return json.loads(report.read_text() or "[]")


def reports(issues: list[dict]) -> dict[str, list[dict]]:
    """Psalm's usage reports by file."""
    found: dict[str, list[dict]] = {}
    for issue in issues:
        if issue["type"] not in USAGE_TYPES:
            continue
        found.setdefault(issue["file_path"], []).append(
            {"line": issue["line_from"], "name": issue["selected_text"].lstrip("&$."),
             "type": issue["type"]})
    for records in found.values():
        records.sort(key=lambda record: (record["line"], record["name"], record["type"]))
    return found


def unanalysed(tools: pathlib.Path, issues: list[dict],
               files: list[pathlib.Path]) -> dict[str, list[list[int]]]:
    """By file, the byte and line ranges (`[start, end, first line, last line]`, the end byte
    exclusive) of the code Psalm does not analyse, read from its reports by `unanalysed.php`:
    - the functions it stops analysing at one of its `STOP_TYPES` reports;
    - every declaration of a class it reports `DuplicateClass` for, and of each class extending
      one: it marks both copies as having visitor issues (ClassLikeNodeScanner::start,
      ClassLikeStorageProvider::addMore), a subclass inherits the mark
      (Populator::populateDataFromParentClass), and ClassAnalyzer::analyze analyses no class so
      marked. Two files declaring `class Dup` with an unused variable in a method each get one
      `DuplicateClass` and no `UnusedVariable`;
    - the whole of a file it reports `DuplicateClass` in: the scanner marks that file too, and
      FileAnalyzer::analyze analyses none of it.
    `files` are every file Psalm analyses (searched for the duplicated classes' declarations)."""
    stops: dict[str, list[list]] = {}
    duplicated: set[str] = set()
    marked_files: set[str] = set()
    for issue in issues:
        if issue["type"] in STOP_TYPES:
            stops.setdefault(issue["file_path"], []).append([issue["from"], issue["type"]])
        elif issue["type"] == "DuplicateClass":
            duplicated.add(DUPLICATE.match(issue["message"]).group(1).lower())
            marked_files.add(issue["file_path"])
    wanted = [str(path) for path in files] if duplicated else sorted(stops)
    chunks = [wanted[start:start + 400] for start in range(0, len(wanted), 400)]

    def read(chunk: list[str]) -> dict:
        run = subprocess.run(
            ["php", "-d", "memory_limit=-1", str(pathlib.Path(__file__).with_name("unanalysed.php")),
             str(tools / "vendor/autoload.php")],
            input=json.dumps({file: stops.get(file, []) for file in chunk}),
            capture_output=True, text=True, check=True)
        return json.loads(run.stdout)
    declared: dict[str, dict] = {}
    with ThreadPoolExecutor(max_workers=os.cpu_count()) as pool:
        for part in pool.map(read, chunks):
            declared.update({file: data for file, data in part.items() if data is not None})
    parents = {name: parent for data in declared.values()
               for name, parent, *_ in data["classes"] if name}
    marked = set(duplicated)
    for name in parents:
        ancestors = []
        while name is not None and name not in marked and name not in ancestors:
            ancestors.append(name)
            name = parents.get(name)
        if name in marked:
            marked.update(ancestors)
    found: dict[str, list[list[int]]] = {}
    for file, data in declared.items():
        if file in marked_files:
            ranges = [[0, data["bytes"], 1, data["lines"]]]
        else:
            ranges = data["stopped"] + [declaration[2:] for declaration in data["classes"]
                                        if declaration[0] in marked
                                        or (declaration[1] is not None and declaration[1] in marked)]
        if ranges:
            found[file] = sorted(ranges)
    return found


def main() -> None:
    tools = pathlib.Path(sys.argv[1]).resolve()
    output = pathlib.Path(sys.argv[2]).resolve()
    corpora = []
    for argument in sys.argv[3:]:
        name, _, paths = argument.partition("=")
        roots = [pathlib.Path(path).resolve() for path in paths.split(",")]
        files = php_files(roots)
        excluded = unparsable(files) + [str(path) for path in files
                                         if any(str(path).endswith(crash) for crash in PSALM_CRASHES)]
        repository = pathlib.Path(subprocess.run(
            ["git", "-C", str(roots[0]), "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, check=True).stdout.strip())
        installed = dependencies(repository, output.parent / f"deps-{name}")
        vendor = output.parent / f"deps-{name}" / "vendor"
        shadowed = shadowing(repository, vendor)
        issues = psalm(tools, output.parent / f"psalm-{name}", roots, excluded, vendor, shadowed)
        commit = subprocess.run(["git", "-C", str(roots[0]), "rev-parse", "HEAD"],
                                capture_output=True, text=True, check=True).stdout.strip()
        corpora.append({"name": name, "commit": commit, "paths": [str(root) for root in roots],
                        "excluded": excluded, "dependencies": installed,
                        "shadowed": [str(path.relative_to(vendor)) for path in shadowed],
                        "reports": reports(issues),
                        "unanalysed": {file: [ranges[:2] for ranges in found]
                                       for file, found in unanalysed(tools, issues, files).items()},
                        "suppressed": suppressions(files)})
    version = subprocess.run(["php", str(tools / "vendor/bin/psalm"), "--version"],
                             capture_output=True, text=True).stdout.strip()
    php = subprocess.run(["php", "-r", "echo PHP_VERSION;"], capture_output=True, text=True).stdout
    output.write_text(json.dumps({"psalm": version, "php": php, "corpora": corpora}, indent=1) + "\n")


if __name__ == "__main__":
    main()
