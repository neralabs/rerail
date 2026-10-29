// Records eslint-plugin-n 18.4.1 for the Node pack (rerail/packs/node/conformance.json).
//
// Pins: eslint 10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser 8.70.1, typescript 5.9.3, globals 17.13.0;
// upstream eslint-plugin-n v18.4.1 = f1d47aff402b9e009578f4b3155b84f7d8beef9b. Refuses any other.
//
// Run by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request
// CI). Usage: node packs/node/upstream/eslint-plugin-n/generate.mjs
//   [--tools <dir>] (default: the nearest .rerail/tools/node)
//   [--summary <file>] (writes the run summary, incl. per-case count mismatches, as JSON)
//
// 1. Copies the chosen rules' upstream tests verbatim to tests/lib/rules/ and runs them with stub-test-helpers.js
//    (ours), which captures every RuleTester case; each case's `code` becomes one file under fixtures/<rule>/.
// 2. Runs the pinned plugin (ESLint Node API, one config, default rule options, every chosen rule on) over every
//    fixture file, and writes fixtures.expected.json.
// 3. Copies ../../licenses/eslint-plugin-n/LICENSE and writes provenance.json and LICENSES.md: every copied test, every
//    case with its upstream test file, and the test data and manifests beside them.
//
// Source type is what Node itself uses for the file: `.mjs`/`.mts` module, `.cjs`/`.cts` commonjs, otherwise the
// nearest package.json's `type` ("module" → module, anything else → commonjs). Globals: `globals.node` (globals
// 17.13.0), where an ES module has none of CommonJS's wrapper names (require, module, exports, __dirname,
// __filename), as the plugin's own flat/mixed-esm-and-cjs config gives each file flat/recommended-script or
// flat/recommended-module (lib/configs/recommended-module.js turns those names off); ecmaVersion latest; TypeScript
// extensions use @typescript-eslint/parser. The fixtures carry their own package.json (ours, see
// fixturePackageJson) so they do not inherit the enclosing repository's `"type": "module"`: it states upstream
// RuleTester's default (commonjs) and the engines of the package the upstream tests run in. An upstream case whose
// tester or case sets sourceType module is written as `.mjs` (`.mts` for TypeScript), unless its code has no module
// syntax and reads a CommonJS wrapper name: upstream's RuleTester declares `globals.node`, those names included, in
// every case, so such a case is written as CommonJS (`.js`/`.ts`), the only module system where Node defines them.
// Each fixture's analysis unit is the fixture and its nearest package.json (the one the plugin and Node read for
// it), so Rerail reads the same manifest: its `type`, `bin` and `engines`. `source.units` names each case's origin,
// the rule whose test file it came from; each case also gets a rename variant under fixtures/renamed/
// (scripts/pack-data/eslint-expected.mjs `writeRenameVariants`) with a copy of its manifest, linted like any case
// and marked `renamedFrom` it.
import { copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertCommit, isVariantPath, lint, pinnedRequire, REPOSITORY, withOrigins, writeExpectedFindings, writeRenameVariants } from "../../../../scripts/pack-data/eslint-expected.mjs";
import { RERAIL, variantEntries, writeProvenance } from "../../../../scripts/pack-data/provenance.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = "eslint-plugin-n";
const VERSION = "18.4.1";
const PINS = { eslint: "10.11.0", "eslint-plugin-n": VERSION, "@typescript-eslint/parser": "8.70.1", typescript: "5.9.3", globals: "17.13.0" };
const UPSTREAM_REPOSITORY = "https://github.com/eslint-community/eslint-plugin-n";
const UPSTREAM_COMMIT = "f1d47aff402b9e009578f4b3155b84f7d8beef9b";

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

/** The bindings Node's CommonJS module wrapper gives a file; an ES module has none of them. */
const COMMONJS_WRAPPER_NAMES = ["require", "module", "exports", "__dirname", "__filename"];

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

/**
 * A fixture's analysis unit: the fixture (relative to `fixtures`), then its nearest package.json. The fixture comes
 * first, as a unit's first path is its key in `source.units`.
 */
