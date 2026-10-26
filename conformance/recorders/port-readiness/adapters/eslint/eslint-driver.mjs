// ESLint, as the port readiness gates run it, inside the pinned runtime of conformance/recorders/port-readiness/runtimes/<runtime>:
// ESLint's own rules (eslint:recommended from @eslint/js) or one plugin's (its configs), as the pack manifest names them.
//
//   node [--import <runtime>/node_modules/tsx/dist/loader.mjs] eslint-driver.mjs < request.json
//
// One JSON request on stdin; the JSON result is written to `request.out`.
// - {op: "facts"}: versions, each named rule's upstream metadata and its first approved configuration, the file
//   types the approved configurations lint and the modules of the parser they lint with, and ESLint's directive
//   comment names.
// - {op: "tests"}: every upstream test file loaded with upstream's own rule testers (ESLint's or typescript-eslint's
//   RuleTester, and the case testers the manifest names, such as eslint-plugin-unicorn's SnapshotRuleTester) under
//   stand-ins for its test framework (vitest, AVA); every case's code, expected outputs,
//   test configuration and expected reports (for not-overfit), and each case of each named rule linted by the pinned
//   ESLint with the rule as its first approved configuration sets it and without the globals and settings the case or
//   its RuleTester declares (for matches-upstream-tests).
//
// A pack whose manifest names an environment (`eslint.environment`, a shareable configuration such as
// eslint-config-next) is linted in it: the configuration's file globs, parsers and globals come before the approved
// configurations, and a test case's own language options give way to them.
import { createRequire, registerHooks } from "node:module";
import { existsSync, readFileSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join, matchesGlob, parse as parsePath, relative, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { format, isDeepStrictEqual } from "node:util";

import {
  caseFixture, casePath, caseWords, configNamed, declaresEnvironment, deprecationText, fileType, firstApproved as firstApprovedIn, frozenFileType,
  modulePackage, parsedAs, parserLanguage, vueReadsAsComponent, vueScriptLangs, vueScriptParser, withoutEnvironment, withSetting, wrappedScript, writtenExpectation,
} from "./eslint-upstream.ts";

const request = JSON.parse(readFileSync(0, "utf8"));
const { plugin, configs: approvedNames, release, fixtureParsers, namedParsers, eslintPackage, environment: environmentSetup, inScope } = request;
// Node identifies modules by real path, and the cache may be reached through a symlink.
const runtime = realpathSync(request.runtime);
const checkout = realpathSync(request.checkout);
const cache = realpathSync(request.cache);

// Upstream's test files and rules may print; only the result file carries data.
for (const method of ["log", "info", "warn", "error", "debug", "trace"]) console[method] = (...args) => process.stderr.write(`${format(...args)}\n`);

const runtimeRequire = createRequire(join(runtime, "package.json"));
// The ESLint upstream's tests run, which may be installed under an alias (facebook/react's eslint-v9).
const eslintDirectory = dirname(runtimeRequire.resolve(`${eslintPackage}/package.json`));
const fromEslint = (path) => runtimeRequire(join(eslintDirectory, path));
const { Linter } = runtimeRequire(eslintPackage);
const { builtinRules } = runtimeRequire(`${eslintPackage}/use-at-your-own-risk`);
const { FlatConfigArray } = fromEslint("lib/config/flat-config-array.js");
const { defaultConfig, defaultRuleTesterConfig } = fromEslint("lib/config/default-config.js");
const { directivesPattern } = fromEslint("lib/shared/directives.js");
// Read from the installed directory: a package's exports need not expose its package.json (eslint-plugin-unicorn's do not).
const packageVersion = (name) => JSON.parse(readFileSync(join(runtime, "node_modules", name, "package.json"), "utf8")).version;

const interop = (module) => (module && module.__esModule && module.default ? module.default : module);
const pluginModule = plugin === null ? null : interop(runtimeRequire(plugin.package));
/** Every rule of the upstream by name: ESLint's builtin rules, or the plugin's. */
const upstreamRules = plugin === null ? new Map(builtinRules) : new Map(Object.entries(pluginModule.rules));
const ruleId = (name) => (plugin === null ? name : `${plugin.namespace}/${name}`);
const configsPackage = plugin === null ? "@eslint/js" : plugin.package;
const configSource = plugin === null ? interop(runtimeRequire("@eslint/js")).configs : pluginModule.configs;
/**
 * The environment's configuration entries, as a project that spreads the shareable configuration lints: only their
 * file globs, ignores and language options (parsers, parser options, globals). Its plugins, rules and settings are not
 * the pack's. A Babel parser resolves the presets it names from its `cwd`, the environment's own runtime, as in a
 * project that installs it.
 */
const environmentRequire = environmentSetup === null ? null : createRequire(join(realpathSync(environmentSetup.runtime), "package.json"));
const environment = environmentRequire === null ? [] : (() => {
  const exported = interop(environmentRequire(environmentSetup.package));
  return (Array.isArray(exported) ? exported : [exported]).map((entry) => {
    const kept = {};
    for (const key of ["name", "files", "ignores"]) if (Object.hasOwn(entry, key)) kept[key] = entry[key];
    if (entry.languageOptions !== undefined) {
      const babelOptions = entry.languageOptions.parserOptions?.babelOptions;
      kept.languageOptions = babelOptions === undefined ? entry.languageOptions : {
        ...entry.languageOptions,
        parserOptions: { ...entry.languageOptions.parserOptions, babelOptions: { ...babelOptions, cwd: realpathSync(environmentSetup.runtime) } },
      };
    }
    return kept;
  });
})();

/** Each approved configuration as the flat config array a user spreads into theirs. */
const approved = approvedNames.map((name) => {
  const config = configNamed(configSource, name);
  if (config === undefined) throw new Error(`${configsPackage} has no configuration named ${name}`);
  return { name, configs: Array.isArray(config) ? config : [config] };
});
const firstApproved = (name) => firstApprovedIn(approved, ruleId(name));
/** The setting a rule is checked with: its first approved configuration's options, default options otherwise. */
const settingFor = (name) => ["error", ...(firstApproved(name)?.options ?? [])];

/** Of the file types ESLint plugins lint, by extension, those ESLint lints under any approved configuration. */
const CANDIDATE_EXTENSIONS = [".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx", ".vue"];
function lintedExtensions() {
  const linted = new Set();
  for (const { configs } of approved) {
    // As Linter reads a plain array: after ESLint's default configuration, with files matched under its cwd.
    const array = new FlatConfigArray([...environment, ...configs], { basePath: "/project" });
    array.normalizeSync();
    for (const extension of CANDIDATE_EXTENSIONS) if (array.getConfig(`/project/src/example${extension}`) !== undefined) linted.add(extension);
  }
  return CANDIDATE_EXTENSIONS.filter((extension) => linted.has(extension));
}
/** Whether upstream lints `.vue` files, as single-file components. */
const lintsComponents = lintedExtensions().includes(".vue");

/**
 * Whether a case upstream read as one script, of this file type and name, is recorded as the component that holds it
 * (wrappedScript), where upstream lints components: a case at a `.vue` name (eslint-plugin-vue's script cases at
 * `test.vue`, which its rules take for a component's script by that name), or a script of a type gates.yaml's
 * `rerail.in_scope` leaves out where it covers components (a case that names no file, which RuleTester lints as a
 * script), which the tool-pack's ports would otherwise never be judged on.
 */
function recordedAsComponent(extension, filename) {
  if (!lintsComponents || extension === ".vue") return false;
  return /\.vue$/u.test(filename ?? "") || (inScope !== null && inScope.includes(".vue") && !inScope.includes(extension));
}

/**
 * The module files of the parsers the approved configurations lint each file type with: those whose exports are the
 * parser, or hold its parse functions (typescript-eslint's configurations wrap its parser's `parseForESLint`).
 */
function parserModules() {
  const files = new Set();
  for (const { configs } of approved) {
    const array = new FlatConfigArray([...environment, ...configs], { basePath: "/project" });
    array.normalizeSync();
    for (const extension of CANDIDATE_EXTENSIONS) {
      const parser = array.getConfig(`/project/src/example${extension}`)?.languageOptions?.parser;
      if (parser === undefined || parser === null) continue;
      const holds = (exports) => exports === parser || exports?.default === parser
        || ["parseForESLint", "parse"].some((key) => typeof parser[key] === "function" && exports?.[key] === parser[key]);
      for (const module of loadedModules()) if (module !== undefined && holds(module.exports)) files.add(realpathSync(module.filename));
    }
  }
  return [...files].sort();
}

/** Every module loaded from the runtime and, when the pack names one, from the environment's runtime. */
function loadedModules() {
  return [...Object.values(runtimeRequire.cache), ...(environmentRequire === null ? [] : Object.values(environmentRequire.cache))];
}

function facts(names) {
  const rules = {};
  for (const name of names) {
    const rule = upstreamRules.get(name);
    const base = rule?.meta?.docs?.extendsBaseRule;
    rules[name] = rule === undefined ? null : {
      deprecated: deprecationText(rule.meta),
      requiresTypeChecking: rule.meta?.docs?.requiresTypeChecking === true,
      // A plugin rule that extends an ESLint rule delegates to that rule's implementation in the installed eslint.
      extendsBaseRule: base === true ? name : typeof base === "string" ? base : null,
      recommended: firstApproved(name),
    };
  }
  const directives = ["eslint", "eslint-env", "eslint-enable", "eslint-disable", "eslint-disable-line", "eslint-disable-next-line", "exported", "global", "globals"]
    .filter((directive) => directivesPattern.test(`${directive} `));
  return {
    eslint: packageVersion(eslintPackage),
    node: process.version,
    plugin: plugin === null ? null : packageVersion(plugin.package),
    configs: { package: configsPackage, version: packageVersion(configsPackage) },
    rules,
    extensions: lintedExtensions(),
    parsers: parserModules(),
    directives,
  };
}

// ---------------------------------------------------------------------------------------------------------------
// Upstream's test files, loaded with upstream's own RuleTester.

const checkoutRelative = (path) => relative(checkout, path).split(sep).join("/");
const releaseFrom = join(checkout, release.path) + sep;
const releaseTo = join(runtime, "node_modules", release.installed) + sep;
/** The test frameworks upstream's test files import, by the module that installs the globals below in their place. */
const FRAMEWORKS = { vitest: "./eslint-vitest.cjs", ava: "./eslint-ava.cjs" };
const shims = Object.fromEntries(Object.entries(FRAMEWORKS).map(([name, shim]) => [name, pathToFileURL(fileURLToPath(new URL(shim, import.meta.url))).href]));
const PLUGIN_MODULE = pathToFileURL(fileURLToPath(new URL("./eslint-plugin-module.cjs", import.meta.url))).href;
const checkoutURL = pathToFileURL(checkout + sep).href;
globalThis.validationPlugin = pluginModule;

// Test files load the rules from the installed package (the code the runtime runs) rather than the checkout's
// sources the package is published from, file for file where the package installs each source's build; a package that
// ships one bundle has none, and one whose build cannot stand in for its sources file for file (`release.sources`)
// keeps them, and testedRule() runs its installed rules in place of the sources' copies. The test frameworks are
// the globals below. A plugin whose tests load it by name as an ES module (its sources' default export) gets the
// installed build as that module. A test that names a path of the package in the checkout that only its build holds
// (eslint-plugin-vue's `../../../dist`, or `require.resolve('../../..')`, whose main is dist/index.js), which the
// checkout lacks until upstream builds it, gets the same path in the installed package.
const packageFrom = join(checkout, dirname(release.path));
const packageTo = join(runtime, "node_modules", ...release.installed.split("/").slice(0, release.installed.startsWith("@") ? 2 : 1));
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (Object.hasOwn(shims, specifier)) return { url: shims[specifier], shortCircuit: true };
    if (release.defaultExport && specifier === plugin?.package && context.parentURL?.startsWith(checkoutURL)) return { url: PLUGIN_MODULE, shortCircuit: true };
    let resolved;
    try {
      resolved = nextResolve(specifier, context);
    } catch (error) {
      const parent = context.parentURL?.startsWith("file:") ? fileURLToPath(context.parentURL) : null;
      const target = parent !== null && /^\.\.?(?:\/|$)/u.test(specifier) ? join(dirname(parent), specifier) : null;
      if (target === null || (target !== packageFrom && !target.startsWith(packageFrom + sep))) throw error;
      return nextResolve(packageTo + target.slice(packageFrom.length), context);
    }
    if (!resolved.url.startsWith("file:")) return resolved;
    const path = fileURLToPath(resolved.url);
    if (request.esmTests && context.conditions?.includes("require") && context.parentURL?.startsWith(checkoutURL) && !/^(?:\.\.?(?:\/|$)|\/|node:)/u.test(specifier)) {
      const shim = esmDefault(path);
      if (shim !== null) return { url: shim, shortCircuit: true };
    }
    if (release.sources || !path.startsWith(releaseFrom)) return resolved;
    const installed = releaseTo + path.slice(releaseFrom.length).replace(/\.[cm]?ts$/u, ".js");
    return existsSync(installed) ? { url: pathToFileURL(installed).href, shortCircuit: true } : resolved;
  },
});

