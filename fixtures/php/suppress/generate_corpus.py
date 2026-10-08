"""Records what each local suppression annotation in pinned PHP corpora drops from PHPStan 2.2.16,
Psalm 6.19.1 and PHP_CodeSniffer 4.0.4, for `benchmark_php_suppression_corpus`
(rerail/native/src/code_rule_dsl/suppress/php_corpus_tests.rs). Offline, not in CI:

    python3 generate_corpus.py <tools dir> <environment dir> <output.json> \
        <name>=<corpus root>:<path>[,<path>...][@<composer root version>] ...

From the checkout root:

    python3 rerail-fixtures/php/suppress/generate_corpus.py .rerail/tools/php \
        .rerail/t3/suppression/env .rerail/t3/suppression.input.json \
        laravel=.rerail/corpora/php/laravel:src,tests symfony=.rerail/corpora/php/symfony:src@8.2.x-dev

`<tools dir>` is a Composer project with phpstan/phpstan 2.2.16, vimeo/psalm 6.19.1 and
squizlabs/php_codesniffer 4.0.4; PHP 8.5.11 runs them. Files PHP cannot parse (`php -l`) and Blade
templates are excluded. PHP's tokenizer inventories every comment token holding `@phpstan-ignore`
(`-line`, `-next-line`), `@psalm-suppress`, `phpcs:` (any case, `@` optional) or
`@codingStandardsIgnore*`. Each corpus is cloned (`cp -c`) to `<environment dir>/<name>` and its
Composer dependencies installed there (`COMPOSER_ROOT_VERSION` from `@...`), so the tools see the
classes the annotated code uses; the installed versions are recorded.

Each annotated file is analysed alone, from its clone, by the tool of its annotation: as written
(`kept`), with every such annotation neutralised (`all`: the marker renamed in place, offsets
unchanged; PHP_CodeSniffer's `--ignore-annotations`), and, for PHPStan and Psalm, with only one
annotation neutralised (`single`, one list per annotation). PHPStan runs at level max with
phpstan/phpstan-deprecation-rules 2.0.5 (installed in `<environment dir>/phpstan-extensions`;
symfony's static analysis loads it), together with the first corpus class using the file's trait
where the file declares one (PHPStan analyses a trait only in a class's context; that class's own
reports are dropped, recorded as `context`); Psalm at errorLevel 1 with
`--find-unused-psalm-suppress`; PHP_CodeSniffer with the PSR12 standard. No project configuration
(baselines, ignoreErrors, issue handlers) applies. A report in the analysed file is [identifier,
line, start byte, message, context] (the start of the line where the tool reports lines only,
PHPStan and PHP_CodeSniffer; context: the class PHPStan analysed a trait in, each one apart, or
""); one the tool places in another file (a parent class's property) is [file,
identifier, line, message], in `allElsewhere` / `keptElsewhere`. Writes {"phpstan",
"phpstanExtensions", "psalm", "phpcs", "php", "corpora": [{"name", "root", "commit", "paths",
"excluded", "files", "dependencies", "annotations": {tool: {file: [[form, line, start, end], ...]}},
"runs": {tool: {file: {"all", "kept", "single", "allElsewhere", "keptElsewhere", "context"?}}}}]},
files relative to the corpus root.
"""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys

INVENTORY = r"""
$found = [];
foreach (preg_split('/\n/', trim(stream_get_contents(STDIN))) as $file) {
    $offset = 0;
    $namespace = '';
    $tokens = token_get_all(file_get_contents($file));
    foreach ($tokens as $index => $token) {
        $text = is_array($token) ? $token[1] : $token;
        if (is_array($token) && in_array($token[0], [T_NAMESPACE, T_TRAIT], true)) {
            $name = $tokens[$index + 2] ?? null;
            if (is_array($name) && in_array($name[0], [T_STRING, T_NAME_QUALIFIED], true)) {
                if ($token[0] === T_NAMESPACE) {
                    $namespace = $name[1] . '\\';
                } else {
                    $found[$file]['traits'][] = $namespace . $name[1];
                }
            }
        }
        if (is_array($token) && in_array($token[0], [T_COMMENT, T_DOC_COMMENT], true)) {
            $end = $offset + strlen($text);
            $doc = $token[0] === T_DOC_COMMENT ? 'doc' : 'comment';
            if (preg_match_all('/@phpstan-ignore(?:-next-line|-line)?/', $text, $m)) {
                $found[$file]['phpstan'][] = [implode(',', $m[0]), $token[2], $offset, $end];
            }
            if (str_contains($text, '@psalm-suppress')) {
                $found[$file]['psalm'][] = [$doc, $token[2], $offset, $end];
            }
            if (preg_match_all('/@?phpcs:\w*|@codingStandardsIgnore\w*/i', $text, $m)) {
                $found[$file]['phpcs'][] = [implode(',', $m[0]), $token[2], $offset, $end];
            }
        }
        $offset += strlen($text);
    }
}
echo json_encode($found, JSON_UNESCAPED_SLASHES);
"""