function fixtureUnit(fixtures, path) {
  const manifest = relative(fixtures, nearest(dirname(join(fixtures, path)), "package.json")).split(sep).join("/");
  return [path, manifest];
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

/** The globals of a file Node runs as `sourceType`: an ES module's lack CommonJS's wrapper names (flat/recommended-module). */
function nodeGlobals(sourceType, tools) {
  if (sourceType !== "module") return tools.globals.node;
  return { ...tools.globals.node, ...Object.fromEntries(COMMONJS_WRAPPER_NAMES.map(name => [name, "off"])) };
}

function config(sourceType, tools, program) {
  const languageOptions = { ecmaVersion: "latest", sourceType, globals: nodeGlobals(sourceType, tools) };
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

function installHooks(toolsDirectory) {
  const toolsParent = pathToFileURL(join(toolsDirectory, "package.json")).href;
  const testsUrl = pathToFileURL(TESTS).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      const fromTests = context.parentURL?.startsWith(testsUrl) === true;
      if (!fromTests) return nextResolve(specifier, context);
      if (specifier === "#test-helpers" || specifier.endsWith("/test-helpers.js")) return { url: STUB, shortCircuit: true };
      // The rule under test and sinon are never exercised: the stub only captures cases.
      if (specifier.includes("/lib/rules/") || specifier === "sinon") return { url: EMPTY_MODULE, shortCircuit: true };
      if (!/^(\.|\/|node:|file:|data:)/u.test(specifier)) return nextResolve(specifier, { ...context, parentURL: toolsParent });
      return nextResolve(specifier, context);
    },
  });
  const noop = () => {};
  globalThis.describe = (_name, body) => body.call({ timeout: noop });
  globalThis.it = noop;
  for (const hook of ["before", "after", "beforeEach", "afterEach"]) globalThis[hook] = noop;
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

/**
 * Whether a case upstream runs as a module reads a CommonJS wrapper name, which upstream's RuleTester declares in
 * every case (`globals.node`), and has no module syntax (it parses as CommonJS): Node runs such code only as CommonJS.
 */
function readsCommonjsWrapper(tools, code, typescript) {
  const { Linter } = tools.eslint;
  const linter = new Linter({ configType: "flat" });
  const file = typescript ? "case.ts" : "case.js";
  const languageOptions = { ecmaVersion: "latest", ...(typescript ? { parser: tools.typescriptParser } : {}) };
  const parses = linter.verify(code, [{ files: ["**"], languageOptions: { ...languageOptions, sourceType: "commonjs" } }], file);
  if (parses.some(message => message.fatal)) return false;
  let reads = false;
  const probe = { rules: { wrapper: { create: context => ({ "Program:exit"() {
    const globalScope = context.sourceCode.scopeManager.globalScope;
    reads = COMMONJS_WRAPPER_NAMES.some(name => (globalScope.set.get(name)?.references.length ?? 0) > 0);
  } }) } } };
  linter.verify(code, [{ files: ["**"], languageOptions: { ...languageOptions, sourceType: "module", globals: tools.globals.node }, plugins: { probe }, rules: { "probe/wrapper": "error" } }], file);
  return reads;
}

