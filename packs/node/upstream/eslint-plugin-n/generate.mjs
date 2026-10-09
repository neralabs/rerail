// Records eslint-plugin-n 18.4.1 for the Node pack (rerail/packs/node/conformance.json).
//
// Pins: eslint 10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser 8.70.1, typescript 5.9.3, globals 17.13.0;
// upstream eslint-plugin-n v18.4.1 = f1d47aff402b9e009578f4b3155b84f7d8beef9b; corpora fastify @
// 0d945819bebc08f4ca666d8283c03a26ed9706a0, prettier @ 5927216227411bc2cdaf30d50559e0a475ab4832. Refuses any other.
//
// Usage (offline, never in CI): node packs/node/upstream/eslint-plugin-n/generate.mjs
//   [--tools <dir>] (default: the nearest .rerail/tools/node) [--corpora <dir>] (default: the nearest .rerail/corpora/node)
//   [--fixtures-only] [--summary <file>] (writes the run summary, incl. per-case count mismatches, as JSON)
//
// 1. Copies the chosen rules' upstream tests verbatim to tests/lib/rules/ and runs them with stub-test-helpers.js
//    (ours), which captures every RuleTester case; each case's `code` becomes one file under fixtures/<rule>/.
// 2. Runs the pinned plugin (ESLint Node API, one config, default rule options, every chosen rule on) over every
//    fixture file and every pinned corpus file, and writes fixtures.expected.json and corpus-<name>.expected.json.
// 3. Writes ../../licenses/eslint-plugin-n/{LICENSE,UPSTREAM.md}.
// 4. Writes source.json, the plugin's source facts (conformance/recorders/eslint-source.mjs), from every upstream rule
//    test file (tests/lib/rules/**/*.js, loaded with the same stub; a file that does not load stops the recorder)
//    under conformance.json's approved configuration. The upstream checkout needs the release tag v18.4.1 fetched.
//    flat/recommended is the module or script variant by the package.json of the working directory the recorder
//    runs in; both turn on the same ported rules.
//
// Source type is what Node itself uses for the file: `.mjs`/`.mts` module, `.cjs`/`.cts` commonjs, otherwise the
// nearest package.json's `type` ("module" → module, anything else → commonjs). Globals: `globals.node` (globals
// 17.13.0), ecmaVersion latest; TypeScript extensions use @typescript-eslint/parser. The fixtures carry their own
// package.json (ours, see fixturePackageJson) so they do not inherit the enclosing repository's `"type": "module"`:
// it states upstream RuleTester's default (commonjs) and the engines of the package the upstream tests run in.
// An upstream case whose tester or case sets sourceType module is written as `.mjs` (`.mts` for TypeScript).
import { copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertCommit, corpusFiles, lint, pack, pinnedRequire, REPOSITORY, writeRecording } from "../../../../conformance/recorders/eslint-record.mjs";
import { assertUpstream, checkoutFiles, sourceFacts, writeSource } from "../../../../conformance/recorders/eslint-source.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = "eslint-plugin-n";
const VERSION = "18.4.1";
const PINS = { eslint: "10.11.0", "eslint-plugin-n": VERSION, "@typescript-eslint/parser": "8.70.1", typescript: "5.9.3", globals: "17.13.0" };
const UPSTREAM_REPOSITORY = "https://github.com/eslint-community/eslint-plugin-n";
const UPSTREAM_COMMIT = "f1d47aff402b9e009578f4b3155b84f7d8beef9b";
const UPSTREAM_TAG = "v18.4.1";
const CORPORA = { fastify: "0d945819bebc08f4ca666d8283c03a26ed9706a0", prettier: "5927216227411bc2cdaf30d50559e0a475ab4832" };

/** The chosen eslint-plugin-n rules (ids without the `n/` prefix); each test file is tests/lib/rules/<rule>.js. */
export const RULES = [
  "callback-return",
  "exports-style",
  "global-require",
  "handle-callback-err",
  "hashbang",
  "no-callback-literal",
  "no-deprecated-api",
  "no-exports-assign",
  "no-mixed-requires",
  "no-new-require",
  "no-path-concat",
  "no-process-env",
  "no-process-exit",
  "no-sync",
  "prefer-global/buffer",
  "prefer-global/console",
  "prefer-global/process",
  "prefer-global/text-decoder",
  "prefer-global/text-encoder",
  "prefer-global/url",
  "prefer-global/url-search-params",
  "prefer-node-protocol",
  "prefer-promises/dns",
  "prefer-promises/fs",
];
const RULE_IDS = RULES.map(rule => `n/${rule}`);

