// Writes psalm.expected.json: for each corpus case (a repository's Psalm config), the issues of the php pack's Psalm
// ports that Psalm 6.19.1 reports as errors (not info, which fails nothing) in the probe at each probe path, run from
// the case's root with no --config, so it finds the config itself (`Config::DEFAULT_FILE_NAMES`) as it does in a
// repository. `test/engine/catalog/lint-config/php-lint-config-oracle.test.ts` reads the configs with Rerail's reader
// and compares.
//
// Run by hand, never in CI, against Psalm installed from scripts/pack-data/runtimes/psalm-6.19.1 (PHP 8.5.11):
//   cp scripts/pack-data/runtimes/psalm-6.19.1/composer.json <install dir>/ && composer install --working-dir=<install dir>
//   node fixtures/host/lint-config/psalm/generate_psalm.mjs <install dir>

import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const VERSION = "6.19.1";
const OUTPUT = new URL("psalm.expected.json", import.meta.url);
const PORTED_ISSUES = ["ForbiddenCode"];
const PROBE = "<?php\nvar_dump(1);\n";
const PROBES = ["probe.php", "lib/src/Legacy/probe.php", "src/Legacy/Kept/probe.php", "src/Legacy/probe.php", "src/probe.php", "tests/probe.php"];

/** A config with `attributes` on its root and `body` in it, analysing every probe. */
const psalm = (attributes, body = "") =>
  `<?xml version="1.0"?>\n<psalm xmlns="https://getpsalm.org/schema/config" ${attributes}>\n  <projectFiles><directory name="."/></projectFiles>\n${body}</psalm>\n`;
const handler = (...lines) => `  <issueHandlers>\n${lines.map(line => `    ${line}\n`).join("")}  </issueHandlers>\n`;

/** The corpus: name, files (the probes apart). */
const CASES = [
  ["level-default", { "psalm.xml": psalm("") }],
  ["level-1", { "psalm.xml": psalm('errorLevel="1"') }],
  ["level-4", { "psalm.xml": psalm('errorLevel="4"') }],
  ["level-5", { "psalm.xml": psalm('errorLevel="5"') }],
  ["level-8", { "psalm.xml": psalm('errorLevel="8"') }],
  ["file-xml-dist", { "psalm.xml.dist": psalm('errorLevel="5"') }],
  ["file-dist-xml", { "psalm.dist.xml": psalm('errorLevel="5"') }],
  ["file-first-wins", { "psalm.xml.dist": psalm('errorLevel="1"'), "psalm.dist.xml": psalm('errorLevel="8"') }],
  ["file-xml-over-dists", { "psalm.xml": psalm('errorLevel="8"'), "psalm.dist.xml": psalm('errorLevel="1"') }],
  ["handler-suppress", { "psalm.xml": psalm('errorLevel="1"', handler('<ForbiddenCode errorLevel="suppress"/>')) }],
  ["handler-info", { "psalm.xml": psalm('errorLevel="1"', handler('<ForbiddenCode errorLevel="info"/>')) }],
  ["handler-error", { "psalm.xml": psalm('errorLevel="8"', handler('<ForbiddenCode errorLevel="error"/>')) }],
  ["handler-plugin-issue", { "psalm.xml": psalm('errorLevel="1"', handler('<PluginIssue name="ForbiddenCode" errorLevel="suppress"/>')) }],
  ["handler-folders", {
    "psalm.xml": psalm('errorLevel="1"', handler(
      "<ForbiddenCode>",
      '  <errorLevel type="suppress"><directory name="src/Legacy"/><file name="src/probe.php"/></errorLevel>',
      '  <errorLevel type="error"><directory name="src/Legacy/Kept"/><directory name="tests/**"/></errorLevel>',
      "</ForbiddenCode>",
    )),
  }],
  ["handler-folders-info", {
    "psalm.xml": psalm('errorLevel="8"', handler(
      '<ForbiddenCode errorLevel="info">',
      '  <errorLevel type="error"><directory name="src"/></errorLevel>',
      '  <errorLevel type="suppress"><directory name="*/Legacy"/></errorLevel>',
      "</ForbiddenCode>",
    )),
  }],
];

/** The ported issues Psalm reports as errors at each probe path, or the error it stops with. */
function record(install, files) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-psalm-oracle-")));
  try {
    for (const [path, text] of [...Object.entries(files), ...PROBES.map(path => [path, PROBE])]) {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), text);
    }
    const run = spawnSync(join(install, "vendor/bin/psalm"), ["--no-cache", "--no-progress", "--output-format=json", "--show-info=false", "--threads=1", ...PROBES], { cwd: root, encoding: "utf8" });
    if (!run.stdout.startsWith("[")) return { error: (run.stdout + run.stderr).trim().split("\n")[0] };
    const issues = JSON.parse(run.stdout).filter(issue => issue.severity === "error" && PORTED_ISSUES.includes(issue.type));
    return {
      reported: Object.fromEntries(PROBES.map(path => [path, [...new Set(issues.filter(issue => issue.file_path === join(root, path)).map(issue => issue.type))].sort()])),
    };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const install = process.argv[2];
if (install === undefined) throw new Error("usage: node generate_psalm.mjs <Psalm install dir>");
const version = execFileSync(join(install, "vendor/bin/psalm"), ["--version"], { encoding: "utf8" });
if (!version.includes(`${VERSION}@`)) throw new Error(`Psalm ${VERSION} required, found: ${version.trim()}`);
const cases = CASES.map(([name, files]) => ({ name, files, ...record(install, files) }));
writeFileSync(OUTPUT, `${JSON.stringify({ tool: `Psalm ${VERSION}`, probe: PROBE, cases }, null, 1)}\n`);
