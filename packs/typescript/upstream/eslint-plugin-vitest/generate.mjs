// Records @vitest/eslint-plugin for the TypeScript pack's vitest ports (packs/typescript/conformance.json), in the
// format rerail/conformance/languages/pack.ts reads: eslint-plugin-vitest/fixtures.expected.json (the rules' own
// RuleTester cases). No vitest port ships, so it records no corpus: a corpus recording covers shipped ports only.
// Run offline by a maintainer, never in CI:
//   node generate.mjs --tools <dir> --vitest <checkout>
// <dir> holds node_modules with the runtime nera-rule-validation pins for @vitest/eslint-plugin 1.6.27
// (validation/upstream/vitest-eslint-plugin-1.6.27): eslint 10.0.0, @vitest/eslint-plugin 1.6.27,
// @typescript-eslint/parser 8.58.0 and tsx (to load the plugin's .ts tests). The checkout is
// github.com/vitest-dev/eslint-plugin-vitest at v1.6.27 (aad57d121647efe764463c37414a8cd67ec9cf92); only its tests/
// are read, and the plugin linted is the published one in <dir>.
//
// Fixtures: each rule's test file is loaded with a stub RuleTester that records its cases instead of linting. Every
// case is linted with every pack rule of the plugin on at its default options (a case's own options are ignored, as
// the V1 gates' upstream-tests gate ignores them) under typescript-eslint's parser, as the plugin's RuleTester
// (@typescript-eslint/rule-tester) parses every case, plus the case's own parserOptions; its globals and settings are
// left out. A case with another parser is left out. A case's code is written to a test file, fixtures/<rule>/<n>.test.ts
// (.test.tsx with JSX, or the TypeScript-family extension of its filename after `.test`). Code that does not parse is left out. Each kept file is
// one analysis unit.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire, registerHooks } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertCommit, lint, pinnedRequire, writeRecording } from "../../../../conformance/recorders/eslint-record.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const PACK = JSON.parse(readFileSync(join(HERE, "..", "..", "conformance.json"), "utf8"));
const TOOL = "eslint-plugin-vitest";
const PINS = { eslint: "10.0.0", "@vitest/eslint-plugin": "1.6.27", "@typescript-eslint/parser": "8.58.0", tsx: "4.21.0" };
const CHECKOUT = { repository: "https://github.com/vitest-dev/eslint-plugin-vitest", commit: "aad57d121647efe764463c37414a8cd67ec9cf92" };
const PREFIX = "vitest/";
const ECMA_EXTENSIONS = [".ts", ".mts", ".cts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];
const TYPESCRIPT_EXTENSIONS = [".d.ts", ".ts", ".tsx", ".mts", ".cts"];

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (["--tools", "--vitest"].includes(flag)) options[flag.slice(2)] = resolve(argv[++index]);
    else throw new Error(`unknown argument ${flag}`);
  }
  for (const key of ["tools", "vitest"]) if (options[key] === undefined) throw new Error(`--${key} is required`);
  return options;
}

const options = parseArguments(process.argv.slice(2));
const toolsRequire = pinnedRequire(options.tools, PINS);
assertCommit(options.vitest, CHECKOUT.commit);
const { ESLint } = toolsRequire("eslint");
const tsParser = toolsRequire("@typescript-eslint/parser");
const pluginModule = toolsRequire("@vitest/eslint-plugin");
const plugin = pluginModule.default ?? pluginModule;

/** The pack's vitest rules, as ESLint reports them (`vitest/no-focused-tests`). */
const RULES = PACK.rules.filter(rule => rule.upstream.tool === TOOL).map(rule => rule.upstream.rule).sort();
const VERSION = PACK.tools.find(entry => entry.name === TOOL).version;