/** Ours: the fixtures' nearest package.json. Upstream's RuleTester defaults to commonjs and runs inside eslint-plugin-n. */
const fixturePackageJson = { private: true, description: "Rerail (not upstream): the eslint-plugin-n fixtures are CommonJS, as upstream's RuleTester default, and inherit the upstream package's engines.", type: "commonjs", engines: { node: "^20.19.0 || ^22.13.0 || >=24" } };

function nearest(start, name) {
  for (let directory = start; ; directory = dirname(directory)) {
    if (existsSync(join(directory, name))) return join(directory, name);
    if (dirname(directory) === directory) return undefined;
  }
}

function defaultDirectory(kind) {
  const found = nearest(REPOSITORY, join(".rerail", kind, "node"));
  if (found === undefined) throw new Error(`no .rerail/${kind}/node above ${REPOSITORY}`);
  return found;
}

/** The source type Node runs `file` (absolute) as. */
export function nodeSourceType(file) {
  const extension = extname(file);
  if (extension === ".mjs" || extension === ".mts") return "module";
  if (extension === ".cjs" || extension === ".cts") return "commonjs";
  const manifest = nearest(dirname(file), "package.json");
  if (manifest === undefined) return "commonjs";
  return JSON.parse(readFileSync(manifest, "utf8")).type === "module" ? "module" : "commonjs";
}

const TYPESCRIPT = [".ts", ".mts", ".cts", ".tsx"];

/**
 * n/no-sync 18.4.1 throws on any file the TypeScript parser parsed without type information (get-parser-services.js,
 * even under default options), so TypeScript files are linted with a program of their own: every TypeScript file of
 * the group, no module resolution, independent of any tsconfig.json around them.
 */
function typescriptProgram(typescript, cwd, files) {
  const roots = files.filter(file => TYPESCRIPT.includes(extname(file))).map(file => join(cwd, file));
  if (roots.length === 0) return undefined;
  return typescript.createProgram(roots, { noEmit: true, noResolve: true, skipLibCheck: true, allowJs: false, jsx: typescript.JsxEmit.Preserve, target: typescript.ScriptTarget.ESNext });
}

function config(sourceType, tools, program) {
  const languageOptions = { ecmaVersion: "latest", sourceType, globals: tools.globals.node };
  return [
    { files: ["**/*.js", "**/*.mjs", "**/*.cjs", "**/*.jsx"], languageOptions: { ...languageOptions, parserOptions: { ecmaFeatures: { jsx: true } } } },
    { files: ["**/*.ts", "**/*.mts", "**/*.cts", "**/*.tsx"], languageOptions: { ...languageOptions, parser: tools.typescriptParser, parserOptions: program === undefined ? {} : { programs: [program] } } },
    { plugins: { n: tools.plugin }, rules: Object.fromEntries(RULE_IDS.map(rule => [rule, "error"])) },
  ];
}

/** Lints `files` (relative to `cwd`), each under the source type Node gives it. */
async function record(tools, cwd, files) {
  const { ESLint } = tools.eslint;
  const findings = [];
  const skipped = [];
  for (const sourceType of ["commonjs", "module"]) {
    const group = files.filter(file => nodeSourceType(join(cwd, file)) === sourceType);
    if (group.length === 0) continue;
    const outcome = await lint(ESLint, cwd, config(sourceType, tools, typescriptProgram(tools.typescript, cwd, group)), group, RULE_IDS);
    findings.push(...outcome.findings);
    skipped.push(...outcome.skipped);
  }
  return { findings, skipped };
}

// ---- 1. upstream tests → fixtures -------------------------------------------------------------------------------

const TESTS = join(HERE, "tests");
const STUB = pathToFileURL(join(HERE, "stub-test-helpers.js")).href;
const EMPTY_MODULE = "data:text/javascript,export default {}";

