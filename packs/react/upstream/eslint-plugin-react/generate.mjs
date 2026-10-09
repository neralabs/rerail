// Recorder for the React pack's eslint-plugin-react rules (rerail/packs/react/candidates/conformance.json).
//
// Pins: eslint 9.39.5, eslint-plugin-react 7.37.5, @typescript-eslint/parser 8.70.1, typescript 5.9.3 (refuses any
// other); upstream jsx-eslint/eslint-plugin-react v7.37.5 = 2c98b83c451a4297edf1787d9a616e50687e27e8; corpus
// excalidraw = excalidraw/excalidraw@ed10ac7dca7e40f3f4a31269b4bfba980d0db41e (candidates/conformance.json).
//
// Usage: node generate.mjs [--tools <.rerail/tools/react>] [--corpora <.rerail/corpora/react>] [--no-corpora]
//
// 1. Copies each chosen rule's upstream test file verbatim to tests/lib/rules/<rule>.js.
// 2. Executes it with the stubs below (ours, not upstream's): a RuleTester that captures `run(name, rule, tests)` and
//    a `parsers.all` that returns the test list unexpanded, so case i is upstream's `valid[i]` / `invalid[i]`. Each
//    case's `code` is written verbatim to fixtures/<rule>/<valid|invalid>-NNN.<jsx|tsx> (or under the case directory
//    when the case names a `filename`). Cases needing Flow or Babel-only syntax are dropped.
// 3. Runs the pinned plugin (common.mjs `config`: every chosen rule, default options) over every fixture file and
//    each corpus, and writes fixtures.expected.json / corpus-<name>.expected.json.
// 4. Writes ../../licenses/eslint-plugin-react/UPSTREAM.md and LICENSE.
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { assertCommit } from "../../../../conformance/recorders/eslint-record.mjs";
import { execute, finishFixtures, options, recordCorpora, RULES, toolsRequire, UPSTREAM, writeLicense } from "../common.mjs";

const TOOL = "eslint-plugin-react";
const DIRECTORY = dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const { tools, corpora } = options(argv);
const require = toolsRequire(tools);
const upstream = join(tools, UPSTREAM[TOOL].checkout);
assertCommit(upstream, UPSTREAM[TOOL].commit);

/** Babel-only syntax: no parser of the pack reads it. */
const DROP_FEATURES = new Set(["flow", "bind operator", "do expressions"]);
/** Features the test harness runs only under a TypeScript (or Babel) parser: the case is TSX. */
const TS_FEATURES = new Set(["ts", "types", "decorators"]);
const PARSER_STUBS = { BABEL_ESLINT: "babel-eslint", "@BABEL_ESLINT": "@babel/eslint-parser", TYPESCRIPT_ESLINT: "typescript-eslint-parser", "@TYPESCRIPT_ESLINT": "@typescript-eslint/parser" };

/** Ours: the stub `tests/helpers/*` and rule modules the copied test files require. */
function stubs(captured) {
  class RuleTester {
    constructor(config) {
      this.config = config;
    }
    run(name, _rule, tests) {
      captured.push({ name, tests });
    }
  }
  const parsers = {
    ...PARSER_STUBS,
    all: tests => tests,
    disableNewTS: test => test,
    skipDueToMultiErrorSorting: false,
    babelParserOptions: test => test.parserOptions,
  };
  return id => {
    if (id.endsWith("helpers/ruleTester")) return RuleTester;
    if (id.endsWith("helpers/parsers")) return parsers;
    if (id.endsWith("helpers/getRuleDefiner")) return () => ({ defineRule() {} });
    if (id.endsWith("helpers/getESLintCoreRule")) return () => ({});
    if (id.includes("/lib/rules/")) return {};
    return require(id);
  };
}

/** The case's file extension, or why it is dropped. */
function classify(test) {
  if (typeof test === "string") return { extension: "jsx" };
  const features = new Set(test.features ?? []);
  const dropped = [...features].filter(feature => DROP_FEATURES.has(feature));
  if (dropped.length > 0) return { drop: `features ${dropped.join(", ")}` };
  if (test.parser === PARSER_STUBS.BABEL_ESLINT || test.parser === PARSER_STUBS["@BABEL_ESLINT"]) return { extension: "jsx" };
  if (test.parser === PARSER_STUBS.TYPESCRIPT_ESLINT || test.parser === PARSER_STUBS["@TYPESCRIPT_ESLINT"]) return { extension: "tsx" };
  if ([...features].some(feature => TS_FEATURES.has(feature))) return { extension: "tsx" };
  if (features.has("no-default") && !features.has("no-ts")) return { extension: "tsx" };
  return { extension: "jsx" };
}

const cases = [];
const copied = [];
for (const rule of RULES[TOOL]) {
  const name = rule.slice("react/".length);
  const relativeTest = `tests/lib/rules/${name}.js`;
  const target = join(DIRECTORY, relativeTest);
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(join(upstream, relativeTest), target);
  copied.push({ file: relativeTest, upstream: relativeTest });
  const captured = [];
  execute(target, stubs(captured));
  if (captured.length !== 1) throw new Error(`${relativeTest}: expected one run, captured ${captured.length}`);
  for (const kind of ["valid", "invalid"]) {
    captured[0].tests[kind].forEach((test, index) => {
      const code = typeof test === "string" ? test : test.code;
      const errors = kind === "valid" ? 0 : typeof test.errors === "number" ? test.errors : test.errors.length;
      cases.push({ rule, directory: name, kind, index, test, code, errors, origin: `${relativeTest} ${kind}[${index}]`, filename: typeof test === "string" ? undefined : test.filename, ...classify(test) });
    });
  }
}

const result = await finishFixtures(require, TOOL, DIRECTORY, cases, tools);
writeLicense(TOOL, upstream, copied, result.kept);
if (!argv.includes("--no-corpora")) {
  const summary = await recordCorpora(require, TOOL, DIRECTORY, corpora);
  for (const entry of summary) console.log(`corpus ${entry.corpus}: ${entry.files} files, ${entry.skipped} skipped, ${entry.findings} findings`);
}
