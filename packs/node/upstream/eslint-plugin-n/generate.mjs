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
//    fixture file, and writes fixtures.expected.json. The run lints a copy of fixtures/ under a fresh temporary
//    directory, so the plugin reads nothing above it while it resolves imports and reads packages: no node_modules,
//    tsconfig.json or package.json of this repository or any folder above. Findings are relative to the copy's root.
// 3. Copies ../../licenses/eslint-plugin-n/LICENSE and writes provenance.json and LICENSES.md: every copied test, every
//    case with its upstream test file, the upstream fixture trees and package files beside them, and Rerail's own
//    package.json files.
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
// every case, so such a case is written as CommonJS (`.js`/`.ts`), the only module system where Node defines them. A
// module case at an upstream fixture path (its `filename`) keeps the path, written as `.mjs`/`.mts` where its `.js`/`.ts`
// name would be CommonJS beside its package.json. A case whose `filename` lies in an upstream fixture tree
// (tests/fixtures/<tree>/…) gets that whole tree, node_modules, dotfiles and tsconfig files included, copied into its
// case directory (fixtures/<rule>/<kind>-NNN/<tree>/…), its own file written over any upstream file at its path: the
// packages, manifests, tsconfig files and modules its imports resolve against, read the same by ESLint and Rerail.
// Each fixture's analysis unit is the fixture, its nearest package.json (the one the plugin and Node read for it)
// and that folder's `.npmignore`/`.gitignore`, so Rerail reads the same package: its `type`, `bin`, `engines` and
// published files; Rerail's engine reads the rest of the case directory from disk. `source.units` names each case's
// origin, the rule whose test file it came from; each case also gets a rename variant under fixtures/renamed/
// (scripts/pack-data/eslint-expected.mjs `writeRenameVariants`) with copies of its package files, linted like any case
// and marked `renamedFrom` it. A case whose only configuration
// is the configured Node version (`options: [{ version }]` or `settings: { n: { version } }`) is written in a
// directory of its own beside a package.json (ours) whose engines.node is that version, which the plugin reads when
// no option sets one, so it is recorded under default options with the range its case states.
import { copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { createRequire, isBuiltin, registerHooks } from "node:module";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertCommit, lint, pinnedRequire, REPOSITORY, withOrigins, writeExpectedFindings, writeRenameVariants } from "../../../../scripts/pack-data/eslint-expected.mjs";
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
  "file-extension-in-import",
  "global-require",
  "handle-callback-err",
  "hashbang",
  "no-callback-literal",
  "no-deprecated-api",
  "no-exports-assign",
  "no-extraneous-import",
  "no-extraneous-require",
  "no-missing-import",
  "no-missing-require",
  "no-mixed-requires",
  "no-new-require",
  "no-path-concat",
  "no-process-env",
  "no-process-exit",
  "no-restricted-import",
  "no-restricted-require",
  "no-sync",
  "no-top-level-await",
  "no-unpublished-bin",
  "no-unpublished-import",
  "no-unpublished-require",
  "no-unsupported-features/es-builtins",
  "no-unsupported-features/es-syntax",
  "no-unsupported-features/node-builtins",
  "prefer-global/buffer",
  "prefer-global/console",
  "prefer-global/crypto",
  "prefer-global/process",
  "prefer-global/text-decoder",
  "prefer-global/text-encoder",
  "prefer-global/timers",
  "prefer-global/url",
  "prefer-global/url-search-params",
  "prefer-import/assert-strict",
  "prefer-node-protocol",
  "prefer-process-get-builtin-module",
  "prefer-promises/dns",
  "prefer-promises/fs",
];
const RULE_IDS = RULES.map(rule => `n/${rule}`);

/** The bindings Node's CommonJS module wrapper gives a file; an ES module has none of them. */
const COMMONJS_WRAPPER_NAMES = ["require", "module", "exports", "__dirname", "__filename"];

/** Ours: the fixtures' nearest package.json. Upstream's RuleTester defaults to commonjs and runs inside eslint-plugin-n. */
const casePackageJson = version => ({ private: true, description: "Rerail (not upstream): this case's configured Node version as engines.node, which eslint-plugin-n reads when no option sets one.", type: "commonjs", engines: { node: version } });
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
 * A fixture's analysis unit: the fixture (relative to `fixtures`), then its nearest package.json and that folder's
 * `.npmignore` and `.gitignore` where present (what the plugin reads of the package). The fixture comes first, as a
 * unit's first path is its key in `source.units`.
 */