function installHooks(toolsDirectory, upstream) {
  const toolsParent = pathToFileURL(join(toolsDirectory, "package.json")).href;
  const testsUrls = [TESTS, join(upstream, "tests")].map(path => pathToFileURL(path).href);
  const installedRules = join(toolsDirectory, "node_modules", "eslint-plugin-n", "lib", "rules");
  registerHooks({
    resolve(specifier, context, nextResolve) {
      const fromTests = testsUrls.some(url => context.parentURL?.startsWith(url) === true);
      if (!fromTests) return nextResolve(specifier, context);
      if (specifier === "#test-helpers" || specifier.endsWith("/test-helpers.js")) return { url: STUB, shortCircuit: true };
      // The rule under test is the installed plugin's (eslint-plugin-n publishes lib/ as it is), so a run's rule is
      // the plugin's own rule object. Sinon is never exercised: the stub only captures cases.
      if (specifier.includes("/lib/rules/")) return { url: pathToFileURL(join(installedRules, specifier.slice(specifier.indexOf("/lib/rules/") + "/lib/rules/".length))).href, shortCircuit: true };
      if (specifier === "sinon") return { url: EMPTY_MODULE, shortCircuit: true };
      if (!/^(\.|\/|node:|file:|data:)/u.test(specifier)) return nextResolve(specifier, { ...context, parentURL: toolsParent });
      return nextResolve(specifier, context);
    },
  });
  const noop = () => {};
  globalThis.describe = (_name, body) => body.call({ timeout: noop });
  globalThis.it = noop;
  // Mocha's hooks, and its skipped tests and suites (process-exit-as-throw's tests are xit()s).
  for (const hook of ["before", "after", "beforeEach", "afterEach", "xit", "xdescribe"]) globalThis[hook] = noop;
}

function copyTests(upstream) {
  rmSync(join(TESTS, "lib"), { recursive: true, force: true });
  for (const rule of RULES) {
    const target = join(TESTS, "lib", "rules", `${rule}.js`);
    mkdirSync(dirname(target), { recursive: true });
    copyFileSync(join(upstream, "tests", "lib", "rules", `${rule}.js`), target);
  }
}

function isTypeScript(run, item) {
  return run.typescript || item.languageOptions?.parser !== undefined;
}

/** The fixture path (relative to fixtures/) of one case, and the upstream fixture package.json files it needs. */
function casePath(upstream, rule, kind, index, run, item) {
  const base = `${rule}/${kind}-${String(index).padStart(3, "0")}`;
  const sourceType = item.languageOptions?.sourceType ?? run.languageOptions.sourceType;
  const typescript = isTypeScript(run, item);
  const filename = item.filename;
  // TsRuleTester's virtual file-N.ts names carry nothing; upstream fixture paths are kept below the case directory.
  if (typeof filename === "string" && !(run.typescript && /^file-\d+\.ts$/u.test(filename))) {
    // The copied tests compute fixture paths from their own location (tests/lib/rules/ here), i.e. below tests/fixtures/.
    const upstreamFixtures = join(upstream, "tests", "fixtures");
    // Upstream also names some relative to its repository root (the RuleTester's cwd): tests/fixtures/….
    const absolute = filename.startsWith(`tests${sep}fixtures${sep}`) ? join(upstream, filename) : filename;
    const fixtureRoot = [upstreamFixtures, join(TESTS, "fixtures")].find(root => absolute.startsWith(root + sep));
    const relativeName = fixtureRoot === undefined ? filename.replace(/^.*[\\/]/u, "") : relative(fixtureRoot, absolute);
    const manifests = [];
    if (fixtureRoot !== undefined) {
      for (let directory = dirname(join(upstreamFixtures, relativeName)); directory.startsWith(upstreamFixtures + sep); directory = dirname(directory)) {
        if (existsSync(join(directory, "package.json"))) manifests.push(relative(upstreamFixtures, join(directory, "package.json")));
      }
    }
    return { path: `${base}/${relativeName.split(sep).join("/")}`, manifests, sourceType };
  }
  const extension = typescript ? (sourceType === "module" ? ".mts" : ".ts") : sourceType === "module" ? ".mjs" : ".js";
  return { path: `${base}${extension}`, manifests: [], sourceType };
}

