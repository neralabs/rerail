// Writes phpcs.expected.json: for each corpus case (a repository's PHP_CodeSniffer rulesets), the messages of the
// sniffs the php pack ports that PHP_CodeSniffer 4.0.4 reports in the same probe file at each probe path, run from the
// case's root with no --standard, so it finds the ruleset itself (`Config::CONFIG_FILENAMES`) as it does in a
// repository. `test/engine/catalog/lint-config/php-lint-config-oracle.test.ts` reads the rulesets with Rerail's reader
// and compares.
//
// Run by hand, never in CI, against PHP_CodeSniffer installed from scripts/pack-data/runtimes/phpcs-4.0.4 (PHP
// 8.5.11, whose deprecations Generic.PHP.DeprecatedFunctions reads):
//   cp scripts/pack-data/runtimes/phpcs-4.0.4/composer.json <install dir>/ && composer install --working-dir=<install dir>
//   node fixtures/host/lint-config/phpcs/generate_phpcs.mjs <install dir>

import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const VERSION = "4.0.4";
const OUTPUT = new URL("phpcs.expected.json", import.meta.url);
/** The sniffs the php pack ports: only their messages are recorded. */
const PORTED_SNIFFS = [
  "Generic.CodeAnalysis.EmptyStatement",
  "Generic.PHP.BacktickOperator",
  "Generic.PHP.DeprecatedFunctions",
  "Generic.PHP.ForbiddenFunctions",
  "Generic.PHP.LowerCaseConstant",
  "Generic.PHP.NoSilencedErrors",
  "Squiz.PHP.DisallowSizeFunctionsInLoops",
  "Squiz.PHP.Eval",
];
/** Code each ported sniff reports on (ForbiddenFunctions' `Found` needs a property: it reports `FoundWithAlternative`). */
const PROBE = [
  "<?php",
  "$a = [1];",
  "if ($a) {",
  "}",
  "foreach ($a as $v) {",
  "}",
  "$b = `ls`;",
  "$c = utf8_encode('x');",
  "$d = sizeof($a);",
  "$e = TRUE;",
  "$f = @file('x');",
  "for ($i = 0; $i < count($a); $i++) {",
  "    echo $i;",
  "}",
  "eval('$g = 1;');",
  "",
].join("\n");
/** Where the probe is written in every case: the root, folders patterns name, and folders they also match. */
const PROBES = [
  "probe.php",
  "build/probe.php",
  "lib/src/Legacy/probe.php",
  "src/Legacy/probe.php",
  "src/LegacyOld/probe.php",
  "src/probe.php",
  "src/tests/probe.php",
  "tests/probe.php",
];

const ruleset = (...rules) => `<?xml version="1.0"?>\n<ruleset name="Case">\n${rules.map(rule => `  ${rule}\n`).join("")}</ruleset>\n`;
const base = (...rules) => ({ "build/base.xml": ruleset(...rules) });