/** The pack's configuration for the plugin: typescript-eslint's parser for every file, every pack rule on. */
function config(languages = []) {
  return [
    {
      files: ECMA_EXTENSIONS.map(extension => `**/*${extension}`),
      plugins: { vitest: plugin },
      languageOptions: { parser: tsParser },
      linterOptions: { reportUnusedDisableDirectives: "off" },
      rules: Object.fromEntries(RULES.map(rule => [rule, "error"])),
    },
    { files: ["**/*.tsx", "**/*.jsx"], languageOptions: { parserOptions: { ecmaFeatures: { jsx: true } } } },
    ...languages.map(({ files, languageOptions }) => ({ files, languageOptions })),
  ];
}

// ---------------------------------------------------------------------------------------------------------------
// Fixture extraction: a stub RuleTester records each test file's runs.

const RUNS = [];
class RuleTesterStub {
  constructor(testerConfig = {}) {
    this.config = testerConfig;
  }
  static afterAll = () => undefined;
  static describe = (_text, method) => method();
  static describeSkip = () => undefined;
  static it = () => undefined;
  static itOnly = () => undefined;
  static itSkip = () => undefined;
  static only = item => (typeof item === "object" ? { ...item, only: true } : { code: item, only: true });
  static setDefaultConfig() {}
  defineRule() {}
  run(name, _rule, tests) {
    RUNS.push({ name, config: this.config, tests });
  }
}
const STUB = Symbol.for("rerail.typescriptPack.vitestRuleTester");
globalThis[STUB] = { RuleTester: RuleTesterStub, noFormat: (raw, ...keys) => String.raw({ raw }, ...keys) };
for (const name of ["describe", "it", "test"]) globalThis[name] ??= Object.assign((_text, method) => (name === "describe" ? method() : undefined), { skip: () => undefined, only: () => undefined, each: () => () => undefined, for: () => () => undefined });

function installHooks() {
  const shim = (name, body) => {
    const path = join(options.tools, ".generate-shims", name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, body);
    return pathToFileURL(path).href;
  };
  const testerShim = shim("vitest-rule-tester.cjs", `module.exports = globalThis[Symbol.for("rerail.typescriptPack.vitestRuleTester")];\n`);
  const ruleDirectory = pathToFileURL(join(options.vitest, "src", "rules")).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "@typescript-eslint/rule-tester") return { url: testerShim, format: "commonjs", shortCircuit: true };
      let resolved;
      try {
        resolved = nextResolve(specifier, context);
      } catch (error) {
        if (specifier.startsWith(".") && context.parentURL?.startsWith(pathToFileURL(join(options.vitest, "tests")).href)) {
          // A rule's source: only its name is read (`rule.name`), so it is a stub named after the file.
          const target = new URL(specifier, context.parentURL).href;
          if (target.startsWith(`${ruleDirectory}/`)) {
            const name = target.slice(ruleDirectory.length + 1);
            return { url: shim(`vitest-rule-${name}.cjs`, `module.exports = { default: { name: ${JSON.stringify(name)}, meta: {}, create() { return {}; } } };\n`), format: "commonjs", shortCircuit: true };
          }
        }
        // The checkout has no node_modules: a bare specifier resolves from the tools directory.
        if (specifier.startsWith(".") || specifier.startsWith("/")) throw error;
        resolved = { url: pathToFileURL(toolsRequire.resolve(specifier)).href, shortCircuit: true };
      }
      return resolved;
    },
  });
}

const isObject = value => typeof value === "object" && value !== null && !Array.isArray(value);
const merge = (...layers) =>
  layers.reduce((merged, layer) => {
    if (!isObject(layer)) return merged;
    const out = { ...merged };
    for (const [key, value] of Object.entries(layer)) out[key] = isObject(value) && isObject(out[key]) ? merge(out[key], value) : value;
    return out;
  }, {});

