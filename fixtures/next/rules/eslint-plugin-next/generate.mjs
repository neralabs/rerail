// Records @next/eslint-plugin-next for the Next pack's T4 tier (rerail/conformance/languages/next/pack.json).
//
// Pins: eslint 10.11.0, @next/eslint-plugin-next 16.3.8, @typescript-eslint/parser 8.70.1, typescript 5.9.3,
// globals 17.13.0 (installed under `.nera/tools/next`); upstream vercel/next.js v16.3.8 =
// b0fad0d45eb4c4430fda5eeeb442e8a5af08a5f6 (`.nera/tools/next/upstream-next`, sparse packages/eslint-plugin-next +
// test/unit/eslint-plugin-next); corpora shadcn-ui @ 0e3abd65a97707f4a9cc3ed07bf5006e1cb67b13, next-examples @
// 6194d642b6fd743437da21dac3c720a587e6b9b2, commerce @ 3761e52e60df9c6a316e067dbfd7032e494d3634
// (`.nera/corpora/next/<name>`). Any other version or commit is refused.
//
// Usage (offline, maintainer only; Node >= 23.6 for type stripping and module.registerHooks):
//   node rerail-fixtures/next/rules/eslint-plugin-next/generate.mjs [--tools <dir>] [--corpora <dir>] [--upstream <dir>]
//                                                                  [--only fixtures|corpora|tests|licenses]
//
// 1. tests: copies the chosen rules' upstream test files verbatim into tests/.
// 2. fixtures: executes each copied test with a stub `RuleTester` (ours, below) that captures run(name, rule,
//    { valid, invalid }), and writes one file per case: fixtures/<rule>/<valid|invalid>-NNN.js, or, for a case with a
//    `filename`, fixtures/<rule>/<valid|invalid>-NNN/<filename> (a leading `/` or `./` dropped, a Windows drive
//    `C:\` turned into `C/`, backslashes into `/`). Then runs the pinned plugin (PACK_CONFIG, default options, every
//    chosen rule) over every fixture file: fixtures.expected.json.
// 3. corpora: the same config over each pinned corpus's files (pack.json paths and excludes): corpus-<name>.expected.json.
// 4. licenses: rerail-fixtures/next/licenses/next.js/{LICENSE,UPSTREAM.md}.
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertCommit, corpusFiles, lint, pack, pinnedRequire, writeRecording } from "../../../frameworks/eslint_record.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const NEXT_ORACLES = dirname(dirname(HERE));
const CHECKOUT = "/Users/tim/orca/workspaces/nera-landing/rerail-php-support";

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
const CORPUS_COMMITS = {
  "shadcn-ui": "0e3abd65a97707f4a9cc3ed07bf5006e1cb67b13",
  "next-examples": "6194d642b6fd743437da21dac3c720a587e6b9b2",
  commerce: "3761e52e60df9c6a316e067dbfd7032e494d3634",
};

/** The chosen rules (plugin rule names); every one is recorded on every fixture and corpus file. */
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

const { values: options } = parseArgs({
  options: {
    tools: { type: "string", default: join(CHECKOUT, ".nera", "tools", "next") },
    corpora: { type: "string", default: join(CHECKOUT, ".nera", "corpora", "next") },
    upstream: { type: "string" },
    only: { type: "string" },
  },
});
const upstream = options.upstream ?? join(options.tools, "upstream-next");
const require = pinnedRequire(options.tools, PINS);
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
        mkdirSync(dirname(join(fixturesDirectory, path)), { recursive: true });
        writeFileSync(join(fixturesDirectory, path), test.code);
        written.push({ rule, kind, index, path, errors: kind === "valid" ? 0 : Array.isArray(test.errors) ? test.errors.length : test.errors });
      });
    }
  }
  return written;
}