/** The runs one test file makes, loaded once: `{ name, rule, languageOptions, typescript, valid, invalid }`. */
const loadedRuns = new Map();
async function capturedRuns(path) {
  if (!loadedRuns.has(path)) {
    const { captured } = await import(STUB);
    const before = captured.length;
    await import(pathToFileURL(path).href);
    loadedRuns.set(path, captured.slice(before));
  }
  return loadedRuns.get(path);
}

async function extract(upstream) {
  copyTests(upstream);
  const cases = [];
  let skippedUpstream = 0;
  for (const rule of RULES) {
    const testFile = `tests/lib/rules/${rule}.js`;
    const runs = await capturedRuns(join(HERE, testFile));
    if (runs.length === 0) throw new Error(`${testFile}: no RuleTester run captured`);
    const counters = { valid: 0, invalid: 0 };
    runs.forEach((run, runIndex) => {
      for (const kind of ["valid", "invalid"]) {
        run[kind].forEach((raw, itemIndex) => {
          const item = typeof raw === "string" ? { code: raw } : raw;
          if (item.skip === true) {
            skippedUpstream += 1;
            return;
          }
          const index = ++counters[kind];
          const where = casePath(upstream, rule, kind, index, run, item);
          const errors = kind === "valid" ? 0 : typeof item.errors === "number" ? item.errors : item.errors.length;
          cases.push({
            rule,
            testFile,
            origin: `${runs.length > 1 ? `run ${runIndex + 1} ` : ""}${kind}[${itemIndex}]`,
            code: item.code,
            errors,
            configured: ["options", "settings", "languageOptions"].filter(key => item[key] !== undefined),
            ...where,
          });
        });
      }
    });
  }
  return { cases, skippedUpstream };
}

function writeFixtures(upstream, cases) {
  const fixtures = join(HERE, "fixtures");
  rmSync(fixtures, { recursive: true, force: true });
  mkdirSync(fixtures, { recursive: true });
  writeFileSync(join(fixtures, "package.json"), `${JSON.stringify(fixturePackageJson, null, 2)}\n`);
  const manifests = new Map();
  for (const entry of cases) {
    const target = join(fixtures, entry.path);
    if (existsSync(target)) throw new Error(`duplicate fixture ${entry.path}`);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, entry.code);
    const caseDirectory = entry.path.split("/").slice(0, 2).join("/");
    for (const manifest of entry.manifests) {
      const copy = `${caseDirectory}/${manifest.split(sep).join("/")}`;
      if (!existsSync(join(fixtures, copy))) {
        mkdirSync(dirname(join(fixtures, copy)), { recursive: true });
        cpSync(join(upstream, "tests", "fixtures", manifest), join(fixtures, copy));
      }
      manifests.set(copy, `tests/fixtures/${manifest.split(sep).join("/")}`);
    }
  }
  return manifests;
}

// ---- 3. licenses --------------------------------------------------------------------------------------------------