/** The fixture path (relative to fixtures/) of one case, and the upstream fixture package.json files it needs. */
function casePath(tools, upstream, rule, kind, index, run, item) {
  const base = `${rule}/${kind}-${String(index).padStart(3, "0")}`;
  const typescript = isTypeScript(run, item);
  const tester = item.languageOptions?.sourceType ?? run.languageOptions.sourceType;
  const sourceType = tester === "module" && readsCommonjsWrapper(tools, item.code, typescript) ? "commonjs" : tester;
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

async function extract(upstream, tools) {
  copyTests(upstream);
  const { captured } = await import(STUB);
  const cases = [];
  let skippedUpstream = 0;
  for (const rule of RULES) {
    const testFile = `tests/lib/rules/${rule}.js`;
    const before = captured.length;
    await import(pathToFileURL(join(HERE, testFile)).href);
    const runs = captured.slice(before);
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
          const where = casePath(tools, upstream, rule, kind, index, run, item);
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

// ---- 3. licence and provenance ------------------------------------------------------------------------------------

/** A copy beside the rename variants: the manifest of the original the variant renames, unchanged. */
const manifestCopy = original => `a copy of \`${original}\`, beside the rename variants it gives a manifest`;

/**
 * ../../licenses/eslint-plugin-n/LICENSE (verbatim) and the tool's provenance.json and LICENSES.md: the copied tests,
 * Rerail's fixtures/package.json, the upstream test data copied beside the cases (`manifests`: copy → upstream path),
 * each kept case, and its rename variant (`recorded`, withOrigins' result) with copies of its manifest.
 */
function writeLicenseAndProvenance(upstream, cases, manifests, recorded) {
  const directory = join(HERE, "..", "..", "licenses", TOOL);
  mkdirSync(directory, { recursive: true });
  copyFileSync(join(upstream, "LICENSE"), join(directory, "LICENSE"));
  const verbatimData = "verbatim (test data a case's `filename` reads)";
  const originals = new Map([
    ["fixtures/package.json", { source: RERAIL, how: "authored", detail: "Rerail's own, not copied: the fixtures' source type and engines (`fixturePackageJson`), under the pack's `LICENSE`" }],
    ...[...manifests].map(([copy, from]) => [`fixtures/${copy}`, { source: TOOL, from, how: "verbatim", detail: verbatimData }]),
    ...cases.map(entry => [`fixtures/${entry.path}`, { source: TOOL, from: entry.testFile, how: "extracted", detail: `extracted from ${entry.testFile} ${entry.origin}` }]),
  ]);
  const byPath = new Map(cases.map(entry => [`fixtures/${entry.path}`, entry]));
  // A variant's manifest is a copy of its original's (Rerail's, or upstream test data verbatim), not renamed.
  const variant = (_path, entry) => {
    const original = byPath.get(entry.renamedFrom);
    if (original !== undefined) return { ...entry, detail: `derived from ${original.testFile} ${original.origin} (\`${entry.renamedFrom}\` with its names and literals renamed)` };
    const { source, from, how } = originals.get(entry.renamedFrom);
    return { source, from, how, detail: manifestCopy(entry.renamedFrom) };
  };
  const entries = new Map([
    ...RULES.map(rule => [`tests/lib/rules/${rule}.js`, { source: TOOL, from: `tests/lib/rules/${rule}.js`, how: "verbatim" }]),
    ...originals,
    ...variantEntries(originals, recorded, variant),
  ]);
  writeProvenance({ pack: "node", tool: TOOL, entries, commits: { [TOOL]: UPSTREAM_COMMIT } });
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
  const { values } = parseArgs({ options: { tools: { type: "string" }, summary: { type: "string" } } });
  const toolsDirectory = resolve(values.tools ?? defaultDirectory("tools"));
  const upstream = join(toolsDirectory, "upstream-eslint-plugin-n");
  assertCommit(upstream, UPSTREAM_COMMIT);
  const tools = await loadTools(toolsDirectory);
  installHooks(toolsDirectory);

  const { cases, skippedUpstream } = await extract(upstream, tools);
  const manifests = writeFixtures(upstream, cases);
  const fixtures = join(HERE, "fixtures");
  // Each case is a case of the rule whose test file it came from. Its rename variant, under renamed/, renames the
  // case's code and copies its manifest.
  const units = new Map(cases.map(entry => [entry.path, fixtureUnit(fixtures, entry.path)]));
  const origins = new Map(cases.map(entry => [entry.path, [`n/${entry.rule}`]]));
  const { variants } = writeRenameVariants(fixtures, [...units.values()], origins);
  const fixtureRun = await record(tools, fixtures, [...cases.map(entry => entry.path), ...variants.map(variant => variant.paths.get(variant.of))]);
  const dropped = new Map(fixtureRun.skipped.map(entry => [entry.path, entry.reason]));
  const kept = cases.filter(entry => !dropped.has(entry.path));
  for (const path of [...dropped.keys()].filter(path => !isVariantPath(path))) rmSync(join(fixtures, path));
  const recorded = withOrigins(fixtures, kept.map(entry => entry.path).sort().map(path => units.get(path)), origins, variants, new Set(dropped.keys()), fixtureRun.findings);
  writeExpectedFindings(join(HERE, "fixtures.expected.json"), TOOL, VERSION, { kind: "fixtures", origin: `${UPSTREAM_REPOSITORY}@${UPSTREAM_COMMIT}`, rules: RULE_IDS, files: recorded.files, units: recorded.units }, recorded.findings);
  writeLicenseAndProvenance(upstream, kept, manifests, recorded);

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
  console.log(JSON.stringify({ ...summary, mismatches: summary.mismatches.length }, null, 1));
  if (values.summary !== undefined) writeFileSync(resolve(values.summary), `${JSON.stringify({ fixtures: summary }, null, 1)}\n`);
}

await main();