# PHPStan's deprecation rules, which symfony's static analysis loads (its `method.deprecated`
# ignores have no rule to match without them), installed apart from PHPStan and loaded by
# `--autoload-file` from their own sources.
EXTENSIONS = ["phpstan/phpstan:2.2.16", "phpstan/phpstan-deprecation-rules:2.0.5"]

EXTENSION_AUTOLOAD = """<?php
spl_autoload_register(function (string $class): void {
    $file = '{src}/' . str_replace('\\\\', '/', substr($class, strlen('PHPStan\\\\'))) . '.php';
    if (str_starts_with($class, 'PHPStan\\\\') && is_file($file)) {
        require $file;
    }
});
"""

# Each tool's marker and its same-length neutral spelling.
NEUTRAL = {
    "phpstan": (re.compile(r"@phpstan-ignore"), "@xxxxxxx-ignore"),
    "psalm": (re.compile(r"@psalm-suppress"), "@xsalm-suppress"),
    "phpcs": (re.compile(r"phpcs:|codingStandardsIgnore", re.IGNORECASE), None),
}

PSALM_CONFIG = """<?xml version="1.0"?>
<psalm errorLevel="1" resolveFromConfigFile="true" findUnusedCode="false"
       findUnusedBaselineEntry="false" findUnusedIssueHandlerSuppression="false">
    <projectFiles><file name="{file}" /></projectFiles>
</psalm>
"""


def clean_env() -> dict[str, str]:
    # PHPStan prints an agent banner before its JSON when it sees agent variables.
    return {"PATH": os.environ["PATH"], "HOME": os.environ["HOME"]}


def php_files(root: pathlib.Path, paths: list[str]) -> list[str]:
    found = []
    for path in paths:
        for file in (root / path).rglob("*.php"):
            if file.is_file() and not file.name.endswith(".blade.php"):
                found.append(str(file.relative_to(root)))
    return sorted(found)


def lints(path: pathlib.Path) -> bool:
    return subprocess.run(["php", "-l", str(path)], capture_output=True).returncode == 0


def inventory(root: pathlib.Path, files: list[str]) -> tuple[dict[str, dict[str, list]], dict[str, list[str]]]:
    """Each tool's annotations by file, and the traits each file declares."""
    run = subprocess.run(["php", "-r", INVENTORY], input="\n".join(str(root / f) for f in files),
                         capture_output=True, text=True, check=True)
    found = json.loads(run.stdout) or {}
    tools: dict[str, dict[str, list]] = {"phpstan": {}, "psalm": {}, "phpcs": {}}
    traits: dict[str, list[str]] = {}
    for file, kinds in found.items():
        relative = str(pathlib.Path(file).relative_to(root))
        for kind, entries in kinds.items():
            (traits if kind == "traits" else tools[kind])[relative] = entries
    return tools, traits


def trait_user(root: pathlib.Path, files: list[str], trait: str) -> str | None:
    """The first corpus class that uses `trait` (imported, or from its own namespace): PHPStan
    analyses a trait only in the context of a class using it."""
    namespace, _, short = trait.rpartition("\\")
    imported = re.compile(rf"^use\s+\\?{re.escape(trait)}\s*;", re.MULTILINE)
    local = re.compile(rf"^namespace\s+{re.escape(namespace)}\s*;", re.MULTILINE)
    used = re.compile(rf"^\s+use\s+[^;]*\b{re.escape(short)}\b[^;]*;", re.MULTILINE)
    declares = re.compile(r"^(?:(?:final|abstract|readonly)\s+)*class\s", re.MULTILINE)
    for file in files:
        text = (root / file).read_text(errors="replace")
        if short in text and (imported.search(text) or local.search(text)) and used.search(text) \
                and declares.search(text):
            return file
    return None


def extensions(env_root: pathlib.Path) -> tuple[pathlib.Path, pathlib.Path, list[list[str]]]:
    """The deprecation rules' `rules.neon`, their autoloader, and the installed versions."""
    project = env_root / "phpstan-extensions"
    if not (project / "vendor/composer/installed.json").exists():
        project.mkdir(parents=True, exist_ok=True)
        (project / "composer.json").write_text("{}\n")
        subprocess.run(["composer", "require", "--no-interaction", "--no-progress", "--no-plugins",
                        "--ignore-platform-reqs", *EXTENSIONS], cwd=project, check=True, capture_output=True)
    rules = project / "vendor/phpstan/phpstan-deprecation-rules"
    autoload = project / "autoload.php"
    autoload.write_text(EXTENSION_AUTOLOAD.replace("{src}", str(rules / "src")))
    installed = json.loads((project / "vendor/composer/installed.json").read_text())
    return rules / "rules.neon", autoload, sorted(
        [package["name"], package["version"]] for package in installed["packages"])


