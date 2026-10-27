// Records where ESLint 10.11.0 reports the JavaScript pack's ported rules (rerail/packs/javascript/conformance.json),
// in the format rerail/conformance/languages/pack.ts reads: eslint/fixtures.expected.json (the rules' own RuleTester
// cases).
// Run by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release or a re-snapshot):
//   node generate.mjs --tools <dir> --eslint <checkout>
// <dir> holds node_modules with exactly eslint@10.11.0, @typescript-eslint/parser@8.70.1 and typescript@6.0.3 (the
// runtime nera-rule-validation pins for ESLint 10.11.0, validation/upstream/eslint-10.11.0). The checkout is
// github.com/eslint/eslint at v10.11.0 (3c0b7c6e1fe2dec778b97a996018126f22d437ae).
//
// Each rule's test file is loaded with a stub RuleTester that records its cases instead of linting. Every case's code
// is written to eslint/fixtures/<rule>/<n>.<ext> and linted with every pack rule on at its default options (the pack's
// ports are the rules as eslint:recommended turns them on; a case's own rule options are ignored, as the pack's
// ports ignore them), under ESLint's defaults plus the case's own
// language: its ecmaVersion, sourceType and parserOptions (`with`, octal literals and `delete x` only parse as a
// script), and typescript-eslint's parser where the case names it. The case's globals and settings are left out, as
// a repository without them would lint the code. So the expected findings are the linter's answer, not the case's own
// expectation. A case's extension is its parser's: .ts (.tsx with JSX) for typescript-eslint's parser, .js (.jsx) for
// espree, or the TypeScript-family extension of its filename. Code that does not parse is left out, and so is code
// over MAX_CASE_BYTES: ESLint's performance stress cases (prefer-template's 1 MB string literal), which exhaust the
// engine's per-file AST work budget once every pack rule runs on the file. Each kept file is one analysis unit. Columns are ESLint's, converted from UTF-16 code units to code points, 1-based.
// `source.units` names each file's origin, the rule whose test file it came from. Each such file also gets a rename
// variant, renamed/<rule>/<n>.<ext> (scripts/pack-data/eslint-expected.mjs `writeRenameVariants`): its identifiers
// and string and number literal contents renamed, in the original's language, linted like any file, and marked
// `renamedFrom` the original.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire, registerHooks } from "node:module";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { isVariantPath, withOrigins, writeRenameVariants } from "../../../scripts/pack-data/eslint-expected.mjs";
import { expectedFindingsWritten } from "../../../scripts/s3/pack-fixtures.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const PACK = JSON.parse(readFileSync(join(HERE, "..", "conformance.json"), "utf8"));
const TOOL = "eslint";
const VERSIONS = { eslint: "10.11.0", "@typescript-eslint/parser": "8.70.1", typescript: "6.0.3" };
const CHECKOUT = { repository: "https://github.com/eslint/eslint", commit: "3c0b7c6e1fe2dec778b97a996018126f22d437ae" };
const ECMA_EXTENSIONS = [".ts", ".mts", ".cts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];
const TYPESCRIPT_EXTENSIONS = [".d.ts", ".ts", ".tsx", ".mts", ".cts"];
/** Larger cases are performance stress tests, not rule behaviour; see the header. */
const MAX_CASE_BYTES = 64 * 1024;

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (["--tools", "--eslint"].includes(flag)) options[flag.slice(2)] = resolve(argv[++index]);
    else throw new Error(`unknown argument ${flag}`);
  }
  for (const key of ["tools", "eslint"]) if (options[key] === undefined) throw new Error(`--${key} is required`);
  return options;
}

