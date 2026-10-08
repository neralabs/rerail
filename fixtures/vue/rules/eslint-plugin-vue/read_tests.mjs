// Reads eslint-plugin-vue's own RuleTester cases for the given rules and prints them as JSON (`generate.mjs` runs
// it in a child process; never CI):
//
//   node read_tests.mjs <eslint-plugin-vue checkout at v10.11.1> <tools dir> <rule>...
//
// Each `tests/lib/rules/<rule>.test.ts` is imported with Node's type stripping. Module hooks replace
// `tests/eslint-compat` with a RuleTester that records `run(...)` instead of running it, the rule module with a stub
// (the cases, not the implementation, are read), and `tests/test-utils/typescript` with a marker for its
// type-aware cases; bare imports resolve from the pinned tools directory. Output:
// `{ <rule>: { config, valid: [case], invalid: [case] } }`, where a parser object is replaced by its package name.

import { registerHooks } from "node:module";
import { createRequire } from "node:module";
import { existsSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [checkout, tools, ...rules] = process.argv.slice(2).map((value, index) => (index < 2 ? resolve(value) : value));
const toolsRequire = createRequire(join(tools, "package.json"));
const capture = new URL("data:text/javascript," + encodeURIComponent(`
  export class RuleTester {
    constructor(config = {}) { this.config = config; }
    run(name, rule, tests) { globalThis.__vueCases.push({ name, config: this.config, tests }); }
  }
  export const Linter = class {}; export const ESLint = class {}; export const FlatESLint = null;
`));
const stub = "data:text/javascript," + encodeURIComponent("export default { meta: {}, create() { return {}; } };");
const typed = "data:text/javascript," + encodeURIComponent(`
  export function getTypeScriptFixtureTestOptions() {
    return { languageOptions: { parserOptions: { project: ["<type-aware>"] } }, filename: "<type-aware>.vue" };
  }`);

registerHooks({
  resolve(specifier, context, next) {
    if (/(^|\/)eslint-compat$/.test(specifier)) return { url: capture.href, shortCircuit: true };
    if (/^\.\.\/(?:\.\.\/)*lib\/rules\/[^/]+$/.test(specifier)) return { url: stub, shortCircuit: true };
    if (/test-utils\/typescript$/.test(specifier)) return { url: typed, shortCircuit: true };
    const fromTests = context.parentURL?.startsWith(pathToFileURL(checkout).href);
    if (fromTests && !specifier.startsWith(".") && !specifier.startsWith("node:") && !specifier.startsWith("data:")
      && !specifier.startsWith("file:")) {
      return { url: pathToFileURL(toolsRequire.resolve(specifier)).href, shortCircuit: true, format: specifier.endsWith(".json") ? "json" : undefined };
    }
    if (fromTests && specifier.startsWith(".")) {
      const base = new URL(specifier, context.parentURL);
      for (const suffix of ["", ".ts", ".js", "/index.ts", "/index.js"]) {
        const url = new URL(base.href + suffix);
        if (existsSync(url) && statSync(url).isFile()) return { url: url.href, shortCircuit: true };
      }
    }
    return next(specifier, context);
  },
  // The upstream package is CommonJS by package.json; its tests are ES modules written in TypeScript.
  load(url, context, next) {
    if (url.startsWith(pathToFileURL(checkout).href) && url.endsWith(".ts")) {
      return { ...next(url, { ...context, format: "module-typescript" }), format: "module-typescript" };
    }
    return next(url, context);
  },
});

const PARSERS = new Map([
  [toolsRequire("vue-eslint-parser"), "vue-eslint-parser"],
  [toolsRequire("@typescript-eslint/parser"), "@typescript-eslint/parser"],
]);
const parserName = value => {
  for (const [parser, name] of PARSERS) {
    if (value === parser || value === parser.default || (parser.parseForESLint && value?.parseForESLint === parser.parseForESLint)) return name;
  }
  if (value?.meta?.name) return value.meta.name;
  // Some tests name a parser by `require.resolve(...)`: its path.
  const path = typeof value === "string" ? /node_modules\/((?:@[^/]+\/)?[^/]+)\//.exec(value) : null;
  if (path) return path[1];
  return typeof value === "string" ? value : "<unknown parser>";
};

/** A case as plain JSON: parsers by package name (also inside a `parserOptions.parser` map). */
function plain(value, key) {
  if ((key === "parser") && value !== null && typeof value === "object" && !value.parseForESLint && !value.parse) {
    return Object.fromEntries(Object.entries(value).map(([lang, parser]) => [lang, parserName(parser)]));
  }
  if (key === "parser") return parserName(value);
  if (value instanceof RegExp) return { regexp: String(value) };
  if (Array.isArray(value)) return value.map(item => plain(item));
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v, k)]));
  }
  return typeof value === "function" ? "<function>" : value;
}

globalThis.__vueCases = [];
// The tests run under vitest, which gives ES modules `require`; they use it only for `require.resolve(<parser>)`.
globalThis.require = toolsRequire;
const out = {};
for (const rule of rules) {
  globalThis.__vueCases.length = 0;
  await import(pathToFileURL(join(checkout, "tests/lib/rules", `${rule}.test.ts`)).href);
  const runs = globalThis.__vueCases.filter(run => run.name === rule);
  if (runs.length === 0) throw new Error(`${rule}: the test file ran no RuleTester for it`);
  out[rule] = runs.map(run => ({
    config: plain(run.config),
    valid: run.tests.valid.map(test => plain(typeof test === "string" ? { code: test } : test)),
    invalid: run.tests.invalid.map(test => plain(test)),
  }));
}
process.stdout.write(JSON.stringify(out));