/**
 * A module that reads as a CommonJS package does to Node's ES module loader, for a checkout file importing it
 * (`esm_tests`): the package's exports, with those exports as its default when it is marked `__esModule` without one.
 * null for a package that needs no stand-in.
 */
const esmDefaults = new Map();
function esmDefault(path) {
  if (!esmDefaults.has(path)) {
    let exports;
    try {
      exports = runtimeRequire(path);
    } catch {
      exports = null;
    }
    if (exports === null || typeof exports !== "object" || exports.__esModule !== true || "default" in exports) {
      esmDefaults.set(path, null);
    } else {
      const shim = join(dirname(request.out), `esm-default-${esmDefaults.size}.cjs`);
      writeFileSync(shim, `const target = require(${JSON.stringify(path)});\nmodule.exports = Object.defineProperties({}, { ...Object.getOwnPropertyDescriptors(target), default: { value: target, enumerable: true } });\n`);
      esmDefaults.set(path, pathToFileURL(shim).href);
    }
  }
  return esmDefaults.get(path);
}

/** The it() callbacks a typescript-eslint RuleTester run registers while it is collected. */
let collecting = null;
const afterAlls = [];
const inert = new Proxy(function inertFunction() {}, { get: () => inert, apply: () => inert });
const describe = (_name, body) => body();
describe.skip = () => undefined;
describe.only = describe;
describe.for = (rows) => (_name, body) => rows.forEach((row) => body(row));
describe.each = (rows) => (_name, body) => rows.forEach((row) => (Array.isArray(row) ? body(...row) : body(row)));
const it = (_name, body) => collecting?.push({ body, skip: false });
it.only = it;
it.skip = (_name, body) => collecting?.push({ body, skip: true });
it.for = it.each = () => () => undefined;
Object.assign(globalThis, {
  describe, it, test: it, expect: inert, vi: inert,
  beforeAll: (body) => body(), afterAll: (body) => afterAlls.push(body), beforeEach: () => undefined, afterEach: () => undefined,
});