function fixtureUnit(fixtures, path) {
  const folder = dirname(nearest(dirname(join(fixtures, path)), "package.json"));
  const packageFiles = PACKAGE_FILES.filter(name => existsSync(join(folder, name))).map(name => relative(fixtures, join(folder, name)).split(sep).join("/"));
  return [path, ...packageFiles.sort()];
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
async function lintTree(tools, cwd, files) {
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

/**
 * What the plugin reads above a file while it resolves an import (enhanced-resolve's node_modules, get-tsconfig's
 * tsconfig.json) or reads its package (the nearest package.json, a pnpm workspace): none may lie above a linted copy.
 */
const ANCESTOR_FILES = ["node_modules", "package.json", "tsconfig.json", "jsconfig.json", "pnpm-workspace.yaml"];

/**
 * Lints `files` (relative to `root`) in a copy of `root` (less any `.git`) under a fresh temporary directory, its real
 * path (macOS's /var is /private/var), so the plugin reads nothing above `root`: not the node_modules, tsconfig.json or
 * package.json of the repository or of any other folder above it. Findings are relative to the copy's root, as they
 * would be to `root`.
 */
async function record(tools, root, files) {
  const scratch = mkdtempSync(join(realpathSync(tmpdir()), "rerail-eslint-plugin-n-"));
  try {
    for (let directory = dirname(scratch); ; directory = dirname(directory)) {
      const found = ANCESTOR_FILES.find(name => existsSync(join(directory, name)));
      if (found !== undefined) throw new Error(`${join(directory, found)} lies above the temporary directory ${scratch}`);
      if (dirname(directory) === directory) break;
    }
    const copy = join(scratch, basename(root));
    cpSync(root, copy, { recursive: true, verbatimSymlinks: true, filter: source => basename(source) !== ".git" });
    return await lintTree(tools, copy, files);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

// ---- 1. upstream tests → fixtures -------------------------------------------------------------------------------

const TESTS = join(HERE, "tests");
const STUB = pathToFileURL(join(HERE, "stub-test-helpers.js")).href;
const EMPTY_MODULE = "data:text/javascript,export default {}";

function installHooks(toolsDirectory) {
  const toolsParent = pathToFileURL(join(toolsDirectory, "package.json")).href;
  const toolsRequire = createRequire(toolsParent);
  const testsUrl = pathToFileURL(TESTS).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      const fromTests = context.parentURL?.startsWith(testsUrl) === true;
      if (!fromTests) return nextResolve(specifier, context);
      if (specifier === "#test-helpers" || specifier.endsWith("/test-helpers.js")) return { url: STUB, shortCircuit: true };
      // The rule under test and sinon are never exercised: the stub only captures cases.
      if (specifier.includes("/lib/rules/") || specifier === "sinon") return { url: EMPTY_MODULE, shortCircuit: true };
      if (!/^(\.|\/|node:|file:|data:)/u.test(specifier) && !isBuiltin(specifier)) {
        // A require() (a test's createRequire) resolves from its caller's module whatever parentURL says.
        if (context.conditions?.includes("require") === true) return { url: pathToFileURL(toolsRequire.resolve(specifier)).href, shortCircuit: true };
        return nextResolve(specifier, { ...context, parentURL: toolsParent });
      }
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

/** A case's configured Node version when that is its only configuration (option or setting), else undefined. */
function configuredVersion(item) {
  const only = (value, key) => value !== null && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === 1 && key in value;
  const fromOptions = Array.isArray(item.options) && item.options.length === 1 && only(item.options[0], "version") ? item.options[0].version : undefined;
  const fromSettings = only(item.settings, "n") && only(item.settings.n, "version") ? item.settings.n.version : undefined;
  if (item.filename !== undefined || item.languageOptions !== undefined) return undefined;
  if (item.options !== undefined && item.settings !== undefined) return undefined;
  const version = item.options !== undefined ? fromOptions : item.settings !== undefined ? fromSettings : undefined;
  return typeof version === "string" ? version : undefined;
}

/** A package folder's files the plugin reads: the manifest, and what npm leaves out of the package (get-npmignore.js). */
const PACKAGE_FILES = ["package.json", ".npmignore", ".gitignore"];

/**
 * The name a case at upstream fixture path `name` is written under. Upstream lints it as `sourceType`, whatever Node
 * would run it as; a module case at a `.js`/`.ts` path whose nearest package.json (`manifest`, absolute: the upstream
 * fixture tree's, copied with it, else undefined for the fixtures' own) does not say `"type": "module"` would be linted
 * as CommonJS here, so it is written as `.mjs`/`.mts` in the same folder: the package facts the plugin reads from its
 * path stay the same.
 */
function moduleName(name, sourceType, manifest) {
  const extension = extname(name);
  if (sourceType !== "module" || (extension !== ".js" && extension !== ".ts")) return name;
  const type = manifest === undefined ? fixturePackageJson.type : JSON.parse(readFileSync(manifest, "utf8")).type;
  return type === "module" ? name : `${name.slice(0, -extension.length)}${extension === ".js" ? ".mjs" : ".mts"}`;
}

/**
 * The fixture path (relative to fixtures/) of one case, its case directory (`<rule>/<kind>-NNN`), and the upstream
 * fixture tree its file lies in (`tree`, the top-level folder of tests/fixtures/), copied whole into the case directory.
 */
function casePath(tools, upstream, rule, kind, index, run, item) {
  const directory = `${rule}/${kind}-${String(index).padStart(3, "0")}`;
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
    const top = relativeName.split(sep)[0];
    const tree = fixtureRoot !== undefined && top !== relativeName && existsSync(join(upstreamFixtures, top)) ? top : undefined;
    let manifest;
    if (tree !== undefined) {
      for (let folder = dirname(join(upstreamFixtures, relativeName)); manifest === undefined && folder.startsWith(upstreamFixtures + sep); folder = dirname(folder)) {
        if (existsSync(join(folder, "package.json"))) manifest = join(folder, "package.json");
      }
    }
    return { path: `${directory}/${moduleName(relativeName, sourceType, manifest).split(sep).join("/")}`, directory, tree, sourceType };
  }
  const extension = typescript ? (sourceType === "module" ? ".mts" : ".ts") : sourceType === "module" ? ".mjs" : ".js";
  const engines = configuredVersion(item);
  if (engines !== undefined) return { path: `${directory}/case${extension}`, directory, sourceType, engines };
  return { path: `${directory}${extension}`, directory, sourceType };
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
            configured: ["options", "settings", "languageOptions"].filter(key => item[key] !== undefined && (where.engines === undefined || key === "languageOptions")),
            ...where,
          });
        });
      }
    });
  }
  return { cases, skippedUpstream };
}