/** Every kept case of the pack's vitest rules, written under `fixtures/<rule>/`, with each file's own parserOptions. */
function extractFixtures() {
  const fixtures = join(HERE, "fixtures");
  rmSync(fixtures, { recursive: true, force: true });
  const files = [];
  const languages = new Map();
  const counts = {};
  for (const rule of RULES) {
    const name = rule.slice(PREFIX.length);
    const path = join(options.vitest, "tests", `${name}.test.ts`);
    RUNS.length = 0;
    createRequire(path)(path);
    const runs = RUNS.filter(run => run.name === name);
    if (runs.length === 0) throw new Error(`${rule}: ${path} ran no RuleTester for ${name}`);
    const seen = new Set();
    const count = (counts[rule] = { cases: 0, withOptions: 0, skipped: {}, written: 0, duplicate: 0 });
    for (const run of runs) {
      for (const kind of ["valid", "invalid"]) {
        for (const entry of run.tests[kind] ?? []) {
          const test = typeof entry === "string" ? { code: entry } : entry;
          count.cases += 1;
          if (test.skip === true) {
            count.skipped.skip = (count.skipped.skip ?? 0) + 1;
            continue;
          }
          if (Array.isArray(test.options) && test.options.length > 0) count.withOptions += 1;
          const languageOptions = merge(run.config.languageOptions, test.languageOptions);
          if (languageOptions.parser !== undefined && languageOptions.parser !== tsParser) {
            count.skipped["custom parser"] = (count.skipped["custom parser"] ?? 0) + 1;
            continue;
          }
          const parserOptions = languageOptions.parserOptions;
          const jsx = parserOptions?.ecmaFeatures?.jsx === true;
          const filename = typeof test.filename === "string" ? test.filename : "";
          const named = TYPESCRIPT_EXTENSIONS.find(extension => filename.endsWith(extension));
          const extension = named !== undefined && !(jsx && named !== ".tsx") ? named : jsx ? ".tsx" : ".ts";
          const key = `${extension}\0${JSON.stringify(parserOptions ?? null)}\0${test.code}`;
          if (seen.has(key)) {
            count.duplicate += 1;
            continue;
          }
          seen.add(key);
          // A test file's name, as the rules' cases are tests (the ports read `*.test.ts` and `*.spec.ts` files).
          const file = `${name}/${String(seen.size).padStart(3, "0")}${extension === ".d.ts" ? extension : `.test${extension}`}`;
          mkdirSync(join(fixtures, name), { recursive: true });
          writeFileSync(join(fixtures, file), test.code);
          files.push(file);
          if (parserOptions !== undefined) {
            const group = JSON.stringify(parserOptions);
            if (!languages.has(group)) languages.set(group, []);
            languages.get(group).push(file);
          }
          count.written += 1;
        }
      }
    }
  }
  return { fixtures, files, languages: [...languages].map(([group, members]) => ({ files: members, languageOptions: { parserOptions: JSON.parse(group) } })), counts };
}

async function recordFixtures() {
  const { fixtures, files, languages, counts } = extractFixtures();
  const { findings, skipped } = await lint(ESLint, fixtures, config(languages), files, RULES);
  for (const { path } of skipped) {
    rmSync(join(fixtures, path));
    const count = counts[`${PREFIX}${path.split("/")[0]}`];
    count.unparsed = (count.unparsed ?? 0) + 1;
  }
  const unparsed = new Set(skipped.map(({ path }) => path));
  const kept = files.filter(file => !unparsed.has(file)).sort();
  writeRecording(join(HERE, "fixtures.expected.json"), TOOL, VERSION, { kind: "fixtures", origin: `${CHECKOUT.repository}@${CHECKOUT.commit}`, rules: RULES, files: kept }, findings);
  console.log(`${TOOL} fixtures: ${kept.length} files, ${findings.length} findings`);
  for (const [rule, count] of Object.entries(counts)) console.log(`  ${rule}: ${JSON.stringify(count)}`);
}

installHooks();
const { register } = await import(pathToFileURL(toolsRequire.resolve("tsx/cjs/api")).href);
register();
await recordFixtures();
