import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "vitest";

import { extensionList, RUNTIMES_DIRECTORY, runtimeFile, unpinnedComposerDependencies, unpinnedNpmDependencies, unpinnedPythonDependencies } from "../../../scripts/pack-data/runtimes.js";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";

test("a runtime's manifest pins every direct dependency at an exact version", () => {
  assert.deepEqual(unpinnedNpmDependencies({ dependencies: { eslint: "10.11.0", "eslint-v9": "npm:eslint@9.39.5", "@scope/parser-v2": "npm:@typescript-eslint/parser@2.34.0", beta: "1.0.0-rc.1" } }), []);
  assert.deepEqual(unpinnedNpmDependencies({ dependencies: { a: "^1.2.3", b: "~1.2.3", c: "1.x", d: "latest", e: "npm:x@^1.0.0" } }), ["a@^1.2.3", "b@~1.2.3", "c@1.x", "d@latest", "e@npm:x@^1.0.0"]);
  assert.deepEqual(unpinnedNpmDependencies({ overrides: { acorn: "8.18.0", "@typescript-eslint/parser-v2": { eslint: "$eslint" }, "@typescript-eslint/typescript-estree": { minimatch: { ".": "10.2.5", "brace-expansion": "^5.0.5" } } } }), ["@typescript-eslint/typescript-estree > minimatch > brace-expansion@^5.0.5"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["bandit==1.9.4", "libcst==1.9.0"]\n'), []);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["ruff>=0.16", "libcst~=1.9", "x"]\n'), ["ruff>=0.16", "libcst~=1.9", "x"]);
  // Single quotes, and requirements after an extras entry's `]`, are read: the list is parsed as TOML.
  assert.deepEqual(unpinnedPythonDependencies("[project]\ndependencies = ['ruff>=0.16']\n"), ["ruff>=0.16"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = [\n  "bandit[toml]==1.9.4",\n  "ruff>=0.16",  # a comment ]\n]\n'), ["ruff>=0.16"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["bandit[toml,sarif]>=1.9"]\n'), ["bandit[toml,sarif]>=1.9"]);
  // uv sync installs the dev group by default, and tool.uv can add requirements or change where packages come from.
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["ruff==0.16.9"]\n[dependency-groups]\ndev = ["pytest>=8", { include-group = "lint" }]\nlint = ["ruff==0.16.9"]\n'), ["dependency-groups.dev: pytest>=8"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["ruff==0.16.9"]\n[tool.uv]\npackage = false\noverride-dependencies = ["libcst>=1"]\nsources = { ruff = { git = "https://example.com/ruff" } }\n'), ["tool.uv.override-dependencies: libcst>=1", "tool.uv.sources (not read by the pin check)"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = "ruff==0.16.9"\n'), ["(not a list)"]);
  assert.match(unpinnedPythonDependencies('[project]\ndependencies = ["ruff==0.16.9"\n')[0]!, /^\(not TOML: /u);
  // Composer: every requirement at an exact release, platform requirements included (`.php-version` pins the
  // interpreter), and no key that can change what is installed or run.
  assert.deepEqual(unpinnedComposerDependencies('{"name": "x", "require": {"squizlabs/php_codesniffer": "4.0.4", "nikic/php-parser": "v5.9.0", "a/b": "4.0.0-RC1"}}'), []);
  assert.deepEqual(unpinnedComposerDependencies('{"require": {"a/b": "^4.0", "c/d": "4.*", "e/f": ">=1.0 <2", "g/h": "dev-main", "php": ">=8.2", "ext-iconv": "*"}}'), ["a/b@^4.0", "c/d@4.*", "e/f@>=1.0 <2", "g/h@dev-main", "php@>=8.2", "ext-iconv@*"]);
  assert.deepEqual(unpinnedComposerDependencies('{"require": {"a/b": "1.0.0"}, "require-dev": {"c/d": "^1"}, "repositories": [], "config": {"platform": {"php": "8.1.0"}}}'), ["config (not read by the pin check)", "repositories (not read by the pin check)", "require-dev (not read by the pin check)"]);
  assert.deepEqual(unpinnedComposerDependencies('{"require": {"a/b": 1}}'), ["a/b@1"]);
  assert.deepEqual(unpinnedComposerDependencies('{"require": []}'), ["require (not an object)"]);
  assert.deepEqual(unpinnedComposerDependencies("[]"), ["(not a JSON object)"]);
  assert.match(unpinnedComposerDependencies("{")[0]!, /^\(not JSON: /u);
});

test("every committed runtime pins its direct dependencies exactly, and has no lockfile", () => {
  const runtimes = join(PACKAGE_ROOT, RUNTIMES_DIRECTORY);
  for (const runtime of readdirSync(runtimes, { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name)) {
    const directory = join(runtimes, runtime);
    for (const lockfile of ["package-lock.json", "uv.lock", "composer.lock"]) assert.ok(!existsSync(join(directory, lockfile)), `${runtime}/${lockfile}`);
    if (existsSync(join(directory, "package.json"))) assert.deepEqual(unpinnedNpmDependencies(JSON.parse(readFileSync(join(directory, "package.json"), "utf8")) as { dependencies?: Record<string, string> }), [], runtime);
    if (existsSync(join(directory, "pyproject.toml"))) assert.deepEqual(unpinnedPythonDependencies(readFileSync(join(directory, "pyproject.toml"), "utf8")), [], runtime);
    if (existsSync(join(directory, "composer.json"))) {
      assert.deepEqual(unpinnedComposerDependencies(readFileSync(join(directory, "composer.json"), "utf8")), [], runtime);
      // Composer installs no interpreter, so a PHP runtime names the exact one it must run on.
      assert.match(readFileSync(join(directory, ".php-version"), "utf8"), /^\d+\.\d+\.\d+\n$/u, runtime);
    }
  }
});

test("PHP's extensions are read from `php -m` as extensions.txt lists them: headers left out, sorted, distinct", () => {
  assert.deepEqual(extensionList("[PHP Modules]\nmbstring\nCore\nZend OPcache\n\n[Zend Modules]\nZend OPcache\n\n"), ["Core", "Zend OPcache", "mbstring"]);
  // The committed list is already in that form, so a byte-identical `php -n -m` is what pinnedPhp accepts.
  const committed = readFileSync(runtimeFile("php", "extensions.txt"), "utf8");
  assert.equal(committed, `${extensionList(committed).join("\n")}\n`);
});