/** The rules whose cases are linted, by rule object: test files import the same objects the runtime registers. */
const lintedRules = new Map();
/** Every RuleTester.run call of the test file being loaded. */
let runs = [];

/**
 * The named rule a RuleTester run tests, and the rule object its cases are linted with: the installed rule the run
 * passes, or, where upstream's tests build their own copy of a plugin rule (eslint-plugin-unicorn's test utilities wrap
 * its rules again; @vitest/eslint-plugin's tests import the sources its package ships bundled), the installed rule of
 * the name the run gives with the same metadata, which runs in the copy's place. null for any other rule.
 */
function testedRule(ruleName, rule) {
  const name = lintedRules.get(rule);
  if (name !== undefined) return { name, rule };
  const installed = plugin === null ? undefined : upstreamRules.get(ruleName);
  if (installed === undefined || !lintedRules.has(installed) || !isDeepStrictEqual(rule?.meta, installed.meta)) return null;
  return { name: lintedRules.get(installed), rule: installed };
}

/**
 * Run a tester's own run() with the tested rule while collecting the test it registers for each case, valid cases
 * first; each registered test lints its case when called.
 */
function collectCases(entry, run, tester) {
  if (entry.tested === null) return;
  collecting = entry.its = [];
  try {
    run.call(tester, entry.ruleName, entry.tested.rule, entry.tests);
  } catch (error) {
    entry.error = String(error?.message ?? error).split("\n")[0];
  } finally {
    collecting = null;
  }
}