function writeLicenses(upstream, cases, manifests) {
  const directory = join(HERE, "..", "..", "licenses", TOOL);
  mkdirSync(directory, { recursive: true });
  copyFileSync(join(upstream, "LICENSE"), join(directory, "LICENSE"));
  const rows = [`|\`upstream/${TOOL}/source.json\`|the rules' implementations and tests, and the helpers they delegate to, at the revision above|extracted: the names, words and regexes \`source.json\` records|`];
  for (const rule of RULES) rows.push(`|\`upstream/${TOOL}/tests/lib/rules/${rule}.js\`|\`tests/lib/rules/${rule}.js\`|verbatim|`);
  rows.push(`|\`upstream/${TOOL}/fixtures/package.json\`|none|Rerail's own, not copied: the fixtures' source type and engines (\`fixturePackageJson\`), under the pack's \`LICENSE\`|`);
  for (const [copy, original] of [...manifests].sort()) rows.push(`|\`upstream/${TOOL}/fixtures/${copy}\`|\`${original}\`|verbatim (test data a case's \`filename\` reads)|`);
  for (const entry of cases) rows.push(`|\`upstream/${TOOL}/fixtures/${entry.path}\`|\`${entry.testFile}\`|extracted from ${entry.testFile} ${entry.origin}|`);
  const text = `# eslint-plugin-n ${VERSION}

Revision: ${UPSTREAM_COMMIT}
Source: ${UPSTREAM_REPOSITORY}

Upstream: [eslint-community/eslint-plugin-n](${UPSTREAM_REPOSITORY}/tree/${UPSTREAM_COMMIT}) at tag \`v${VERSION}\` (commit \`${UPSTREAM_COMMIT}\`). License: MIT (Copyright (c) 2015 Toru Nagashima); [\`LICENSE\`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below are the upstream rule tests of the rules the Node pack records (\`conformance.json\`, \`upstream/${TOOL}/generate.mjs\`), the upstream test data they read, and one file per test case holding that case's \`code\` verbatim; they remain under eslint-plugin-n's license. One listed file is Rerail's and says so in its row: \`upstream/${TOOL}/fixtures/package.json\` (the fixtures' source type and engines), listed because it sits among the copied fixtures. Every other file in this pack is Rerail's, under the Apache License 2.0 (the pack's \`LICENSE\`): the rule documents in \`rules/\` and \`candidates/\`, the recorder \`upstream/${TOOL}/generate.mjs\`, \`upstream/${TOOL}/stub-test-helpers.js\` (stands in for upstream's \`tests/test-helpers.js\` and only captures cases), and the \`*.expected.json\` recordings written by running eslint-plugin-n. \`upstream/${TOOL}/source.json\`, the facts the recorder records from the upstream source and tests at the revision above, holds names, words and regexes extracted from them: it is eslint-plugin-n's material and Rerail's, under \`MIT AND Apache-2.0\`, as \`REUSE.toml\` annotates it, and the table lists it.

Paths are relative to the pack directory, \`packs/node/\`.

|File|Upstream path|Copy|
|---|---|---|
${rows.join("\n")}
`;
  writeFileSync(join(directory, "UPSTREAM.md"), text);
}

// ---- 4. source facts ----------------------------------------------------------------------------------------------

/**
 * source.json: the plugin's source facts, from every RuleTester run of every upstream rule test file, with the cases
 * the fixtures leave out (`unscored`, `{ rule, test, case, reason }`).
 */
async function recordSource(toolsDirectory, upstream, unscored) {
  const pinned = pack("node").tools.find(entry => entry.name === TOOL);
  if (pinned?.recommended === undefined) throw new Error(`conformance.json declares no recommended configurations for ${TOOL}`);
  const plugin = { package: "eslint-plugin-n", namespace: "n" };
  const release = { path: "lib", installed: "eslint-plugin-n/lib" };
  assertUpstream({ checkout: upstream, commit: UPSTREAM_COMMIT, tag: UPSTREAM_TAG, version: VERSION, tools: toolsDirectory, plugin, release });
  const runs = [];
  const failed = [];
  for (const testFile of checkoutFiles(upstream, ["tests/lib/rules/**/*.js"])) {
    // The chosen rules' tests load from their verbatim copies under tests/, which are the same files.
    const copy = join(TESTS, relative("tests", testFile));
    try {
      for (const run of await capturedRuns(existsSync(copy) ? copy : join(upstream, testFile))) runs.push({ testFile, rule: run.rule, tests: { valid: run.valid, invalid: run.invalid } });
    } catch (error) {
      failed.push(`${testFile}: ${String(error?.message ?? error).split("\n")[0]}`);
    }
  }
  if (failed.length > 0) throw new Error(`${failed.length} upstream test file(s) do not load:\n${failed.join("\n")}`);
  const facts = sourceFacts({
    tool: TOOL,
    version: VERSION,
    repository: UPSTREAM_REPOSITORY,
    commit: UPSTREAM_COMMIT,
    tag: UPSTREAM_TAG,
    tools: toolsDirectory,
    checkout: upstream,
    plugin,
    configurations: pinned.recommended,
    rules: pack("node").rules.map(rule => rule.upstream.rule),
    implementation: "lib/rules/{rule}.js",
    helpers: ["@eslint-community/eslint-utils"],
    runs,
    unscored: unscored.filter(entry => pack("node").rules.some(rule => rule.upstream.rule === entry.rule)),
  });
  writeSource(join(HERE, "source.json"), facts);
  console.log(`source: ${Object.keys(facts.rules).length} rules, ${facts.testWords.length} test words`);
}

// ---- main ---------------------------------------------------------------------------------------------------------