/**
 * Writes each case's file, with the upstream fixture tree it lies in copied whole into its case directory first (the
 * case's own file wins over an upstream file at its path), and returns the copied trees (fixtures-relative folders) by
 * their upstream folders.
 */
function writeFixtures(upstream, cases) {
  const fixtures = join(HERE, "fixtures");
  rmSync(fixtures, { recursive: true, force: true });
  mkdirSync(fixtures, { recursive: true });
  writeFileSync(join(fixtures, "package.json"), `${JSON.stringify(fixturePackageJson, null, 2)}\n`);
  const upstreamFixtures = join(upstream, "tests", "fixtures");
  const copies = new Map();
  const written = new Set();
  for (const entry of cases) {
    if (written.has(entry.path)) throw new Error(`duplicate fixture ${entry.path}`);
    written.add(entry.path);
    if (entry.tree !== undefined) {
      cpSync(join(upstreamFixtures, entry.tree), join(fixtures, entry.directory, entry.tree), { recursive: true });
      copies.set(`${entry.directory}/${entry.tree}`, `tests/fixtures/${entry.tree}`);
    }
    const target = join(fixtures, entry.path);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, entry.code);
    if (entry.engines !== undefined) writeFileSync(join(fixtures, entry.directory, "package.json"), `${JSON.stringify(casePackageJson(entry.engines), null, 2)}\n`);
  }
  return copies;
}

// ---- 3. licence and provenance ------------------------------------------------------------------------------------

/** Every file below `directory`, relative to it, `/`-separated. */
function filesBelow(directory, prefix = "") {
  return readdirSync(join(directory, prefix), { withFileTypes: true }).flatMap(entry => {
    const path = prefix === "" ? entry.name : `${prefix}/${entry.name}`;
    return entry.isDirectory() ? filesBelow(directory, path) : [path];
  });
}

/** A copy beside the rename variants: the file of the original the variant renames, unchanged. */
const variantCopy = original => `a copy of \`${original}\`, beside the rename variants it gives a package`;