def environment(root: pathlib.Path, env: pathlib.Path, root_version: str | None) -> list[list[str]]:
    if not (env / "vendor/composer/installed.json").exists():
        if env.exists():
            shutil.rmtree(env)
        env.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["cp", "-cR", str(root), str(env)], check=True)
        extra = {"COMPOSER_ROOT_VERSION": root_version} if root_version else {}
        subprocess.run(["composer", "install", "--no-interaction", "--no-scripts", "--no-plugins",
                        "--ignore-platform-reqs", "--prefer-dist", "--no-progress"],
                       cwd=env, env={**os.environ, **extra}, check=True, capture_output=True)
    installed = json.loads((env / "vendor/composer/installed.json").read_text())
    return sorted([package["name"], package["version"],
                   (package.get("dist") or package.get("source") or {}).get("reference", "")]
                  for package in installed["packages"])


def line_starts(source: bytes) -> list[int]:
    starts = [0]
    for at, byte in enumerate(source):
        if byte == 0x0A:
            starts.append(at + 1)
    return starts


def neutralise(source: bytes, annotations: list[list], tool: str) -> bytes:
    pattern, neutral = NEUTRAL[tool]
    out = bytearray(source)
    for _, _, start, end in annotations:
        comment = source[start:end].decode("latin-1")
        out[start:end] = pattern.sub(neutral, comment).encode("latin-1")
    assert len(out) == len(source)
    return bytes(out)


def split(env: pathlib.Path, file: str, found: list[tuple[str, str, int, int | None, str]]) -> tuple[list, list]:
    """A run's reports (path, identifier, line, start byte or None, message): those in the analysed
    file as [identifier, line, start, message, context] (the line's start where the tool reports
    lines only; the class PHPStan analysed a trait in, or ""), and those it places in another file
    (a parent class's property) as [file, identifier, line, message]."""
    starts = line_starts((env / file).read_bytes())
    here, elsewhere = [], []
    for path, identifier, line, start, message in found:
        path, _, context = path.partition(" (in context of ")
        if pathlib.Path(path).resolve() == (env / file).resolve():
            here.append([identifier, line, starts[line - 1] if start is None else start, message,
                         context.removesuffix(")")])
            assert starts[line - 1] <= here[-1][2]
        else:
            located = pathlib.Path(path).resolve()
            if located.is_relative_to(env.resolve()):
                located = located.relative_to(env.resolve())
            elsewhere.append([str(located), identifier, line, message])
    return sorted(here), sorted(elsewhere)


def phpstan(tools: pathlib.Path, env: pathlib.Path, file: str, context: str | None, work: pathlib.Path,
            extension: tuple[pathlib.Path, pathlib.Path]) -> tuple[list, list]:
    rules, autoload = extension
    work.mkdir(parents=True, exist_ok=True)
    config = work / "phpstan.neon"
    config.write_text(f"includes:\n    - {rules}\nparameters:\n    level: max\n    tmpDir: {work / 'cache'}\n")
    paths = [str(env / file)] + ([str(env / context)] if context else [])
    run = subprocess.run([str(tools / "vendor/bin/phpstan"), "analyse", "-c", str(config), "--autoload-file",
                          str(autoload), "--no-progress", "--error-format=json", "--memory-limit=8G", *paths],
                         cwd=env, capture_output=True, text=True, env=clean_env())
    data = json.loads(run.stdout)
    assert not data["errors"], (file, data["errors"])
    # The context class's own reports are another analysed file's, not the annotated file's.
    return split(env, file, [
        (path, str(message.get("identifier", "")), int(message["line"]), None,
         message["message"])
        for path, messages in data["files"].items() for message in messages["messages"]
        if not (context and path == str(env / context))
    ])


def psalm(tools: pathlib.Path, env: pathlib.Path, file: str) -> tuple[list, list]:
    config = env / f"psalm.t3-suppression-{hashlib.sha256(file.encode()).hexdigest()[:12]}.xml"
    config.write_text(PSALM_CONFIG.format(file=file))
    try:
        run = subprocess.run([str(tools / "vendor/bin/psalm"), "-c", str(config), "--no-cache", "--no-progress",
                              "--threads=1", "--output-format=json", "--find-unused-psalm-suppress"],
                             cwd=env, capture_output=True, text=True, env=clean_env())
    finally:
        config.unlink()
    # Psalm places an issue at a byte offset; Rerail receives it there.
    return split(env, file, [
        (issue["file_path"], issue["type"], int(issue["line_from"]), int(issue["from"]), issue["message"])
        for issue in json.loads(run.stdout)
    ])


