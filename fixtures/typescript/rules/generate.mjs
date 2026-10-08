// Records where ESLint 10.11.0 and typescript-eslint 8.70.1 report the TypeScript pack's T4 rules
// (rerail/conformance/languages/typescript/pack.json), in the format rerail/conformance/languages/pack.ts reads:
//   eslint/fixtures.expected.json, typescript-eslint/fixtures.expected.json  (the rules' own RuleTester cases)
//   <tool>/corpus-<name>.expected.json                                       (the pack's pinned corpora)
// Run offline by a maintainer, never in CI:
//   node generate.mjs --tools <dir> --eslint <checkout> --typescript-eslint <checkout> [--fixtures] [--corpus <name>=<checkout>]...
// <dir> holds node_modules with exactly eslint@10.11.0, @typescript-eslint/parser and
// @typescript-eslint/eslint-plugin @8.70.1, typescript@5.9.3 and tsx (to load typescript-eslint's .ts tests).
// The checkouts are github.com/eslint/eslint at v10.11.0 (3c0b7c6e1fe2dec778b97a996018126f22d437ae) and
// github.com/typescript-eslint/typescript-eslint at v8.70.1 (23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7).
//
// Fixtures: each rule's test file is loaded with a stub RuleTester that records its cases instead of linting.
// A case with rule options is left out (the pack's port is the rule with its default options). Every other case's
// code is written to <tool>/fixtures/<rule>/<n>.<ext> and linted, with every pack rule of the tool on, under the
// pack's one configuration (CONFIG below: ESLint's defaults, typescript-eslint's parser for TypeScript files), so
// the recording is the linter's answer under the configuration the ports read, not the case's own expectation.
// A case's extension is its parser's: .ts (.tsx with JSX) for typescript-eslint's parser, .js (.jsx) for espree,
// or the TypeScript-family extension of its filename (.d.ts, .tsx, .mts, .cts). Code the configuration cannot
// parse is left out. Each kept file is one analysis unit.
//
// Corpora: every file the runner scores (TypeScript and JavaScript files under the corpus's paths, outside
// directories the host ignores and the pack's excludes) is linted with every pack rule of the tool on. Files ESLint
// cannot parse are `skipped`. Columns are ESLint's, converted from UTF-16 code units to code points, 1-based.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createRequire, registerHooks } from "node:module";
import { dirname, join, matchesGlob, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPOSITORY = resolve(HERE, "..", "..", "..");
const PACK = JSON.parse(readFileSync(join(REPOSITORY, "conformance", "languages", "typescript", "pack.json"), "utf8"));
const VERSIONS = { eslint: "10.11.0", "@typescript-eslint/parser": "8.70.1", "@typescript-eslint/eslint-plugin": "8.70.1", typescript: "5.9.3" };
const CHECKOUTS = {
  eslint: { repository: "https://github.com/eslint/eslint", commit: "3c0b7c6e1fe2dec778b97a996018126f22d437ae" },
  "typescript-eslint": { repository: "https://github.com/typescript-eslint/typescript-eslint", commit: "23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7" },
};
/** The host's `IGNORED_DIRECTORIES` (rerail/src/executable-policy-enforcer.ts), which the runner's walk skips. */
const IGNORED_DIRECTORIES = new Set([".git", ".hg", ".svn", ".rerail", ".venv", "__pycache__", "node_modules", "dist", "build", ".next", ".turbo", "coverage", "target", "vendor"]);
const ECMA_EXTENSIONS = [".ts", ".mts", ".cts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];
const TYPESCRIPT_EXTENSIONS = [".d.ts", ".ts", ".tsx", ".mts", ".cts"];

function parseArguments(argv) {
  const options = { corpora: new Map(), fixtures: false };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === "--fixtures") options.fixtures = true;
    else if (flag === "--corpus") {
      const [name, directory] = argv[++index].split("=");
      options.corpora.set(name, resolve(directory));
    } else if (["--tools", "--eslint", "--typescript-eslint"].includes(flag)) options[flag.slice(2)] = resolve(argv[++index]);
    else throw new Error(`unknown argument ${flag}`);
  }
  for (const key of ["tools", "eslint", "typescript-eslint"]) if (options[key] === undefined) throw new Error(`--${key} is required`);
  return options;
}

