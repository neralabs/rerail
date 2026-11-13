// Writes phpstan.expected.json: for each corpus case (a repository's PHPStan config files), the identifiers of the
// rules the php pack ports that PHPStan 2.2.16 reports in the probe at each probe path, run from the case's root
// with no --configuration and no --level, so it finds the config itself as it does in a repository. An identifier
// counts only on the probe line written for its rule, so another rule reporting it elsewhere does not.
// `test/engine/catalog/lint-config/php-lint-config-oracle.test.ts` reads the configs with Rerail's reader and compares.
//
// Run by hand, never in CI, against PHPStan installed from scripts/pack-data/runtimes/phpstan-2.2.16 (PHP 8.5.11):
//   cp scripts/pack-data/runtimes/phpstan-2.2.16/composer.json <install dir>/ && composer install --working-dir=<install dir>
//   node fixtures/host/lint-config/phpstan/generate_phpstan.mjs <install dir>

import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const VERSION = "2.2.16";
const OUTPUT = new URL("phpstan.expected.json", import.meta.url);
/** The probe after its namespace line, and the line each ported identifier is reported on. */
const PROBE = [
  "function probe_return() { return 1; }",
  "function probe_sprintf(): string { return sprintf('%s %s', 'a'); }",
  "function probe_sscanf(): mixed { return sscanf('a b', '%s %s', $x); }",
  "/** @return array<string, int> */ function probe_keys(): array { return ['a' => 1, 'a' => 2]; }",
  "function probe_if(): void { $x = 0; if ($x) { echo 1; } }",
  "function probe_random(): int { return random_int(10, 1); }",
  "",
];
const LINES = { 3: "missingType.return", 4: "argument.sprintf", 5: "argument.sscanf", 6: "array.duplicateKey", 7: "if.alwaysFalse", 8: "argument.type" };
const PROBES = ["probe.php", "lib/src/Legacy/probe.php", "src/Legacy/probe.php", "src/probe.php", "tests/probe.php"];
/** A probe of its own namespace, so the probes' functions do not clash. */
const probe = path => ["<?php", `namespace Probe\\${dirname(path).replaceAll(/[^A-Za-z]/gu, "_") || "Root"};`, ...PROBE].join("\n");