def phpcs(tools: pathlib.Path, env: pathlib.Path, file: str, *flags: str) -> tuple[list, list]:
    run = subprocess.run([str(tools / "vendor/bin/phpcs"), "--report=json", "--no-cache", "-q",
                          "--standard=PSR12", *flags, str(env / file)],
                         cwd=env, capture_output=True, text=True, env=clean_env())
    data = json.loads(run.stdout)
    return split(env, file, [
        (path, str(message["source"]), int(message["line"]), None, message["message"])
        for path, messages in data["files"].items() for message in messages["messages"]
    ])


def record(tool: str, tools: pathlib.Path, env: pathlib.Path, file: str, annotations: list[list],
           context: str | None, work: pathlib.Path, extension: tuple[pathlib.Path, pathlib.Path]) -> dict[str, list]:
    """The file's runs, written in place in the clone and restored afterwards."""
    path = env / file
    original = path.read_bytes()

    def analysed(source: bytes, *flags: str) -> tuple[list, list]:
        path.write_bytes(source)
        if tool == "phpstan":
            return phpstan(tools, env, file, context, work, extension)
        if tool == "psalm":
            return psalm(tools, env, file)
        return phpcs(tools, env, file, *flags)

    try:
        runs: dict[str, list] = {}
        runs["kept"], runs["keptElsewhere"] = analysed(original)
        if tool == "phpcs":
            runs["all"], runs["allElsewhere"] = analysed(original, "--ignore-annotations")
            runs["single"] = []
        else:
            runs["all"], runs["allElsewhere"] = analysed(neutralise(original, annotations, tool))
            runs["single"] = (
                [runs["all"]] if len(annotations) == 1
                else [analysed(neutralise(original, [annotation], tool))[0] for annotation in annotations]
            )
    finally:
        path.write_bytes(original)
    return runs


def version(command: list[str]) -> str:
    return subprocess.run(command, capture_output=True, text=True, env=clean_env()).stdout.strip()


def main() -> None:
    tools = pathlib.Path(sys.argv[1]).resolve()
    env_root = pathlib.Path(sys.argv[2]).resolve()
    output = pathlib.Path(sys.argv[3])
    rules, autoload, extension_versions = extensions(env_root)
    corpora = []
    for argument in sys.argv[4:]:
        name, _, spec = argument.partition("=")
        spec, _, root_version = spec.partition("@")
        root_text, _, paths_text = spec.partition(":")
        root = pathlib.Path(root_text).resolve()
        paths = paths_text.split(",")
        candidates = php_files(root, paths)
        with concurrent.futures.ThreadPoolExecutor(os.cpu_count()) as pool:
            parsed = list(pool.map(lambda file: lints(root / file), candidates))
        files = [file for file, ok in zip(candidates, parsed) if ok]
        excluded = [file for file, ok in zip(candidates, parsed) if not ok]
        annotations, traits = inventory(root, files)
        contexts = {
            file: trait_user(root, files, traits[file][0])
            for file in annotations["phpstan"] if file in traits
        }
        env = env_root / name
        dependencies = environment(root, env, root_version or None)
        jobs = {}
        with concurrent.futures.ThreadPoolExecutor(6) as pool:
            for tool, by_file in annotations.items():
                for index, (file, found) in enumerate(sorted(by_file.items())):
                    work = env_root / "work" / name / tool / str(index)
                    context = contexts.get(file) if tool == "phpstan" else None
                    jobs[(tool, file)] = pool.submit(record, tool, tools, env, file, found, context, work,
                                                     (rules, autoload))
        runs: dict[str, dict[str, object]] = {tool: {} for tool in annotations}
        for (tool, file), job in sorted(jobs.items()):
            runs[tool][file] = job.result()
            if tool == "phpstan" and file in contexts:
                runs[tool][file]["context"] = contexts[file]
        commit = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True,
                                text=True, check=True).stdout.strip()
        corpora.append({
            "name": name, "root": str(root), "commit": commit, "paths": paths, "excluded": excluded,
            "files": files,
            "dependencies": {"composerRootVersion": root_version or None, "packages": dependencies},
            "annotations": annotations, "runs": runs,
        })
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps({
        "phpstan": version([str(tools / "vendor/bin/phpstan"), "--version", "--no-ansi"]),
        "phpstanExtensions": extension_versions,
        "psalm": version([str(tools / "vendor/bin/psalm"), "--version"]),
        "phpcs": version([str(tools / "vendor/bin/phpcs"), "--version"]),
        "php": version(["php", "-r", "echo PHP_VERSION;"]),
        "corpora": corpora,
    }) + "\n")


if __name__ == "__main__":
    main()