/** The corpus: name, files (the probes apart). */
const CASES = [
  ["standard-psr12", { "phpcs.xml.dist": ruleset('<rule ref="PSR12"/>') }],
  ["standard-squiz", { "phpcs.xml": ruleset('<rule ref="Squiz"/>') }],
  ["standard-generic", { "phpcs.xml": ruleset('<rule ref="Generic"/>') }],
  ["standard-pear", { "phpcs.xml": ruleset('<rule ref="PEAR"/>') }],
  ["standard-psr1", { "phpcs.xml": ruleset('<rule ref="PSR1"/>') }],
  ["standard-psr2", { "phpcs.xml": ruleset('<rule ref="PSR2"/>') }],
  ["standard-zend", { "phpcs.xml": ruleset('<rule ref="Zend"/>') }],
  ["first-config-wins", { ".phpcs.xml": ruleset('<rule ref="Squiz.PHP.Eval"/>'), "phpcs.xml": ruleset('<rule ref="Generic"/>') }],
  ["category-less-sniff", { "phpcs.xml": ruleset('<rule ref="Generic.PHP"><exclude name="Generic.PHP.NoSilencedErrors"/></rule>', '<rule ref="Generic.CodeAnalysis"/>') }],
  ["sniff-severity-0", { "phpcs.xml": ruleset('<rule ref="Generic.PHP.BacktickOperator"><severity>0</severity></rule>', '<rule ref="Generic.PHP.LowerCaseConstant"/>') }],
  ["message-code-only", { "phpcs.xml": ruleset('<rule ref="Generic.PHP.BacktickOperator.Found"/>', '<rule ref="Generic.CodeAnalysis.EmptyStatement.DetectedIf"/>') }],
  ["message-code-back-on", { "phpcs.xml": ruleset('<rule ref="Generic.PHP.BacktickOperator"><severity>0</severity></rule>', '<rule ref="Generic.PHP.BacktickOperator.Found"/>') }],
  ["standard-less-sniff", { "phpcs.xml": ruleset('<rule ref="Squiz"><exclude name="Squiz.PHP.Eval"/></rule>') }],
  ["message-code-excluded", { "phpcs.xml": ruleset('<rule ref="Generic.CodeAnalysis.EmptyStatement"><exclude name="Generic.CodeAnalysis.EmptyStatement.DetectedIf"/></rule>', '<rule ref="Generic.PHP.LowerCaseConstant"><exclude name="Generic.PHP.LowerCaseConstant.Found"/></rule>') }],
  ["message-code-severity", { "phpcs.xml": ruleset('<rule ref="Generic"/>', '<rule ref="Generic.PHP.NoSilencedErrors.Discouraged"><severity>4</severity></rule>', '<rule ref="Generic.PHP.LowerCaseConstant.Found"><severity>6</severity></rule>') }],
  ["standard-severity", { "phpcs.xml": ruleset('<rule ref="Generic"><severity>3</severity></rule>', '<rule ref="Squiz.PHP.Eval"/>') }],
  ["category-severity", { "phpcs.xml": ruleset('<rule ref="Generic.PHP"><severity>4</severity></rule>', '<rule ref="Generic.PHP.LowerCaseConstant"><severity>5</severity></rule>', '<rule ref="Squiz.PHP"/>') }],
  ["exclude-patterns", {
    "phpcs.xml": ruleset(
      '<rule ref="Squiz.PHP.Eval"><exclude-pattern>src/Legacy/*</exclude-pattern><exclude-pattern>*/tests/*</exclude-pattern><exclude-pattern>*.inc</exclude-pattern></rule>',
      '<rule ref="Generic.PHP.LowerCaseConstant"><exclude-pattern>src/Legacy</exclude-pattern></rule>',
      '<rule ref="Generic.PHP.BacktickOperator"><exclude-pattern>*/src/*</exclude-pattern></rule>',
    ),
  }],
  ["exclude-pattern-standard", { "phpcs.xml": ruleset('<rule ref="Generic"><exclude-pattern>build/*</exclude-pattern></rule>', '<rule ref="Squiz.PHP.Eval"><exclude-pattern>/build/*</exclude-pattern></rule>') }],
  ["exclude-pattern-message", { "phpcs.xml": ruleset('<rule ref="Generic.CodeAnalysis.EmptyStatement"/>', '<rule ref="Generic.CodeAnalysis.EmptyStatement.DetectedIf"><exclude-pattern>build/*</exclude-pattern></rule>') }],
  ["exclude-pattern-ruleset", { "phpcs.xml": ruleset('<exclude-pattern>build/*</exclude-pattern>', '<exclude-pattern>*/tests/*</exclude-pattern>', '<rule ref="Squiz.PHP.Eval"/>') }],
  ["exclude-pattern-ruleset-nested", { "phpcs.xml": ruleset('<rule ref="./build/base.xml"/>', '<rule ref="Squiz.PHP.Eval"/>'), ...base('<exclude-pattern>src/Legacy/*</exclude-pattern>') }],
  ["ignore-argument", { "phpcs.xml": ruleset('<arg name="ignore" value="build/*,*/tests/*"/>', '<rule ref="Squiz.PHP.Eval"/>') }],
  ["include-pattern", { "phpcs.xml": ruleset('<rule ref="Squiz.PHP.Eval"><include-pattern>src/*</include-pattern></rule>', '<rule ref="Generic.PHP.LowerCaseConstant"/>') }],
  // The reviewer's case: a ruleset referencing the ruleset PHP_CodeSniffer would otherwise use, by its file name.
  ["reference-dist-by-name", { ".phpcs.xml": ruleset('<rule ref="phpcs.xml.dist"/>', '<rule ref="Generic.PHP.LowerCaseConstant"/>'), "phpcs.xml.dist": ruleset('<rule ref="Squiz.PHP.Eval"/>') }],
  ["reference-relative", { "phpcs.xml": ruleset('<rule ref="./build/base.xml"/>'), ...base('<rule ref="Generic.PHP.NoSilencedErrors"/>') }],
  ["reference-bare-path", { "phpcs.xml": ruleset('<rule ref="build/base.xml"/>'), ...base('<rule ref="Generic.PHP.NoSilencedErrors"/>') }],
  ["reference-bare-name-file", { "phpcs.xml": ruleset('<rule ref="standard"/>'), "standard": ruleset('<rule ref="Squiz.PHP.Eval"/>') }],
  ["reference-dotted-name-file", { "phpcs.xml": ruleset('<rule ref="Acme.Custom.Rules"/>'), "Acme.Custom.Rules": ruleset('<rule ref="Squiz.PHP.Eval"/>') }],
  // From a nested ruleset, `./` is its own folder; a bare path is the working directory's (the root's).
  ["reference-nested-paths", {
    "phpcs.xml": ruleset('<rule ref="./build/base.xml"/>'),
    ...base('<rule ref="./near.xml"/>', '<rule ref="far.xml"/>', '<rule ref="./root-only.xml"/>'),
    "build/near.xml": ruleset('<rule ref="Squiz.PHP.Eval"/>'),
    "near.xml": ruleset('<rule ref="Generic.PHP.BacktickOperator"/>'),
    "far.xml": ruleset('<rule ref="Generic.PHP.LowerCaseConstant"/>'),
    "build/far.xml": ruleset('<rule ref="Generic.PHP.NoSilencedErrors"/>'),
    "root-only.xml": ruleset('<rule ref="Generic.PHP.DeprecatedFunctions"/>'),
  }],
  ["reference-folder", { "phpcs.xml": ruleset('<rule ref="./standard"/>', '<rule ref="build"/>'), "standard/ruleset.xml": ruleset('<rule ref="Squiz.PHP.Eval"/>'), "build/ruleset.xml": ruleset('<rule ref="Generic.PHP.NoSilencedErrors"/>') }],
  ["nested-exclude-reincluded", { "phpcs.xml": ruleset('<rule ref="./build/base.xml"/>', '<rule ref="Squiz.PHP.Eval"/>'), ...base('<rule ref="Squiz"><exclude name="Squiz.PHP.Eval"/></rule>') }],
  ["nested-severity-kept", { "phpcs.xml": ruleset('<rule ref="./build/base.xml"/>', '<rule ref="Generic.PHP.LowerCaseConstant"/>'), ...base('<rule ref="Generic.PHP.LowerCaseConstant"><severity>0</severity></rule>') }],
  ["argument-exclude", { "phpcs.xml": ruleset('<arg name="exclude" value="Squiz.PHP.Eval, generic.php.backtickoperator"/>', '<rule ref="Squiz"/>', '<rule ref="Generic.PHP"/>') }],
  ["argument-sniffs", { "phpcs.xml": ruleset('<arg name="sniffs" value="Generic.PHP.LowerCaseConstant"/>', '<rule ref="Squiz"/>', '<rule ref="Generic"/>') }],
  ["argument-nested-first-wins", {
    "phpcs.xml": ruleset('<arg name="exclude" value="Squiz.PHP.Eval"/>', '<rule ref="./build/base.xml"/>', '<rule ref="Squiz"/>', '<rule ref="Generic.PHP"/>'),
    ...base('<arg name="exclude" value="Generic.PHP.BacktickOperator"/>', '<arg name="sniffs" value="Squiz.PHP.Eval,Generic.PHP.LowerCaseConstant"/>'),
  }],
  ["argument-severity", { "phpcs.xml": ruleset('<arg name="severity" value="6"/>', '<rule ref="Generic"/>', '<rule ref="Squiz.PHP.Eval"><severity>7</severity></rule>') }],
  ["argument-no-warnings", { "phpcs.xml": ruleset('<arg value="n"/>', '<rule ref="Generic"/>', '<rule ref="Squiz"><exclude name="Squiz.PHP.Eval"/></rule>') }],
  ["tool-only-elements", { "phpcs.xml": ruleset('<rule ref="Squiz.PHP.Eval" phpcbf-only="true"/>', '<rule ref="Generic.PHP.LowerCaseConstant" phpcs-only="true"/>', '<rule ref="Generic.PHP"><exclude name="Generic.PHP.NoSilencedErrors" phpcbf-only="true"/></rule>') }],
  ["internal-reference", { "phpcs.xml": ruleset('<rule ref="Internal.NoCodeFound"><severity>0</severity></rule>', '<rule ref="Squiz.PHP.Eval"/>') }],
  ["message-type", { "phpcs.xml": ruleset('<rule ref="Generic.PHP.NoSilencedErrors"><type>error</type></rule>', '<rule ref="Squiz.PHP.Eval"><type>warning</type></rule>') }],
];

