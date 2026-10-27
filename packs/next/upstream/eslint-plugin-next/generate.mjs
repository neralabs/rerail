// Records @next/eslint-plugin-next for the Next pack's T4 tier (packs/next/conformance.json).
//
// Pins: eslint 10.11.0, @next/eslint-plugin-next 16.3.8, @typescript-eslint/parser 8.70.1, typescript 5.9.3,
// globals 17.13.0 (installed under `.rerail/tools/next`); upstream vercel/next.js v16.3.8 =
// b0fad0d45eb4c4430fda5eeeb442e8a5af08a5f6 (`.rerail/tools/next/upstream-next`, sparse packages/eslint-plugin-next +
// test/unit/eslint-plugin-next). Any other version or commit is refused.
//
// Run by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request
// CI). Usage (Node >= 23.6 for type stripping and module.registerHooks):
//   node packs/next/upstream/eslint-plugin-next/generate.mjs [--tools <dir>] [--upstream <dir>]
//                                                            [--only fixtures|tests|licenses]
//
// 1. tests: copies the chosen rules' upstream test files verbatim into tests/.
// 2. fixtures: executes each copied test with a stub `RuleTester` (ours, below) that captures run(name, rule,
//    { valid, invalid }), and writes one file per case: fixtures/<rule>/<valid|invalid>-NNN.js, or, for a case with a
//    `filename`, fixtures/<rule>/<valid|invalid>-NNN/<filename> (a leading `/` or `./` dropped, a Windows drive
//    `C:\` turned into `C/`, backslashes into `/`). Writes Rerail's own cases (AUTHORED, below) as
//    fixtures/<rule>/rerail-<name>.tsx and lists them in UPSTREAM.md. Then runs the pinned plugin (PACK_CONFIG, default
//    options, every chosen rule) over every fixture file: fixtures.expected.json.
// 3. licenses: packs/next/licenses/next.js/{LICENSE,UPSTREAM.md}.
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertCommit, assertPortableCase, isVariantPath, lint, pinnedRequire, withOrigins, writeRecording, writeRenameVariants } from "../../../../conformance/recorders/eslint-record.mjs";
import { packFixturesWritten, requirePackFixture } from "../../../../scripts/s3/pack-fixtures.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
/** The pack directory, packs/next/. */
const PACK = dirname(dirname(HERE));
const REPOSITORY = dirname(dirname(PACK));

/** `.rerail/<kind>/next` in the nearest ancestor of this repository holding it (gitignored there). */
function rerailDirectory(kind) {
  for (let directory = REPOSITORY; ; directory = dirname(directory)) {
    const candidate = join(directory, ".rerail", kind, "next");
    if (existsSync(candidate)) return candidate;
    if (dirname(directory) === directory) throw new Error(`no .rerail/${kind}/next above ${REPOSITORY}; pass --${kind} <dir>`);
  }
}

const TOOL = "eslint-plugin-next";
const PLUGIN_VERSION = "16.3.8";
const PINS = {
  eslint: "10.11.0",
  "@next/eslint-plugin-next": PLUGIN_VERSION,
  "@typescript-eslint/parser": "8.70.1",
  typescript: "5.9.3",
  globals: "17.13.0",
};
const UPSTREAM_REPOSITORY = "https://github.com/vercel/next.js";
const UPSTREAM_COMMIT = "b0fad0d45eb4c4430fda5eeeb442e8a5af08a5f6";
const UPSTREAM_TESTS = "test/unit/eslint-plugin-next";

/** The chosen rules (plugin rule names); every one is recorded on every fixture file. */
export const RULES = [
  "google-font-display",
  "google-font-preconnect",
  "inline-script-id",
  "next-script-for-ga",
  "no-assign-module-variable",
  "no-async-client-component",
  "no-before-interactive-script-outside-document",
  "no-css-tags",
  "no-document-import-in-page",
  "no-duplicate-head",
  "no-head-element",
  "no-head-import-in-document",
  "no-img-element",
  "no-location-assign-relative-destination",
  "no-page-custom-font",
  "no-script-component-in-head",
  "no-styled-jsx-in-document",
  "no-sync-scripts",
  "no-title-in-document-head",
  "no-typos",
  "no-unwanted-polyfillio",
];
const RULE_IDS = RULES.map(rule => `@next/next/${rule}`);

/**
 * Ours: cases Rerail wrote for the plugin to score beside the upstream tests, each an input the engine once misread.
 * `errors` is how many findings the case's own rule must report, checked against the recording as a test's count is.
 */