// ESLint's RuleTester assembles each case's configuration inside run(); the cases are recorded here and linted below
// with that same assembly.
const ESLintRuleTester = fromEslint("lib/rule-tester/rule-tester.js");
ESLintRuleTester.prototype.run = function recordRun(ruleName, rule, tests) {
  runs.push({ tester: "eslint", testerConfig: this.testerConfig.slice(0, 2), ruleName, rule, tests, tested: testedRule(ruleName, rule) });
};

/**
 * The case being linted through a test its tester registered: the options it is linted with, for a case tester the
 * rule setting its configuration takes in place of the case's (`rule-to-test/<rule>`), and its first verify.
 */
let substitute = null;

// typescript-eslint's RuleTester lints through runRuleForItem, which takes the approved options in place of the case's.
let TsRuleTester = null;
try {
  TsRuleTester = runtimeRequire("@typescript-eslint/rule-tester").RuleTester;
} catch {
  // This runtime has no typescript-eslint RuleTester; its upstream's tests use ESLint's.
}
if (TsRuleTester !== null) {
  const run = TsRuleTester.prototype.run;
  TsRuleTester.prototype.run = function recordRun(ruleName, rule, tests) {
    const entry = { tester: "typescript-eslint", ruleName, rule, tests, tested: testedRule(ruleName, rule), its: null, error: null };
    runs.push(entry);
    // Upstream's own run() normalizes the cases and registers one it() per case, valid cases first.
    collectCases(entry, run, this);
  };
  const runRuleForItem = TsRuleTester.prototype.runRuleForItem;
  TsRuleTester.prototype.runRuleForItem = function withApprovedOptions(ruleName, rule, item) {
    if (substitute === null) return runRuleForItem.call(this, ruleName, rule, item);
    const recommended = { ...item };
    if (substitute.options.length === 0) delete recommended.options;
    else recommended.options = substitute.options;
    return runRuleForItem.call(this, ruleName, rule, recommended);
  };
}