function phpcs(install, args, cwd) {
  return spawnSync(join(install, "vendor/bin/phpcs"), args, { cwd, encoding: "utf8" });
}

/** The ported sniffs' messages PHP_CodeSniffer reports at each probe path, or the error it stops with. */
function record(install, files) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-phpcs-oracle-")));
  try {
    for (const [path, text] of [...Object.entries(files), ...PROBES.map(path => [path, PROBE])]) {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), text);
    }
    const run = phpcs(install, ["--report=json", "-q", "--no-cache", "--no-colors", ...PROBES], root);
    if (!run.stdout.startsWith("{")) return { error: (run.stdout + run.stderr).trim().split("\n")[0] };
    const report = JSON.parse(run.stdout).files;
    return {
      reported: Object.fromEntries(PROBES.map(path => {
        const messages = report[join(root, path)]?.messages ?? [];
        const codes = messages.map(message => message.source).filter(code => PORTED_SNIFFS.some(sniff => code.startsWith(`${sniff}.`)));
        return [path, [...new Set(codes)].sort()];
      })),
    };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const install = process.argv[2];
if (install === undefined) throw new Error("usage: node generate_phpcs.mjs <PHP_CodeSniffer install dir>");
const version = execFileSync(join(install, "vendor/bin/phpcs"), ["--version"], { encoding: "utf8" });
if (!version.includes(`version ${VERSION} `)) throw new Error(`PHP_CodeSniffer ${VERSION} required, found: ${version.trim()}`);
const cases = CASES.map(([name, files]) => ({ name, files, ...record(install, files) }));
writeFileSync(OUTPUT, `${JSON.stringify({ tool: `PHP_CodeSniffer ${VERSION}`, probe: PROBE, cases }, null, 1)}\n`);