async function loadTools(toolsDirectory) {
  const require = pinnedRequire(toolsDirectory, PINS);
  return {
    eslint: require("eslint"),
    plugin: (await import(pathToFileURL(require.resolve("eslint-plugin-n")).href)).default,
    globals: require("globals"),
    typescriptParser: require("@typescript-eslint/parser"),
    typescript: require("typescript"),
  };
}

async function main() {
  const { values } = parseArgs({ options: { tools: { type: "string" }, corpora: { type: "string" }, "fixtures-only": { type: "boolean" }, summary: { type: "string" } } });
  const report = { fixtures: undefined, corpora: [] };
  const writeSummary = () => values.summary !== undefined && writeFileSync(resolve(values.summary), `${JSON.stringify(report, null, 1)}\n`);
  const toolsDirectory = resolve(values.tools ?? defaultDirectory("tools"));
  const corporaDirectory = resolve(values.corpora ?? defaultDirectory("corpora"));
  const upstream = join(toolsDirectory, "upstream-eslint-plugin-n");
  assertCommit(upstream, UPSTREAM_COMMIT);
  const tools = await loadTools(toolsDirectory);
  installHooks(toolsDirectory, upstream);

  const { cases, skippedUpstream } = await extract(upstream);
  const manifests = writeFixtures(upstream, cases);
  const fixtures = join(HERE, "fixtures");
  const fixtureRun = await record(tools, fixtures, cases.map(entry => entry.path));
  const dropped = new Map(fixtureRun.skipped.map(entry => [entry.path, entry.reason]));
  const kept = cases.filter(entry => !dropped.has(entry.path));
  for (const path of dropped.keys()) rmSync(join(fixtures, path));
  writeRecording(join(HERE, "fixtures.expected.json"), TOOL, VERSION, { kind: "fixtures", origin: `${UPSTREAM_REPOSITORY}@${UPSTREAM_COMMIT}`, rules: RULE_IDS, files: kept.map(entry => entry.path).sort() }, fixtureRun.findings);
  writeLicenses(upstream, kept, manifests);
  const unscored = cases.filter(entry => dropped.has(entry.path)).map(entry => ({ rule: `n/${entry.rule}`, test: entry.testFile, case: entry.origin, reason: "unparsable" }));
  await recordSource(toolsDirectory, upstream, unscored);

  const mismatches = [];
  for (const entry of kept) {
    const own = fixtureRun.findings.filter(finding => finding.path === entry.path && finding.rule === `n/${entry.rule}`).length;
    if (own !== entry.errors) mismatches.push({ path: entry.path, origin: `${entry.testFile} ${entry.origin}`, expected: entry.errors, recorded: own, configured: entry.configured });
  }
  const summary = {
    extracted: cases.length + skippedUpstream,
    skippedUpstream,
    dropped: [...dropped].map(([path, reason]) => ({ path, reason })),
    kept: kept.length,
    findings: fixtureRun.findings.length,
    perRule: Object.fromEntries(RULES.map(rule => [rule, kept.filter(entry => entry.rule === rule).length])),
    mismatches,
  };
  report.fixtures = summary;
  console.log(JSON.stringify({ ...summary, mismatches: summary.mismatches.length }, null, 1));
  writeSummary();
  if (values["fixtures-only"]) return;

  for (const corpus of pack("node").corpora) {
    if (CORPORA[corpus.name] !== corpus.commit) throw new Error(`pack.json pins ${corpus.name} @ ${corpus.commit}; this recorder pins ${CORPORA[corpus.name]}`);
    const root = join(corporaDirectory, corpus.name);
    const files = corpusFiles(root, corpus);
    const run = await record(tools, root, files);
    writeRecording(join(HERE, `corpus-${corpus.name}.expected.json`), TOOL, VERSION, { kind: "corpus", name: corpus.name, repository: corpus.repository, commit: corpus.commit, rules: RULE_IDS, skipped: run.skipped.map(entry => entry.path).sort() }, run.findings);
    const perRule = Object.fromEntries(RULE_IDS.map(rule => [rule, run.findings.filter(finding => finding.rule === rule).length]));
    const corpusSummary = { corpus: corpus.name, files: files.length, skipped: run.skipped, findings: run.findings.length, perRule };
    report.corpora.push(corpusSummary);
    console.log(JSON.stringify(corpusSummary, null, 1));
    writeSummary();
  }
}

await main();