/**
 * ../../licenses/eslint-plugin-n/LICENSE (verbatim) and the tool's provenance.json and LICENSES.md: the copied tests,
 * Rerail's fixtures/package.json and each case folder's package.json of ours (`casePackageJson`), the upstream fixture
 * trees copied beside the cases (`copies`: fixtures-relative folder → upstream folder), each kept case, and its rename
 * variant (`recorded`, withOrigins' result) with copies of its package files.
 */
function writeLicenseAndProvenance(upstream, cases, copies, recorded) {
  const directory = join(HERE, "..", "..", "licenses", TOOL);
  mkdirSync(directory, { recursive: true });
  copyFileSync(join(upstream, "LICENSE"), join(directory, "LICENSE"));
  const fixtures = join(HERE, "fixtures");
  const originals = new Map([
    ["fixtures/package.json", { source: RERAIL, how: "authored", detail: "Rerail's own, not copied: the fixtures' source type and engines (`fixturePackageJson`), under the pack's `LICENSE`" }],
  ]);
  for (const [copy, from] of [...copies].sort()) {
    for (const path of filesBelow(join(fixtures, copy))) {
      originals.set(`fixtures/${copy}/${path}`, { source: TOOL, from: `${from}/${path}`, how: "verbatim", detail: "verbatim (the upstream fixture tree a case's `filename` lies in)" });
    }
  }
  for (const entry of cases) {
    if (entry.engines !== undefined) {
      originals.set(`fixtures/${entry.directory}/package.json`, { source: RERAIL, how: "authored", detail: "Rerail's own, not copied: the case's configured Node version as engines.node (`casePackageJson`), under the pack's `LICENSE`" });
    }
    originals.set(`fixtures/${entry.path}`, { source: TOOL, from: entry.testFile, how: "extracted", detail: `extracted from ${entry.testFile} ${entry.origin}` });
  }
  const byPath = new Map(cases.map(entry => [`fixtures/${entry.path}`, entry]));
  // A variant's package files are copies of its original's (Rerail's, or upstream test data verbatim), not renamed.
  const variant = (_path, entry) => {
    const original = byPath.get(entry.renamedFrom);
    if (original !== undefined) return { ...entry, detail: `derived from ${original.testFile} ${original.origin} (\`${entry.renamedFrom}\` with its names and literals renamed)` };
    const { source, from, how } = originals.get(entry.renamedFrom);
    return { source, ...(from === undefined ? {} : { from }), how, detail: variantCopy(entry.renamedFrom) };
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
  const copies = writeFixtures(upstream, cases);
  const fixtures = join(HERE, "fixtures");
  // Each case is a case of the rule whose test file it came from. Its rename variant, under renamed/, renames the
  // case's code and copies its manifest.
  const units = new Map(cases.map(entry => [entry.path, fixtureUnit(fixtures, entry.path)]));
  const origins = new Map(cases.map(entry => [entry.path, [`n/${entry.rule}`]]));
  const { variants } = writeRenameVariants(fixtures, [...units.values()], origins);
  const fixtureRun = await record(tools, fixtures, [...cases.map(entry => entry.path), ...variants.map(variant => variant.paths.get(variant.of))]);
  const dropped = new Map(fixtureRun.skipped.map(entry => [entry.path, entry.reason]));
  const kept = cases.filter(entry => !dropped.has(entry.path));
  // A dropped case goes with its directory when it has one of its own (a copied fixture tree, a package.json of ours).
  for (const entry of cases.filter(entry => dropped.has(entry.path))) {
    if (entry.tree === undefined && entry.engines === undefined) {
      rmSync(join(fixtures, entry.path));
      continue;
    }
    rmSync(join(fixtures, entry.directory), { recursive: true });
    for (const copy of copies.keys()) if (copy.startsWith(`${entry.directory}/`)) copies.delete(copy);
  }
  const recorded = withOrigins(fixtures, kept.map(entry => entry.path).sort().map(path => units.get(path)), origins, variants, new Set(dropped.keys()), fixtureRun.findings);
  writeExpectedFindings(join(HERE, "fixtures.expected.json"), TOOL, VERSION, { kind: "fixtures", origin: `${UPSTREAM_REPOSITORY}@${UPSTREAM_COMMIT}`, rules: RULE_IDS, files: recorded.files, units: recorded.units }, recorded.findings);
  writeLicenseAndProvenance(upstream, kept, copies, recorded);

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