const neon = (...lines) => `${lines.join("\n")}\n`;
const level = value => neon("parameters:", `\tlevel: ${value}`);
/** The corpus: name, files (the probes apart). */
const CASES = [
  ["level-0", { "phpstan.neon": level(0) }],
  ["level-3", { "phpstan.neon": level(3) }],
  ["level-4", { "phpstan.neon": level(4) }],
  ["level-5-string", { "phpstan.neon": level("'5'") }],
  ["level-6", { "phpstan.neon": level(6) }],
  ["level-max", { "phpstan.neon": level("max") }],
  ["file-neon-dist", { "phpstan.neon.dist": level(6) }],
  ["file-dot-neon", { ".phpstan.neon": level(6) }],
  ["file-dist-neon", { "phpstan.dist.neon": level(6) }],
  ["file-dot-dist-neon", { ".phpstan.dist.neon": level(6) }],
  ["file-first-wins", { "phpstan.neon": level(6), "phpstan.neon.dist": level(0) }],
  // The level configs are in the phar alone (its alias phpstan.phar); the Composer package's conf/ has bleedingEdge.neon.
  ["level-config", { "phpstan.neon": neon("includes:", "\t- phar://phpstan.phar/conf/config.level4.neon", "\t- phar://phpstan.phar/conf/bleedingEdge.neon") }],
  ["level-config-and-level", { "phpstan.neon": neon("includes:", "\t- phar://phpstan.phar/conf/config.level4.neon", "parameters:", "\tlevel: 5") }],
  ["level-config-max", { "phpstan.neon": neon("includes:", "\t- phar://phpstan.phar/conf/config.levelmax.neon", "\t- '%rootDir%/conf/bleedingEdge.neon'") }],
  ["level-config-root-dir", { "phpstan.neon": neon("includes:", "\t- '%rootDir%/conf/config.level4.neon'") }],
  ["no-level", { "phpstan.neon": neon("parameters:", "\tpaths: [src]") }],
  ["custom-ruleset", { "phpstan.neon": neon("parameters:", "\tcustomRulesetUsed: true") }],
  ["custom-ruleset-rules", { "phpstan.neon": neon("parameters:", "\tcustomRulesetUsed: true", "rules:", "\t- PHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule") }],
  ["rules-own-class", { "phpstan.neon": neon("parameters:", "\tlevel: 0", "rules:", "\t- PHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule") }],
  ["rules-other-class", { "phpstan.neon": neon("parameters:", "\tlevel: 4", "rules:", "\t- PHPStan\\Rules\\Methods\\MissingMethodReturnTypehintRule") }],
  ["services-tagged", {
    "phpstan.neon": neon("parameters:", "\tlevel: 0", "services:", "\t-", "\t\tclass: \\PHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule", "\t\ttags: [phpstan.rules.rule]"),
  }],
  ["services-untagged", { "phpstan.neon": neon("parameters:", "\tlevel: 5", "services:", "\t- PHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule") }],
  ["ignore-identifier", { "phpstan.neon": neon("parameters:", "\tlevel: 6", "\tignoreErrors:", "\t\t- identifier: missingType.return", "\t\t- identifiers: [argument.sprintf, if.alwaysFalse]") }],
  ["ignore-folder", {
    "phpstan.neon": neon(
      "parameters:",
      "\tlevel: 6",
      "\treportUnmatchedIgnoredErrors: false",
      "\tignoreErrors:",
      "\t\t-",
      "\t\t\tidentifier: argument.sprintf",
      "\t\t\tpaths: [src/Legacy, src/probe.php]",
      "\t\t-",
      "\t\t\tidentifier: argument.type",
      "\t\t\tpath: '%currentWorkingDirectory%/src'",
      "\t\t-",
      "\t\t\tidentifier: array.duplicateKey",
      "\t\t\tpath: '*/Legacy/*'",
    ),
  }],
  ["ignore-partial", {
    "phpstan.neon": neon(
      "parameters:",
      "\tlevel: 6",
      "\treportUnmatchedIgnoredErrors: false",
      "\tignoreErrors:",
      "\t\t-",
      "\t\t\tidentifier: if.alwaysFalse",
      "\t\t\tmessage: '#^If condition#'",
      "\t\t-",
      "\t\t\tidentifier: missingType.return",
      "\t\t\tcount: 1",
      "\t\t\tpath: probe.php",
      "\t\t- '#^Call to sprintf#'",
    ),
  }],
  ["include-repository-file", {
    "phpstan.neon": neon("includes:", "\t- build/base.neon", "parameters:", "\tlevel: 6"),
    "build/base.neon": neon("parameters:", "\tlevel: 2", "\treportUnmatchedIgnoredErrors: false", "\tignoreErrors:", "\t\t- identifier: argument.type", "\t\t  path: ../src"),
  }],
  ["include-level-only", { "phpstan.neon": neon("includes:", "\t- build/base.neon"), "build/base.neon": level(5) }],
  ["ignore-errors-replaced", {
    "phpstan.neon": neon("includes:", "\t- base.neon", "parameters:", "\tlevel: 6", "\tignoreErrors!:", "\t\t- identifier: argument.type"),
    "base.neon": neon("parameters:", "\tignoreErrors:", "\t\t- identifier: missingType.return"),
  }],
];

/** The ported identifiers PHPStan reports at each probe path, or the error it stops with. */
function record(install, files) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-phpstan-oracle-")));
  try {
    for (const [path, text] of [...Object.entries(files), ...PROBES.map(path => [path, probe(path)])]) {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), text);
    }
    const run = spawnSync(join(install, "vendor/bin/phpstan"), ["analyse", "--no-progress", "--error-format=json", "--memory-limit=1G", ...PROBES], { cwd: root, encoding: "utf8" });
    if (!run.stdout.startsWith("{")) return { error: run.stderr.trim().split("\n").find(line => line.trim() !== "" && !line.startsWith("Note:"))?.trim() };
    const report = JSON.parse(run.stdout).files;
    return {
      reported: Object.fromEntries(PROBES.map(path => {
        const messages = report[join(root, path)]?.messages ?? [];
        return [path, [...new Set(messages.filter(message => LINES[message.line] === message.identifier).map(message => message.identifier))].sort()];
      })),
    };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const install = process.argv[2];
if (install === undefined) throw new Error("usage: node generate_phpstan.mjs <PHPStan install dir>");
const version = execFileSync(join(install, "vendor/bin/phpstan"), ["--version"], { encoding: "utf8" });
if (!version.includes(` ${VERSION}`)) throw new Error(`PHPStan ${VERSION} required, found: ${version.trim()}`);
const cases = CASES.map(([name, files]) => ({ name, files, ...record(install, files) }));
writeFileSync(OUTPUT, `${JSON.stringify({ tool: `PHPStan ${VERSION}`, probe: PROBE.join("\n"), cases }, null, 1)}\n`);