/**
 * Upstream's own rule testers that the manifest names (`case_testers`, such as eslint-plugin-unicorn's
 * SnapshotRuleTester): each checkout module default-exports a class whose run(ruleName, rule, {valid, invalid})
 * registers one test per case, valid cases first, and each test lints its case with Linter.verify, the rule configured
 * as `rule-to-test/<ruleName>`.
 */
async function recordCaseTesters(paths) {
  for (const path of paths) {
    const Tester = (await import(pathToFileURL(join(checkout, path)).href)).default;
    const run = Tester.prototype.run;
    Tester.prototype.run = function recordRun(ruleName, rule, tests) {
      const entry = { tester: "case", ruleName, rule, tests, tested: testedRule(ruleName, rule), its: null, error: null };
      runs.push(entry);
      collectCases(entry, run, this);
    };
  }
}

const verify = Linter.prototype.verify;
// The first verify of a case is its initial lint, before any fix pass. Its configuration (the tester's merged with the
// case's) loses the globals and settings it declares, as for ESLint's own RuleTester, and a case tester's takes the
// approved setting.
Linter.prototype.verify = function firstVerify(code, config, filenameOrOptions) {
  if (substitute === null) return verify.call(this, code, config, filenameOrOptions);
  const stripped = withoutEnvironment(config);
  const linted = substitute.ruleKey === undefined ? stripped : withSetting(stripped, substitute.ruleKey, substitute.setting);
  const messages = verify.call(this, code, linted, filenameOrOptions);
  if (substitute.captured === null) {
    const filename = typeof filenameOrOptions === "string" ? filenameOrOptions : filenameOrOptions?.filename;
    substitute.captured = { messages, filename, code, ownEnvironment: declaresEnvironment(config), config: linted };
  }
  return messages;
};

function listFiles(directory) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "node_modules") found.push(...listFiles(path));
    } else if (entry.isFile()) found.push(path);
  }
  return found;
}