const AUTHORED = [
  // A bare `&` in a JSX attribute string is text; the vendored TSX grammar once read it as a parse error.
  {
    rule: "no-img-element",
    name: "bare-ampersand",
    code: 'export default function Page() {\n  return <img src="/a?w=8&h=8" alt="" />;\n}\n',
    errors: 1,
  },
  {
    rule: "no-sync-scripts",
    name: "bare-ampersand",
    code: 'export default function Page() {\n  return <script src="https://example.com/widget.js?a=1&b=2" />;\n}\n',
    errors: 1,
  },
];

const { values: options } = parseArgs({
  options: {
    tools: { type: "string" },
    upstream: { type: "string" },
    only: { type: "string" },
  },
});
const tools = options.tools ?? rerailDirectory("tools");
const upstream = options.upstream ?? join(tools, "upstream-next");
const require = pinnedRequire(tools, PINS);
const { ESLint } = require("eslint");
const plugin = require("@next/eslint-plugin-next");
const tsParser = require("@typescript-eslint/parser");
const globals = require("globals");

/**
 * The one pack config: Next's own split (eslint-config-next parses JS with a JSX-enabled parser and TS with
 * typescript-eslint), latest syntax, browser + Node globals, every chosen rule on at its default options.
 */
function packConfig() {
  const rules = Object.fromEntries(RULE_IDS.map(id => [id, "error"]));
  const common = { plugins: { "@next/next": plugin }, rules, linterOptions: { reportUnusedDisableDirectives: "off" } };
  const languageGlobals = { ...globals.browser, ...globals.node };
  return [
    {
      ...common,
      files: ["**/*.js", "**/*.jsx", "**/*.mjs"],
      languageOptions: { ecmaVersion: "latest", sourceType: "module", parserOptions: { ecmaFeatures: { jsx: true } }, globals: languageGlobals },
    },
    {
      ...common,
      files: ["**/*.cjs"],
      languageOptions: { ecmaVersion: "latest", sourceType: "commonjs", parserOptions: { ecmaFeatures: { jsx: true } }, globals: languageGlobals },
    },
    {
      ...common,
      files: ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"],
      languageOptions: { parser: tsParser, ecmaVersion: "latest", sourceType: "module", parserOptions: { ecmaFeatures: { jsx: true } }, globals: languageGlobals },
    },
  ];
}

const testsDirectory = join(HERE, "tests");
const fixturesDirectory = join(HERE, "fixtures");

function copyTests() {
  assertCommit(upstream, UPSTREAM_COMMIT);
  rmSync(testsDirectory, { recursive: true, force: true });
  mkdirSync(testsDirectory, { recursive: true });
  for (const rule of RULES) copyFileSync(join(upstream, UPSTREAM_TESTS, `${rule}.test.ts`), join(testsDirectory, `${rule}.test.ts`));
}

// Ours: the stub `eslint` module the copied tests import. Its RuleTester only captures what run() is given.
const STUB_ESLINT = `export class RuleTester {
  constructor(config) { this.config = config; }
  run(name, rule, tests) { globalThis.__nextCaptured.push({ name, rule, config: this.config, valid: tests.valid ?? [], invalid: tests.invalid ?? [] }); }
}`;
// Ours: the stub plugin module; `rules[name]` is the name itself.
const STUB_PLUGIN = `export const rules = new Proxy({}, { get: (_, name) => String(name) }); export default { rules };`;

/** Executes each copied test file with the stubs; returns { rule: { valid: [case], invalid: [case] } }. */
async function captureCases() {
  registerHooks({
    resolve(specifier, context, next) {
      if (specifier === "eslint") return { url: `data:text/javascript,${encodeURIComponent(STUB_ESLINT)}`, shortCircuit: true };
      if (specifier === "@next/eslint-plugin-next") return { url: `data:text/javascript,${encodeURIComponent(STUB_PLUGIN)}`, shortCircuit: true };
      return next(specifier, context);
    },
  });
  globalThis.describe = (_name, body) => body();
  globalThis.it = () => {
    throw new Error("a chosen test file uses it(); extend the stub");
  };
  const cases = {};
  for (const rule of RULES) {
    globalThis.__nextCaptured = [];
    await import(pathToFileURL(join(testsDirectory, `${rule}.test.ts`)).href);
    const runs = globalThis.__nextCaptured;
    if (runs.length !== 1 || runs[0].rule !== rule) throw new Error(`${rule}.test.ts: expected one run of ${rule}, got ${JSON.stringify(runs.map(run => run.rule))}`);
    cases[rule] = { valid: runs[0].valid, invalid: runs[0].invalid };
  }
  return cases;
}