async function recordFixtures() {
  const written = writeFixtures(await captureCases());
  const { findings, skipped } = await lint(ESLint, fixturesDirectory, packConfig(), written.map(entry => entry.path), RULE_IDS);
  const dropped = new Set(skipped.map(entry => entry.path));
  for (const entry of skipped) {
    console.warn(`dropped (ESLint cannot parse under the pack config): ${entry.path}: ${entry.reason}`);
    rmSync(join(fixturesDirectory, entry.path));
  }
  const kept = written.filter(entry => !dropped.has(entry.path));
  writeRecording(join(HERE, "fixtures.expected.json"), TOOL, PLUGIN_VERSION, { kind: "fixtures", origin: `${UPSTREAM_REPOSITORY}@${UPSTREAM_COMMIT}`, rules: RULE_IDS, files: kept.map(entry => entry.path).sort() }, findings);
  // Sanity check: the case's own rule under the pack config vs the test's expected error count.
  const mismatches = [];
  for (const entry of kept) {
    const own = findings.filter(finding => finding.path === entry.path && finding.rule === `@next/next/${entry.rule}`).length;
    if (own !== entry.errors) mismatches.push({ path: entry.path, expected: entry.errors, recorded: own });
  }
  writeFileSync(join(HERE, "fixtures.cases.json"), `${JSON.stringify({ cases: written.map(({ rule, kind, index, path, errors }) => ({ rule, kind, index, path, errors, dropped: dropped.has(path) })), mismatches }, null, 1)}\n`);
  console.log(`fixtures: ${written.length} cases extracted, ${dropped.size} dropped, ${findings.length} findings, ${mismatches.length} mismatches vs the tests`);
  for (const mismatch of mismatches) console.log(`  mismatch ${mismatch.path}: test expects ${mismatch.expected}, plugin reports ${mismatch.recorded}`);
}

async function recordCorpora() {
  const manifest = pack("next");
  for (const corpus of manifest.corpora) {
    if (CORPUS_COMMITS[corpus.name] !== corpus.commit) throw new Error(`pack.json pins ${corpus.name} at ${corpus.commit}; this recorder pins ${CORPUS_COMMITS[corpus.name]}`);
    const root = join(options.corpora, corpus.name);
    const files = corpusFiles(root, corpus);
    const { findings, skipped } = await lint(ESLint, root, packConfig(), files, RULE_IDS);
    writeRecording(
      join(HERE, `corpus-${corpus.name}.expected.json`),
      TOOL,
      PLUGIN_VERSION,
      { kind: "corpus", name: corpus.name, repository: corpus.repository, commit: corpus.commit, rules: RULE_IDS, skipped: skipped.map(entry => entry.path).sort() },
      findings,
    );
    console.log(`corpus ${corpus.name}: ${files.length} files, ${skipped.length} skipped, ${findings.length} findings`);
    for (const entry of skipped) console.log(`  skipped ${entry.path}: ${entry.reason}`);
  }
}

function writeLicenses() {
  assertCommit(upstream, UPSTREAM_COMMIT);
  const directory = join(NEXT_ORACLES, "licenses", "next.js");
  mkdirSync(directory, { recursive: true });
  copyFileSync(join(upstream, "license.md"), join(directory, "LICENSE"));
  const { cases } = JSON.parse(readFileSync(join(HERE, "fixtures.cases.json"), "utf8"));
  const rows = [
    ...RULES.map(rule => `| \`rules/${TOOL}/tests/${rule}.test.ts\` | \`${UPSTREAM_TESTS}/${rule}.test.ts\` | verbatim |`),
    ...cases
      .filter(entry => !entry.dropped)
      .map(entry => `| \`rules/${TOOL}/fixtures/${entry.path}\` | \`${UPSTREAM_TESTS}/${entry.rule}.test.ts\` | extracted from ${entry.kind}[${entry.index}] (its \`code\`, verbatim) |`),
  ];
  const text = `# Next.js ${PLUGIN_VERSION} (@next/eslint-plugin-next)

Upstream: [vercel/next.js](${UPSTREAM_REPOSITORY}/tree/${UPSTREAM_COMMIT}) at tag \`v${PLUGIN_VERSION}\` (commit \`${UPSTREAM_COMMIT}\`), the source of \`@next/eslint-plugin-next\` ${PLUGIN_VERSION}. License: MIT (Copyright (c) 2025 Vercel, Inc.); [\`LICENSE\`](LICENSE) is the upstream \`license.md\`, copied verbatim at that tag.

The files listed below are the plugin's rule tests and the test cases extracted from them, and remain under Next.js's license. Every other file under \`rules/${TOOL}/\` is Rerail's, under Rerail's MIT license (\`rerail/LICENSE\`): \`generate.mjs\` (with its stub \`RuleTester\` and stub plugin module), \`fixtures.cases.json\`, and the recordings it writes by running the pinned plugin.

Paths are relative to \`rerail-fixtures/next/\`.

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
if (only === undefined || only === "corpora") await recordCorpora();