/** The literal directories a glob starts with, so only those are walked. */
const globRoot = (glob) => {
  const parts = glob.split("/");
  const wild = parts.findIndex((part) => /[*?[{]/u.test(part));
  return parts.slice(0, wild === -1 ? parts.length - 1 : wild).join("/");
};

const RULE_TESTER_PARAMETERS = ["name", "code", "filename", "options", "before", "after", "errors", "output", "only"];
const position = (message) => [message.line, message.column, message.endLine ?? null, message.endColumn ?? null];
const own = (item, key) => item !== null && typeof item === "object" && Object.hasOwn(item, key);

/** The real path of the module file a parser object was loaded from, or null when no loaded module exports it. */
const parserFiles = new Map();
function parserFile(parser) {
  if (!parserFiles.has(parser)) {
    const entry = loadedModules().find((module) => module?.exports === parser || module?.exports?.default === parser);
    parserFiles.set(parser, entry === undefined ? null : realpathSync(entry.filename));
  }
  return parserFiles.get(parser);
}

/** The script parser vue-eslint-parser (loaded from `file`) reads a case that is no component with, espree when none is picked. */
function vueReadsScriptWith(languageOptions, file, filename) {
  // vue-eslint-parser loads a script parser named by module from its own location.
  const picked = vueScriptParser(languageOptions?.parserOptions?.parser, vueScriptLangs(filename));
  const vueRequire = createRequire(file ?? join(runtime, "package.json"));
  return interop(typeof picked === "string" || picked === undefined ? vueRequire(picked ?? "espree") : picked);
}

/**
 * The file type of a case linted with these language options, or why it has none. A parser upstream's tests load from
 * the checkout returns a fixed tree: the tree another parser built for the case's own code, in a directory the
 * manifest's `fixture_parsers` names with that code's file type, or otherwise a tree whatever the code. Any other
 * parser reads the language its name declares, or the manifest's `parsers` gives for its package. vue-eslint-parser
 * reads a case (its file name, Linter's `<input>` when it names none, and its code) as a single-file component, or
 * else as one script in the language of the script parser its `parserOptions.parser` picks.
 */
function languageOf(languageOptions, filename, code) {
  const parser = languageOptions?.parser;
  const file = parser === undefined ? null : parserFile(parser);
  if (file !== null && file.startsWith(checkout + sep)) {
    const fixture = checkoutRelative(file);
    const extension = frozenFileType(fixture, fixtureParsers);
    if (extension !== undefined) return { extension };
    return { skipped: `upstream lints it with the test fixture parser ${fixture}, which returns a fixed tree whatever the code` };
  }
  const name = parser?.meta?.name ?? (typeof parser?.name === "string" ? parser.name : undefined);
  const packageName = file === null ? undefined : modulePackage(file);
  const language = parserLanguage(name, packageName, namedParsers);
  if (language === undefined) return { skipped: `upstream lints it with the parser ${name ?? packageName ?? "(unnamed)"}, a language the engine has no file type for` };
  if (language === "vue" && !vueReadsAsComponent(filename, code ?? "")) {
    return languageOf({ ...languageOptions, parser: vueReadsScriptWith(languageOptions, file, filename) }, filename, code);
  }
  const sourceType = languageOptions?.sourceType;
  return { extension: fileType({ language, jsx: languageOptions?.parserOptions?.ecmaFeatures?.jsx === true, sourceType }), script: language === "javascript" && sourceType === "script" };
}

/** The file type of a case a case tester linted with this configuration (Linter.verify's), or why it has none. */
function configuredLanguage(config, filename, code) {
  const configs = new FlatConfigArray(config, filename ? { basePath: parsePath(filename).root || undefined } : {});
  configs.normalizeSync();
  return languageOf(configs.getConfig(filename || "__placeholder__.js")?.languageOptions, filename, code);
}

/**
 * A configuration without the language options it sets: in a pack's environment, the environment decides the parser,
 * the ECMAScript version and source type, and the globals of every file.
 */
function withoutLanguage(config) {
  if (Array.isArray(config)) return config.map(withoutLanguage);
  if (config === null || typeof config !== "object" || !Object.hasOwn(config, "languageOptions")) return config;
  const { languageOptions: _languageOptions, ...rest } = config;
  return rest;
}

/**
 * One case of an ESLint RuleTester run, linted with the configuration RuleTester.run assembles and the given setting.
 * In a pack's environment, the case is linted as a project in it would lint a file of its name: the environment's
 * entries decide its language (parser, ECMAScript version, source type, globals), the case keeps its code, its name and
 * its rule setting, and a case upstream names no file for is named `nameless(<its own parser's file type>)`, the path
 * the engine evaluates it at. A script recordedAsComponent is linted as the component that holds it (wrappedScript),
 * with the language options it was read with and vue-eslint-parser handing its `<script>` to the parser that read it;
 * the result carries that component's code and how many lines it moved the script down.
 */
function lintESLintCase(entry, item, setting, nameless) {
  const named = own(item, "filename") ? item.filename : undefined;
  const at = defaultConfig[0].plugins["@"];
  const baseConfig = [
    {
      plugins: { "@": { parsers: { ...at.parsers }, rules: at.rules, languages: at.languages }, "rule-to-test": { rules: { [entry.ruleName]: entry.tested.rule } } },
      language: defaultConfig[0].language,
    },
    ...defaultRuleTesterConfig,
  ];
  const itemConfig = { ...item };
  for (const parameter of RULE_TESTER_PARAMETERS) delete itemConfig[parameter];
  const ownEnvironment = declaresEnvironment(entry.testerConfig) || declaresEnvironment(itemConfig);
  const assemble = (filename, inEnvironment, language) => {
    const strip = (config) => (inEnvironment ? withoutLanguage(withoutEnvironment(config)) : withoutEnvironment(config));
    const configs = new FlatConfigArray(strip(entry.testerConfig), { baseConfig, ...(filename ? { basePath: parsePath(filename).root || undefined } : {}) });
    if (inEnvironment) for (const environmentEntry of environment) configs.push(environmentEntry);
    configs.push(strip(itemConfig));
    if (language !== undefined) configs.push(language);
    configs.push({ rules: { [`rule-to-test/${entry.ruleName}`]: setting } });
    configs.normalizeSync();
    return configs;
  };
  // Linter matches configurations against this name when a case has none.
  let filename = named;
  let configs = assemble(filename, false);
  let language = languageOf(configs.getConfig(filename || "__placeholder__.js")?.languageOptions, filename, item.code);
  if (language.skipped !== undefined) return { language, filename, ownEnvironment };
  if (environment.length > 0) {
    filename = named ?? nameless(language.extension);
    configs = assemble(filename, true);
    language = languageOf(configs.getConfig(filename)?.languageOptions, filename, item.code);
    if (language.skipped !== undefined) return { language, filename, ownEnvironment };
  }
  let code = item.code;
  let lineOffset = 0;
  const scriptOptions = configs.getConfig(filename || "__placeholder__.js")?.languageOptions;
  const scriptParserFile = scriptOptions?.parser === undefined ? null : parserFile(scriptOptions.parser);
  if (recordedAsComponent(language.extension, filename) && !(scriptParserFile?.startsWith(checkout + sep) ?? false)) {
    // The component holding the script, linted by vue-eslint-parser with the script parser that read it alone.
    const scriptParserName = scriptOptions.parser?.meta?.name ?? (typeof scriptOptions.parser?.name === "string" ? scriptOptions.parser.name : undefined);
    const scriptParser = parserLanguage(scriptParserName, scriptParserFile === null ? undefined : modulePackage(scriptParserFile), namedParsers) === "vue"
      ? vueReadsScriptWith(scriptOptions, scriptParserFile, filename)
      : scriptOptions.parser;
    const wrapped = wrappedScript(item.code, language.extension, named, nameless(".vue"));
    ({ code, lineOffset } = wrapped);
    filename = wrapped.filename;
    configs = assemble(filename, environment.length > 0, { languageOptions: { parser: interop(runtimeRequire("vue-eslint-parser")), parserOptions: { parser: scriptParser } } });
    language = languageOf(configs.getConfig(filename)?.languageOptions, filename, code);
    if (language.skipped !== undefined) return { language, filename, ownEnvironment, code, lineOffset };
  }
  if (typeof item.before === "function") item.before();
  try {
    return { language, filename, ownEnvironment, code, lineOffset, messages: new Linter({ configType: "flat" }).verify(code, configs, filename), ruleId: `rule-to-test/${entry.ruleName}` };
  } catch (error) {
    // Upstream's own rule or parser throws on the case: upstream has no answer for it.
    return { language: { ...language, skipped: `upstream's linter throws on it: ${String(error?.message ?? error).split("\n")[0]}` }, filename, ownEnvironment, code, lineOffset };
  } finally {
    if (typeof item.after === "function") item.after();
  }
}

async function tests(globs, names, caseTesters) {
  for (const name of names) {
    const rule = upstreamRules.get(name);
    if (rule !== undefined) lintedRules.set(rule, name);
  }
  await recordCaseTesters(caseTesters);
  const files = [...new Set(globs.map(globRoot))].flatMap((root) => listFiles(join(checkout, root)))
    .filter((path) => globs.some((glob) => matchesGlob(checkoutRelative(path), glob)))
    .sort((a, b) => (checkoutRelative(a) < checkoutRelative(b) ? -1 : 1));
  const loaded = [];
  const texts = new Map();
  const configurations = new Set();
  const cases = Object.fromEntries(names.map((name) => [name, []]));
  const testFiles = Object.fromEntries(names.map((name) => [name, []]));

  for (const file of files) {
    const testFile = checkoutRelative(file);
    runs = [];
    let error = null;
    try {
      createRequire(file)(file);
    } catch (thrown) {
      error = String(thrown?.message ?? thrown).split("\n")[0];
    }
    const fileRuns = runs;
    loaded.push({ file: testFile, runs: fileRuns.length, error });

    fileRuns.forEach((entry, run) => {
      const valid = Array.isArray(entry.tests?.valid) ? entry.tests.valid : [];
      const invalid = Array.isArray(entry.tests?.invalid) ? entry.tests.invalid : [];
      for (const item of [...valid, ...invalid]) {
        const words = caseWords(item);
        for (const code of [words.code, ...words.outputs]) texts.set(`${words.jsx}\0${code}`, { code, jsx: words.jsx });
        configurations.add(words.configuration);
      }
      if (entry.tested === null) return;
      const { name } = entry.tested;
      if (!testFiles[name].includes(testFile)) testFiles[name].push(testFile);
      const approvedOptions = firstApproved(name)?.options ?? [];
      const items = [...valid.map((item, index) => ({ kind: "valid", index, item })), ...invalid.map((item, index) => ({ kind: "invalid", index, item }))];
      const its = entry.tester === "eslint" ? null : entry.its ?? [];
      const unregistered = its === null || its.length === items.length
        ? null
        : `upstream's RuleTester registered ${its.length} test(s) for ${items.length} case(s)${entry.error === null ? "" : `: ${entry.error}`}`;

      items.forEach(({ kind, index, item: raw }, order) => {
        const item = typeof raw === "string" ? { code: raw } : raw;
        const ownOptions = own(item, "options") ? item.options : undefined;
        const base = { fixture: caseFixture(testFile, run, kind, index), variant: ownOptions === undefined ? "." : `options ${JSON.stringify(ownOptions)}`, findings: [], markup: null };
        const at = (filename, extension, skipped, code = item.code) => {
          const path = casePath(name, testFile, run, kind, index, filename, checkout, cache, extension);
          const type = parsedAs(path, extension);
          return { ...base, code, path, ...(type === undefined ? {} : { parsedAs: type }), skipped };
        };
        let outcome;
        if (entry.tester === "eslint") {
          const linted = lintESLintCase(entry, item, settingFor(name), (extension) => casePath(name, testFile, run, kind, index, undefined, checkout, cache, extension));
          const extension = linted.language.extension ?? ".js";
          if (linted.language.skipped !== undefined) return void cases[name].push(at(linted.filename, extension, linted.language.skipped, linted.code));
          outcome = { messages: linted.messages, ruleId: linted.ruleId, filename: linted.filename, extension, script: linted.language.script === true, ownEnvironment: linted.ownEnvironment, code: linted.code, lineOffset: linted.lineOffset };
        } else {
          if (environment.length > 0) throw new Error(`${testFile}: a pack environment applies to ESLint's RuleTester only, not to ${entry.tester === "case" ? "a case tester" : "typescript-eslint's RuleTester"}`);
          // typescript-eslint's RuleTester names every case's file (file.ts by default); a case tester's case without one
          // is linted as <input>, the file type its parser reads.
          const caseTester = entry.tester === "case";
          const fallback = typeof item.filename === "string" ? item.filename : caseTester ? undefined : "file.ts";
          const fallbackExtension = caseTester ? ".js" : ".ts";
          const registration = unregistered === null ? its[order] : undefined;
          if (registration === undefined) return void cases[name].push(at(fallback, fallbackExtension, unregistered));
          if (registration.skip) return void cases[name].push(at(fallback, fallbackExtension, "upstream skips it (skip: true)"));
          const ruleKey = caseTester ? `rule-to-test/${entry.ruleName}` : `@rule-tester/${entry.ruleName}`;
          substitute = caseTester ? { options: approvedOptions, captured: null, ruleKey, setting: settingFor(name) } : { options: approvedOptions, captured: null };
          let thrown = null;
          try {
            // An AVA test takes its execution context, whose assertions the result does not depend on.
            if (caseTester) registration.body(inert);
            else registration.body();
          } catch (error_) {
            thrown = String(error_?.message ?? error_).split("\n")[0];
          }
          const captured = substitute.captured;
          substitute = null;
          if (captured === null) return void cases[name].push(at(fallback, fallbackExtension, `upstream's RuleTester did not lint it: ${thrown ?? "no verify call"}`));
          if (caseTester) {
            const language = configuredLanguage(captured.config, captured.filename, captured.code);
            const extension = language.extension ?? ".js";
            if (language.skipped !== undefined) return void cases[name].push(at(captured.filename, extension, language.skipped));
            outcome = { messages: captured.messages, ruleId: ruleKey, filename: captured.filename, extension, script: language.script === true, ownEnvironment: captured.ownEnvironment };
          } else {
            outcome = { messages: captured.messages, ruleId: ruleKey, filename: captured.filename ?? fallback, extension: ".ts", script: false, ownEnvironment: captured.ownEnvironment };
          }
        }
        const fatal = outcome.messages.find((message) => message.fatal);
        if (fatal !== undefined) return void cases[name].push(at(outcome.filename, outcome.extension, `upstream's parser rejects it: ${fatal.message}`, outcome.code));
        // The written expectation is of the case's own code, which a wrapped script moved lineOffset lines down. A script
        // recorded as a component under a `.vue` name it did not have is another file than the one its errors describe.
        const lineOffset = outcome.lineOffset ?? 0;
        const renamed = lineOffset > 0 && !/\.vue$/u.test(own(item, "filename") ? item.filename : "");
        const written = writtenExpectation(kind, item.errors, ownOptions, approvedOptions, isDeepStrictEqual);
        cases[name].push({
          ...at(outcome.filename, outcome.extension, null, outcome.code),
          script: outcome.script,
          findings: outcome.messages.filter((message) => message.ruleId === outcome.ruleId).map(position),
          // A case that declares globals or settings was written for an environment other than the approved one.
          markup: outcome.ownEnvironment || renamed ? null : written?.map(([line, column, endLine, endColumn]) => [line + lineOffset, column, endLine === null ? null : endLine + lineOffset, endColumn]) ?? null,
        });
      });
    });
    for (const body of afterAlls.splice(0)) {
      try {
        body();
      } catch {
        // Cache clearing between test files; a failure changes no result.
      }
    }
  }
  return { files: loaded, texts: [...texts.keys()].sort().map((key) => texts.get(key)), configurations: [...configurations].sort(), testFiles, cases };
}

const operations = {
  facts: () => facts(request.names),
  tests: () => tests(request.tests, request.names, request.caseTesters ?? []),
};
if (!(request.op in operations)) throw new Error(`unknown op ${request.op}`);
writeFileSync(request.out, JSON.stringify(await operations[request.op]()));
// Upstream's test files can leave handles open (project services, timers); the result is written.
process.exit(0);