const options = parseArguments(process.argv.slice(2));
const toolsRequire = createRequire(join(options.tools, "package.json"));
for (const [name, version] of Object.entries(VERSIONS)) {
  const found = toolsRequire(`${name}/package.json`).version;
  if (found !== version) throw new Error(`${name} is ${found}; this generator records ${version}`);
}
const head = execFileSync("git", ["-C", options.eslint, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
if (head !== CHECKOUT.commit) throw new Error(`${options.eslint} is at ${head}; this generator reads eslint at ${CHECKOUT.commit}`);

const { ESLint } = toolsRequire("eslint");
const tsParser = toolsRequire("@typescript-eslint/parser");

/** The pack's ESLint rules: `upstream.rule` is the rule id ESLint reports (`no-debugger`). */
const RULES = PACK.rules.filter(rule => rule.upstream.tool === TOOL).map(rule => rule.upstream.rule).sort();
const VERSION = PACK.tools.find(entry => entry.name === TOOL).version;

/**
 * The pack's configuration: ESLint's defaults (ecmaVersion latest, sourceType module, espree), typescript-eslint's
 * parser for TypeScript files, then each group of fixtures' own language. Concurrent linting needs cloneable options,
 * so it is an options module that ESLint's worker threads import; the rules and the groups reach it through the
 * environment.
 */
function optionsModule(cwd, rules, languages) {
  const path = join(options.tools, ".generate-shims", "eslint-options.mjs");
  mkdirSync(dirname(path), { recursive: true });
  const from = name => JSON.stringify(pathToFileURL(toolsRequire.resolve(name)).href);
  writeFileSync(
    path,
    `import tsParser from ${from("@typescript-eslint/parser")};
const { rules, languages } = JSON.parse(process.env.RERAIL_JAVASCRIPT_PACK_EXPECTED);
export default {
  cwd: ${JSON.stringify(cwd)},
  overrideConfigFile: true,
  ignore: false,
  overrideConfig: [
    {
      files: ${JSON.stringify(ECMA_EXTENSIONS.map(extension => `**/*${extension}`))},
      linterOptions: { reportUnusedDisableDirectives: "off" },
      rules: Object.fromEntries(rules.map(rule => [rule, "error"])),
    },
    { files: ["**/*.jsx"], languageOptions: { parserOptions: { ecmaFeatures: { jsx: true } } } },
    { files: ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"], languageOptions: { parser: tsParser } },
    { files: ["**/*.tsx"], languageOptions: { parserOptions: { ecmaFeatures: { jsx: true } } } },
    ...languages.map(({ files, languageOptions }) => ({ files, languageOptions })),
  ],
};
`,
  );
  process.env.RERAIL_JAVASCRIPT_PACK_EXPECTED = JSON.stringify({ rules, languages });
  return pathToFileURL(path);
}

/** ESLint's 1-based UTF-16 column on `line`, as 1-based code points. */
function codePointColumn(lines, line, column) {
  const text = lines[line - 1] ?? "";
  return [...text.slice(0, column - 1)].length + 1;
}

/** ESLint's line terminators (`\r\n`, `\r`, `\n`, U+2028, U+2029). */
const splitLines = text => text.split(/\r\n|[\r\n\u2028\u2029]/u);

/** Lints `files` (relative to `cwd`) with `rules` on; returns findings and the files ESLint could not parse. */
async function lint(cwd, files, rules, languages) {
  const eslint = await ESLint.fromOptionsModule(optionsModule(cwd, rules, languages));
  const findings = [];
  const unparsed = [];
  const wanted = new Set(rules);
  for (const result of await eslint.lintFiles(files)) {
    const path = relative(cwd, result.filePath).split(sep).join("/");
    if (result.messages.some(message => message.fatal)) {
      unparsed.push(path);
      continue;
    }
    const reported = result.messages.filter(message => wanted.has(message.ruleId));
    if (reported.length === 0) continue;
    // ESLint reads the text without a leading byte order mark; the file's first line, as the runner counts it, has it.
    const text = readFileSync(result.filePath, "utf8");
    const bom = text.startsWith("\uFEFF") ? 1 : 0;
    const lines = splitLines(text.slice(bom));
    for (const message of reported) {
      const column = codePointColumn(lines, message.line, message.column) + (message.line === 1 ? bom : 0);
      findings.push({ rule: message.ruleId, path, line: message.line, column });
    }
  }
  const order = (left, right) => (left < right ? -1 : left > right ? 1 : 0);
  findings.sort((left, right) => order(left.path, right.path) || left.line - right.line || left.column - right.column || order(left.rule, right.rule));
  return { findings, unparsed: unparsed.sort(order) };
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
  console.log(expectedFindingsWritten(path, value.source.kind));
}

// ---------------------------------------------------------------------------------------------------------------
// Fixture extraction: a stub RuleTester records each test file's runs.

const RUNS = [];
class RuleTesterStub {
  constructor(config = {}) {
    this.config = config;
  }
  static describe = (_text, method) => method();
  static it = () => undefined;
  static itOnly = () => undefined;
  static only = item => (typeof item === "object" ? { ...item, only: true } : { code: item, only: true });
  static setDefaultConfig() {}
  run(name, _rule, tests) {
    RUNS.push({ name, config: this.config, tests });
  }
}
const STUBS = Symbol.for("rerail.javascriptPack.ruleTesterStub");
globalThis[STUBS] = RuleTesterStub;
for (const name of ["describe", "it", "test"]) globalThis[name] ??= Object.assign((_text, method) => (name === "describe" ? method() : undefined), { skip: () => undefined, only: () => undefined, each: () => () => undefined, for: () => () => undefined });

function installHooks() {
  const shim = (name, body) => {
    const path = join(options.tools, ".generate-shims", name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, body);
    return pathToFileURL(path).href;
  };
  const testerShim = shim("eslint-rule-tester.cjs", `module.exports = globalThis[Symbol.for("rerail.javascriptPack.ruleTesterStub")];\n`);
  const emptyShim = shim("rule.cjs", "module.exports = { meta: {}, create() { return {}; } };\n");
  const tester = pathToFileURL(join(options.eslint, "lib", "rule-tester")).href;
  const ruleDirectory = pathToFileURL(join(options.eslint, "lib", "rules")).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      let resolved;
      try {
        resolved = nextResolve(specifier, context);
      } catch (error) {
        // The checkout has no node_modules: a bare specifier resolves from the tools directory.
        if (specifier.startsWith(".") || specifier.startsWith("/")) throw error;
        resolved = { url: pathToFileURL(toolsRequire.resolve(specifier)).href, shortCircuit: true };
      }
      if (resolved.url.startsWith(tester)) return { url: testerShim, format: "commonjs", shortCircuit: true };
      if (resolved.url.startsWith(`${ruleDirectory}/`)) return { url: emptyShim, format: "commonjs", shortCircuit: true };
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

/** The extension a case is written with, or why it is left out. */
function extensionOf(test, languageOptions) {
  const parser = languageOptions.parser;
  if (parser !== undefined && parser !== tsParser) return { skip: "custom parser" };
  const parserOptions = languageOptions.parserOptions ?? {};
  if (parserOptions.project !== undefined || parserOptions.projectService !== undefined || parserOptions.programs !== undefined) return { skip: "type information" };
  const jsx = parserOptions.ecmaFeatures?.jsx === true;
  if (parser === tsParser) {
    const filename = typeof test.filename === "string" ? test.filename : "";
    const named = TYPESCRIPT_EXTENSIONS.find(extension => filename.endsWith(extension));
    if (named !== undefined && !(jsx && named !== ".tsx")) return { extension: named };
    return { extension: jsx ? ".tsx" : ".ts" };
  }
  return { extension: jsx ? ".jsx" : ".js" };
}

/** The case's own language, beyond what its extension gives: ecmaVersion, sourceType and parserOptions, or nothing. */
function languageOf(languageOptions) {
  const own = {};
  for (const key of ["ecmaVersion", "sourceType", "parserOptions"]) if (languageOptions[key] !== undefined) own[key] = languageOptions[key];
  return Object.keys(own).length === 0 ? undefined : own;
}

/** Every kept case of the pack's rules, written under `eslint/fixtures/<rule>/`, with each file's own language. */
function extractFixtures() {
  const fixtures = join(HERE, TOOL, "fixtures");
  rmSync(fixtures, { recursive: true, force: true });
  const files = [];
  const languages = new Map();
  const counts = {};
  for (const rule of RULES) {
    const path = join(options.eslint, "tests", "lib", "rules", `${rule}.js`);
    if (!existsSync(path)) throw new Error(`${rule}: no test file`);
    RUNS.length = 0;
    createRequire(path)(path);
    const runs = RUNS.filter(run => run.name === rule);
    if (runs.length === 0) throw new Error(`${rule}: ${path} ran no RuleTester for ${rule}`);
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
          if (Buffer.byteLength(test.code) > MAX_CASE_BYTES) {
            count.skipped["stress case"] = (count.skipped["stress case"] ?? 0) + 1;
            continue;
          }
          const languageOptions = merge(run.config.languageOptions, test.languageOptions);
          const chosen = extensionOf(test, languageOptions);
          if (chosen.skip !== undefined) {
            count.skipped[chosen.skip] = (count.skipped[chosen.skip] ?? 0) + 1;
            continue;
          }
          const { parser: _parser, ...ownOptions } = languageOptions;
          const language = languageOf(ownOptions);
          const key = `${chosen.extension}\0${JSON.stringify(language ?? null)}\0${test.code}`;
          if (seen.has(key)) {
            count.duplicate += 1;
            continue;
          }
          seen.add(key);
          const file = `${rule}/${String(seen.size).padStart(3, "0")}${chosen.extension}`;
          mkdirSync(join(fixtures, rule), { recursive: true });
          writeFileSync(join(fixtures, file), test.code);
          files.push(file);
          if (language !== undefined) {
            const group = JSON.stringify(language);
            if (!languages.has(group)) languages.set(group, []);
            languages.get(group).push(file);
          }
          count.written += 1;
        }
      }
    }
  }
  return { fixtures, files, languages: [...languages].map(([group, members]) => ({ files: members, languageOptions: JSON.parse(group) })), counts };
}

const { fixtures, files, languages, counts } = (installHooks(), extractFixtures());
// Each file is a case of the rule whose test file it came from (`<rule>/<n>.<ext>`); its rename variant joins its
// language group.
const origins = new Map(files.map(file => [file, [file.split("/")[0]]]));
const { variants, skipped: unrenamed } = writeRenameVariants(fixtures, files, origins);
const variantOf = new Map(variants.map(variant => [variant.of, variant.unit]));
const variantLanguages = languages.map(({ files: members, languageOptions }) => ({ files: [...members, ...members.flatMap(member => variantOf.get(member) ?? [])], languageOptions }));
const linted = await lint(fixtures, [...files, ...variants.map(variant => variant.unit)], RULES, variantLanguages);
const unparsedSet = new Set(linted.unparsed);
for (const path of linted.unparsed.filter(path => !isVariantPath(path))) {
  rmSync(join(fixtures, path));
  const count = counts[path.split("/")[0]];
  count.unparsed = (count.unparsed ?? 0) + 1;
}
const kept = files.filter(file => !unparsedSet.has(file));
const { files: units, units: origin, findings } = withOrigins(fixtures, kept, origins, variants, unparsedSet, linted.findings);
writeJson(join(HERE, TOOL, "fixtures.expected.json"), {
  tool: TOOL,
  version: VERSION,
  source: { kind: "fixtures", origin: `${CHECKOUT.repository}@${CHECKOUT.commit}`, rules: RULES, files: units, units: origin },
  findings,
});
console.log(`${TOOL} fixtures: ${kept.length} files, ${units.length - kept.length} rename variants (${unrenamed.length} units without one), ${findings.length} findings`);
for (const [rule, count] of Object.entries(counts)) console.log(`  ${rule}: ${JSON.stringify(count)}`);