const options = parseArguments(process.argv.slice(2));
const toolsRequire = createRequire(join(options.tools, "package.json"));
for (const [name, version] of Object.entries(VERSIONS)) {
  const found = toolsRequire(`${name}/package.json`).version;
  if (found !== version) throw new Error(`${name} is ${found}; this generator records ${version}`);
}
for (const [source, { commit }] of Object.entries(CHECKOUTS)) {
  const head = execFileSync("git", ["-C", options[source], "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (head !== commit) throw new Error(`${options[source]} is at ${head}; this generator reads ${source} at ${commit}`);
}

const { ESLint } = toolsRequire("eslint");
const tsParser = toolsRequire("@typescript-eslint/parser");
const PLUGIN_PREFIX = "@typescript-eslint/";

/** Pack rules by tool: `upstream.rule` is the rule id ESLint reports (`no-debugger`, `@typescript-eslint/no-namespace`). */
const RULES = new Map();
for (const rule of PACK.rules) {
  if (!RULES.has(rule.upstream.tool)) RULES.set(rule.upstream.tool, []);
  RULES.get(rule.upstream.tool).push(rule.upstream.rule);
}
for (const rules of RULES.values()) rules.sort();
const versionOf = tool => PACK.tools.find(entry => entry.name === tool).version;

let optionsRuns = 0;
/**
 * The pack's configuration: ESLint's defaults (ecmaVersion latest, sourceType module, espree), typescript-eslint's
 * parser for TypeScript files. Concurrent linting needs cloneable options, so it is an options module that ESLint's
 * worker threads import; `rules` reach it through the environment.
 */
function optionsModule(cwd, rules, concurrency) {
  const path = join(options.tools, ".generate-shims", "eslint-options.mjs");
  mkdirSync(dirname(path), { recursive: true });
  const from = name => JSON.stringify(pathToFileURL(toolsRequire.resolve(name)).href);
  writeFileSync(
    path,
    `import tsParser from ${from("@typescript-eslint/parser")};
import tsPlugin from ${from("@typescript-eslint/eslint-plugin")};
const rules = JSON.parse(process.env.RERAIL_TYPESCRIPT_PACK_RULES);
export default {
  cwd: ${JSON.stringify(cwd)},
  overrideConfigFile: true,
  ignore: false,
  concurrency: ${JSON.stringify(concurrency)},
  overrideConfig: [
    {
      files: ${JSON.stringify(ECMA_EXTENSIONS.map(extension => `**/*${extension}`))},
      plugins: { "@typescript-eslint": tsPlugin },
      linterOptions: { reportUnusedDisableDirectives: "off" },
      rules: Object.fromEntries(rules.map(rule => [rule, "error"])),
    },
    { files: ["**/*.jsx"], languageOptions: { parserOptions: { ecmaFeatures: { jsx: true } } } },
    { files: ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"], languageOptions: { parser: tsParser } },
    { files: ["**/*.tsx"], languageOptions: { parserOptions: { ecmaFeatures: { jsx: true } } } },
  ],
};
`,
  );
  process.env.RERAIL_TYPESCRIPT_PACK_RULES = JSON.stringify(rules);
  // ESLint imports the module once per URL; each configuration gets its own.
  const url = pathToFileURL(path);
  url.search = `?run=${++optionsRuns}`;
  return url;
}

/** ESLint's 1-based UTF-16 column on `line`, as 1-based code points. */
function codePointColumn(lines, line, column) {
  const text = lines[line - 1] ?? "";
  return [...text.slice(0, column - 1)].length + 1;
}

/** ESLint's line terminators (`\r\n`, `\r`, `\n`, U+2028, U+2029). */
const splitLines = text => text.split(/\r\n|[\r\n\u2028\u2029]/u);

/** Lints `files` (relative to `cwd`) with `rules` on; returns findings and the files ESLint could not parse. */
async function lint(cwd, files, rules) {
  const eslint = await ESLint.fromOptionsModule(optionsModule(cwd, rules, files.length > 1000 ? "auto" : "off"));
  const findings = [];
  const unparsed = [];
  const wanted = new Set(rules);
  const chunk = 2000;
  for (let start = 0; start < files.length; start += chunk) {
    for (const result of await eslint.lintFiles(files.slice(start, start + chunk))) {
      const path = relative(cwd, result.filePath).split(sep).join("/");
      if (result.messages.some(message => message.fatal)) {
        unparsed.push(path);
        continue;
      }
      const reported = result.messages.filter(message => wanted.has(message.ruleId));
      if (reported.length === 0) continue;
      const lines = splitLines(readFileSync(result.filePath, "utf8"));
      for (const message of reported) findings.push({ rule: message.ruleId, path, line: message.line, column: codePointColumn(lines, message.line, message.column) });
    }
  }
  const order = (left, right) => (left < right ? -1 : left > right ? 1 : 0);
  findings.sort((left, right) => order(left.path, right.path) || left.line - right.line || left.column - right.column || order(left.rule, right.rule));
  return { findings, unparsed: unparsed.sort(order) };
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

// ---------------------------------------------------------------------------------------------------------------
// Fixture extraction: stub RuleTesters record each test file's runs.

const RUNS = [];
class EslintRuleTesterStub {
  constructor(config = {}) {
    this.config = config;
  }
  static describe = (_text, method) => method();
  static it = () => undefined;
  static itOnly = () => undefined;
  static only = item => (typeof item === "object" ? { ...item, only: true } : { code: item, only: true });
  static setDefaultConfig() {}
  run(name, _rule, tests) {
    RUNS.push({ source: "eslint", name, config: this.config, tests });
  }
}
class TypescriptEslintRuleTesterStub {
  constructor(config = {}) {
    this.config = config;
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
    RUNS.push({ source: "typescript-eslint", name, config: this.config, tests });
  }
}
const noFormat = (raw, ...keys) => String.raw({ raw }, ...keys);
const STUBS = Symbol.for("rerail.typescriptPack.ruleTesterStubs");
globalThis[STUBS] = { eslint: EslintRuleTesterStub, typescriptEslint: { RuleTester: TypescriptEslintRuleTesterStub, noFormat } };
for (const name of ["describe", "it", "test"]) globalThis[name] ??= Object.assign((_text, method) => (name === "describe" ? method() : undefined), { skip: () => undefined, only: () => undefined, each: () => () => undefined, for: () => () => undefined });

function installHooks() {
  const shim = (name, body) => {
    const path = join(options.tools, ".generate-shims", name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, body);
    return pathToFileURL(path).href;
  };
  const eslintShim = shim("eslint-rule-tester.cjs", `module.exports = globalThis[Symbol.for("rerail.typescriptPack.ruleTesterStubs")].eslint;\n`);
  const typescriptShim = shim("typescript-eslint-rule-tester.cjs", `module.exports = globalThis[Symbol.for("rerail.typescriptPack.ruleTesterStubs")].typescriptEslint;\n`);
  const emptyShim = shim("rule.cjs", "module.exports = { meta: {}, create() { return {}; } };\n");
  const eslintTester = pathToFileURL(join(options.eslint, "lib", "rule-tester")).href;
  const ruleDirectories = [join(options.eslint, "lib", "rules"), join(options["typescript-eslint"], "packages", "eslint-plugin", "src", "rules")].map(path => pathToFileURL(path).href);
  const toolsUrl = pathToFileURL(join(options.tools, "package.json")).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "@typescript-eslint/rule-tester") return { url: typescriptShim, format: "commonjs", shortCircuit: true };
      let resolved;
      try {
        resolved = nextResolve(specifier, context);
      } catch (error) {
        // The checkouts have no node_modules: a bare specifier resolves from the tools directory.
        if (specifier.startsWith(".") || specifier.startsWith("/")) throw error;
        resolved = nextResolve(specifier, { ...context, parentURL: toolsUrl });
      }
      if (resolved.url.startsWith(eslintTester)) return { url: eslintShim, format: "commonjs", shortCircuit: true };
      if (ruleDirectories.some(directory => resolved.url.startsWith(`${directory}/`))) return { url: emptyShim, format: "commonjs", shortCircuit: true };
      return resolved;
    },
  });
}

function testFile(tool, rule) {
  if (tool === "eslint") return join(options.eslint, "tests", "lib", "rules", `${rule}.js`);
  const name = rule.slice(PLUGIN_PREFIX.length);
  const directory = join(options["typescript-eslint"], "packages", "eslint-plugin", "tests", "rules");
  return [join(directory, `${name}.test.ts`), join(directory, name, `${name}.test.ts`)].find(existsSync);
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
function extensionOf(source, test, languageOptions) {
  const parser = languageOptions.parser ?? (source === "typescript-eslint" ? tsParser : undefined);
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

/** Every kept case of `tool`'s pack rules, written under `<tool>/fixtures/<rule>/`. */
function extractFixtures(tool) {
  const fixtures = join(HERE, tool, "fixtures");
  rmSync(fixtures, { recursive: true, force: true });
  const files = [];
  const counts = {};
  for (const rule of RULES.get(tool)) {
    const name = tool === "eslint" ? rule : rule.slice(PLUGIN_PREFIX.length);
    const path = testFile(tool, rule);
    if (path === undefined) throw new Error(`${rule}: no test file`);
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
          if (Array.isArray(test.options) && test.options.length > 0) {
            count.withOptions += 1;
            continue;
          }
          const languageOptions = merge(run.config.languageOptions, test.languageOptions);
          const chosen = extensionOf(tool, test, languageOptions);
          if (chosen.skip !== undefined) {
            count.skipped[chosen.skip] = (count.skipped[chosen.skip] ?? 0) + 1;
            continue;
          }
          const key = `${chosen.extension}\0${test.code}`;
          if (seen.has(key)) {
            count.duplicate += 1;
            continue;
          }
          seen.add(key);
          const file = `${name}/${String(seen.size).padStart(3, "0")}${chosen.extension}`;
          mkdirSync(join(fixtures, name), { recursive: true });
          writeFileSync(join(fixtures, file), test.code);
          files.push(file);
          count.written += 1;
        }
      }
    }
  }
  return { fixtures, files, counts };
}

async function recordFixtures(tool) {
  const { fixtures, files, counts } = extractFixtures(tool);
  const { findings, unparsed } = await lint(fixtures, files, RULES.get(tool));
  for (const path of unparsed) rmSync(join(fixtures, path));
  const unparsedSet = new Set(unparsed);
  const kept = files.filter(file => !unparsedSet.has(file));
  for (const path of unparsed) {
    const count = counts[tool === "eslint" ? path.split("/")[0] : `${PLUGIN_PREFIX}${path.split("/")[0]}`];
    count.unparsed = (count.unparsed ?? 0) + 1;
  }
  const source = CHECKOUTS[tool];
  writeJson(join(HERE, tool, "fixtures.expected.json"), {
    tool,
    version: versionOf(tool),
    source: { kind: "fixtures", origin: `${source.repository}@${source.commit}`, rules: RULES.get(tool), files: kept },
    findings,
  });
  console.log(`${tool} fixtures: ${kept.length} files, ${findings.length} findings`);
  for (const [rule, count] of Object.entries(counts)) console.log(`  ${rule}: ${JSON.stringify(count)}`);
}

// ---------------------------------------------------------------------------------------------------------------
// Corpora.

function corpusFiles(corpus, directory) {
  const files = [];
  const walk = path => {
    const absolute = join(directory, path);
    if (!statSync(absolute).isDirectory()) {
      files.push(path);
      return;
    }
    for (const entry of readdirSync(absolute, { withFileTypes: true })) {
      const child = path === "" ? entry.name : `${path}/${entry.name}`;
      if (entry.isDirectory()) {
        if (!IGNORED_DIRECTORIES.has(entry.name)) walk(child);
      } else if (entry.isFile()) files.push(child);
    }
  };
  for (const path of corpus.paths) walk(path);
  return [...new Set(files)]
    .filter(path => {
      const name = path.slice(path.lastIndexOf("/") + 1).toLowerCase();
      return ECMA_EXTENSIONS.some(extension => name.length > extension.length && name.endsWith(extension)) && !corpus.exclude.some(pattern => matchesGlob(path, pattern));
    })
    .sort();
}

async function recordCorpus(name, directory) {
  const corpus = PACK.corpora.find(entry => entry.name === name);
  if (corpus === undefined) throw new Error(`corpus ${name} is not in pack.json`);
  const head = execFileSync("git", ["-C", directory, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (head !== corpus.commit) throw new Error(`${directory} is at ${head}; the pack pins ${corpus.commit}`);
  const files = corpusFiles(corpus, directory);
  const allRules = [...RULES.values()].flat();
  const started = performance.now();
  const { findings, unparsed } = await lint(directory, files, allRules);
  console.log(`corpus ${name}: ${files.length} files, ${unparsed.length} unparsed, ${Math.round(performance.now() - started)} ms`);
  for (const [tool, rules] of RULES) {
    const own = new Set(rules);
    const recorded = findings.filter(finding => own.has(finding.rule));
    writeJson(join(HERE, tool, `corpus-${name}.expected.json`), {
      tool,
      version: versionOf(tool),
      source: { kind: "corpus", name, repository: corpus.repository, commit: corpus.commit, rules, skipped: unparsed },
      findings: recorded,
    });
    console.log(`  ${tool}: ${recorded.length} findings`);
  }
}

if (options.fixtures) {
  installHooks();
  const { register } = await import(pathToFileURL(toolsRequire.resolve("tsx/cjs/api")).href);
  register();
  for (const tool of RULES.keys()) await recordFixtures(tool);
}
for (const [name, directory] of options.corpora) await recordCorpus(name, directory);
if (!options.fixtures && options.corpora.size === 0) console.error("nothing to do: pass --fixtures and/or --corpus <name>=<checkout>");