/** A test case's filename as a relative path under its case directory. */
function caseFilename(filename) {
  let path = filename.replaceAll("\\", "/").replace(/^([A-Za-z]):\//u, "$1/").replace(/^\.\//u, "").replace(/^\/+/u, "");
  if (path.split("/").some(part => part === ".." || part === "")) throw new Error(`unsupported case filename ${filename}`);
  return path;
}

/** Writes the fixture files; returns [{ rule, kind, index, path, errors }] (path relative to fixtures/). */
function writeFixtures(cases) {
  rmSync(fixturesDirectory, { recursive: true, force: true });
  const written = [];
  for (const rule of RULES) {
    for (const kind of ["valid", "invalid"]) {
      cases[rule][kind].forEach((entry, index) => {
        const test = typeof entry === "string" ? { code: entry } : entry;
        const name = `${kind}-${String(index).padStart(3, "0")}`;
        const path = test.filename === undefined ? `${rule}/${name}.js` : `${rule}/${name}/${caseFilename(test.filename)}`;
        if (test.options !== undefined || test.settings !== undefined || test.languageOptions !== undefined)
          throw new Error(`${rule} ${name}: case options/settings/languageOptions are not supported by this recorder`);
        assertPortableCase(path, test.code);
        mkdirSync(dirname(join(fixturesDirectory, path)), { recursive: true });
        writeFileSync(join(fixturesDirectory, path), test.code);
        written.push({ rule, kind, index, path, errors: kind === "valid" ? 0 : Array.isArray(test.errors) ? test.errors.length : test.errors });
      });
    }
  }
  AUTHORED.forEach(({ rule, name, code, errors }, index) => {
    const path = `${rule}/rerail-${name}.tsx`;
    mkdirSync(dirname(join(fixturesDirectory, path)), { recursive: true });
    writeFileSync(join(fixturesDirectory, path), code);
    written.push({ rule, kind: "authored", index, path, errors });
  });
  return written;
}

/** UPSTREAM.md beside this recorder: the fixtures Rerail wrote, which are Rerail's, not Next.js's. */
function writeAuthoredRecord(kept) {
  const rows = kept.filter(entry => entry.kind === "authored").map(entry => `| \`upstream/${TOOL}/fixtures/${entry.path}\` | written by Rerail |`);
  const text = `# ${TOOL} ${PLUGIN_VERSION}: cases Rerail wrote

The fixtures listed below were written by Rerail for the pinned plugin to score, beside the cases extracted from the plugin's own tests (\`licenses/next.js/UPSTREAM.md\`). Each is an input the engine once misread; \`generate.mjs\` holds them (\`AUTHORED\`) and records the plugin's findings on them in \`fixtures.expected.json\`. They are Rerail's (Apache-2.0, the pack's licence), so nothing goes under \`licenses/\`. Paths are relative to the pack directory.

| File | Origin |
|---|---|
${rows.join("\n")}
`;
  writeFileSync(join(HERE, "UPSTREAM.md"), text);
}

async function recordFixtures() {
  const written = writeFixtures(await captureCases());
  // Each upstream case is a case of the rule whose test file it came from; Rerail's own cases have no upstream origin.
  const origins = new Map(written.filter(entry => entry.kind !== "authored").map(entry => [entry.path, [`@next/next/${entry.rule}`]]));
  const { variants, skipped: unrenamed } = writeRenameVariants(fixturesDirectory, [...origins.keys()], origins);
  const linted = await lint(ESLint, fixturesDirectory, packConfig(), [...written.map(entry => entry.path), ...variants.map(variant => variant.unit)], RULE_IDS);
  const dropped = new Set(linted.skipped.map(entry => entry.path));
  for (const entry of linted.skipped.filter(({ path }) => !isVariantPath(path))) {
    console.warn(`dropped (ESLint cannot parse under the pack config): ${entry.path}: ${entry.reason}`);
    rmSync(join(fixturesDirectory, entry.path));
  }
  const kept = written.filter(entry => !dropped.has(entry.path));
  const recorded = withOrigins(fixturesDirectory, kept.map(entry => entry.path).sort(), origins, variants, dropped, linted.findings);
  const findings = recorded.findings;
  writeAuthoredRecord(kept);
  writeRecording(join(HERE, "fixtures.expected.json"), TOOL, PLUGIN_VERSION, { kind: "fixtures", origin: `${UPSTREAM_REPOSITORY}@${UPSTREAM_COMMIT}`, rules: RULE_IDS, files: recorded.files, units: recorded.units }, findings);
  // Sanity check: the case's own rule under the pack config vs the test's expected error count.
  const mismatches = [];
  for (const entry of kept) {
    const own = findings.filter(finding => finding.path === entry.path && finding.rule === `@next/next/${entry.rule}`).length;
    if (own !== entry.errors) mismatches.push({ path: entry.path, expected: entry.errors, recorded: own });
  }
  const byPath = new Map(written.map(entry => [entry.path, entry]));
  const renamed = Object.entries(recorded.units).flatMap(([path, { renamedFrom }]) => {
    if (renamedFrom === undefined) return [];
    const { rule, kind, index } = byPath.get(renamedFrom);
    return [{ rule, kind: "renamed", index, path, of: renamedFrom, renamedKind: kind }];
  });
  const cases = [...written.map(({ rule, kind, index, path, errors }) => ({ rule, kind, index, path, errors, dropped: dropped.has(path) })), ...renamed];
  writeFileSync(join(HERE, "fixtures.cases.json"), `${JSON.stringify({ cases, unrenamed, mismatches }, null, 1)}\n`);
  console.log(packFixturesWritten([join(HERE, "fixtures.cases.json")]));
  console.log(`fixtures: ${written.length} cases extracted, ${dropped.size} dropped, ${renamed.length} rename variants, ${findings.length} findings, ${mismatches.length} mismatches vs the tests`);
  for (const mismatch of mismatches) console.log(`  mismatch ${mismatch.path}: test expects ${mismatch.expected}, plugin reports ${mismatch.recorded}`);
}

function writeLicenses() {
  assertCommit(upstream, UPSTREAM_COMMIT);
  const directory = join(PACK, "licenses", "next.js");
  mkdirSync(directory, { recursive: true });
  copyFileSync(join(upstream, "license.md"), join(directory, "LICENSE"));
  const { cases } = JSON.parse(readFileSync(requirePackFixture(join(HERE, "fixtures.cases.json")), "utf8"));
  const rows = [
    ...RULES.map(rule => `| \`upstream/${TOOL}/tests/${rule}.test.ts\` | \`${UPSTREAM_TESTS}/${rule}.test.ts\` | verbatim |`),
    ...cases
      .filter(entry => !entry.dropped && entry.kind !== "authored" && entry.kind !== "renamed")
      .map(entry => `| \`upstream/${TOOL}/fixtures/${entry.path}\` | \`${UPSTREAM_TESTS}/${entry.rule}.test.ts\` | extracted from ${entry.kind}[${entry.index}] (its \`code\`, verbatim) |`),
    ...cases
      .filter(entry => entry.kind === "renamed")
      .map(entry => `| \`upstream/${TOOL}/fixtures/${entry.path}\` | \`${UPSTREAM_TESTS}/${entry.rule}.test.ts\` | derived from ${entry.renamedKind}[${entry.index}] (\`fixtures/${entry.of}\` with its names and literals renamed) |`),
  ];
  const text = `# Next.js ${PLUGIN_VERSION} (@next/eslint-plugin-next)

Upstream: [vercel/next.js](${UPSTREAM_REPOSITORY}/tree/${UPSTREAM_COMMIT}) at tag \`v${PLUGIN_VERSION}\` (commit \`${UPSTREAM_COMMIT}\`), the source of \`@next/eslint-plugin-next\` ${PLUGIN_VERSION}. License: MIT (Copyright (c) 2025 Vercel, Inc.); [\`LICENSE\`](LICENSE) is the upstream \`license.md\`, copied verbatim at that tag.

The files listed below are the plugin's rule tests and the test cases extracted from them, and remain under Next.js's license. Every other file in this pack is Rerail's. The rule documents in \`rules/\` and \`candidates/rules/\` are Rerail's re-implementations of @next/eslint-plugin-next's rules, published under the pack's licence expression, \`Apache-2.0 AND MIT\`, as \`REUSE.toml\` annotates them. The rest is under the Apache License 2.0 (the pack's \`LICENSE\`): the recorder \`upstream/${TOOL}/generate.mjs\` (with its stub \`RuleTester\` and stub plugin module), \`upstream/${TOOL}/fixtures.cases.json\`, and the \`*.expected.json\` outputs recorded by running the pinned plugin.

Paths are relative to the pack directory, \`packs/next/\`.

| File | Upstream path | Copy |
|---|---|---|
${rows.join("\n")}
`;
  writeFileSync(join(directory, "UPSTREAM.md"), text);
  console.log(`licenses: ${rows.length} rows`);
}

const only = options.only;
if (only === undefined || only === "tests") copyTests();
if (only === undefined || only === "fixtures") await recordFixtures();
if (only === undefined || only === "licenses") writeLicenses();
