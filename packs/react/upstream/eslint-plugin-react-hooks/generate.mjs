// Recorder for the React pack's eslint-plugin-react-hooks rules (rerail/packs/react/conformance.json).
//
// Pins: eslint 9.39.5, eslint-plugin-react-hooks 7.1.1, eslint-plugin-react 7.37.5, @typescript-eslint/parser 8.70.1,
// typescript 5.9.3 (refuses any other); upstream facebook/react 1ddff43c41147b880c22eb363e07aade5a71c5d9 (the
// published 7.1.1's gitHead); corpus excalidraw = excalidraw/excalidraw@ed10ac7dca7e40f3f4a31269b4bfba980d0db41e
// (conformance.json).
//
// Usage: node generate.mjs [--tools <.rerail/tools/react>] [--corpora <.rerail/corpora/react>] [--no-corpora]
//
// 1. Copies the rules' upstream test files verbatim to tests/__tests__/.
// 2. Executes each with the stubs below (ours, not upstream's): `eslint-v7`/`eslint-v9` RuleTesters that capture every
//    `run(name, rule, tests)`, jest's `describe` that calls its body. The tests run the same case objects under several
//    parsers; each case is written once, verbatim, to fixtures/<rule>/<valid|invalid>-NNN.<jsx|tsx> (NNN numbers the
//    distinct cases in the order first captured; the license table names the run and index). A case some Babel run
//    reads is JSX, one only the @typescript-eslint runs read is TSX, and one only hermes-eslint reads (Flow component
//    and hook syntax) is dropped.
// 3. Runs the pinned plugins (common.mjs `config`) over every fixture file and each corpus.
// 4. Writes ../../licenses/react/UPSTREAM.md and LICENSE.
// 5. Writes source.json, the plugin's source facts (common.mjs `recordSource`), from every run of the two test files,
//    which are all of upstream's ESLint rule tests (`__tests__/ESLint*-test.js`).
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { assertCommit } from "../../../../conformance/recorders/eslint-record.mjs";
import { checkoutFiles } from "../../../../conformance/recorders/eslint-source.mjs";
import { execute, finishFixtures, options, recordCorpora, recordSource, toolsRequire, UPSTREAM, writeLicense } from "../common.mjs";

const TOOL = "eslint-plugin-react-hooks";
const DIRECTORY = dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const { tools, corpora } = options(argv);
const require = toolsRequire(tools);
const upstream = join(tools, UPSTREAM[TOOL].checkout);
assertCommit(upstream, UPSTREAM[TOOL].commit);

const TESTS = [
  { rule: "react-hooks/rules-of-hooks", file: "ESLintRulesOfHooks-test.js" },
  { rule: "react-hooks/exhaustive-deps", file: "ESLintRuleExhaustiveDeps-test.js" },
];
const TEST_DIRECTORY = "packages/eslint-plugin-react-hooks/__tests__";

/** Ours: the modules the copied test files require. */
function stubs(captured) {
  class RuleTester {
    run(name, _rule, tests) {
      captured.push({ name, tests });
    }
  }
  const rules = new Proxy({}, { get: () => ({}) });
  return id => {
    if (id === "eslint-v7" || id === "eslint-v9") return { RuleTester };
    if (id === "eslint-plugin-react-hooks") return { default: { rules } };
    return {};
  };
}

const copied = [];
const cases = [];
const runs = [];
for (const { rule, file } of TESTS) {
  const relativeTest = `tests/__tests__/${file}`;
  const target = join(DIRECTORY, relativeTest);
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(join(upstream, TEST_DIRECTORY, file), target);
  copied.push({ file: relativeTest, upstream: `${TEST_DIRECTORY}/${file}` });
  const captured = [];
  // The harness's local-testing filter (skip/only) runs unless CI is set; upstream's CI sets it.
  const previous = process.env.CI;
  process.env.CI = "true";
  execute(target, stubs(captured), { describe: (_name, body) => body() });
  if (previous === undefined) delete process.env.CI;
  else process.env.CI = previous;
  for (const { tests } of captured) runs.push({ testFile: `${TEST_DIRECTORY}/${file}`, rule: rule.slice("react-hooks/".length), tests });
  const seen = new Map();
  for (const { name, tests } of captured) {
    for (const kind of ["valid", "invalid"]) {
      tests[kind].forEach((test, index) => {
        let entry = seen.get(test);
        if (entry === undefined) {
          entry = { kind, test, runs: [], origin: `${relativeTest} run "${name}" ${kind}[${index}]`, label: `run "${name}" ${kind}[${index}]` };
          seen.set(test, entry);
        }
        entry.runs.push(name);
      });
    }
  }
  const counters = { valid: 0, invalid: 0 };
  for (const entry of seen.values()) {
    const babel = entry.runs.some(name => name.includes("babel"));
    const typescript = entry.runs.some(name => name.includes("@typescript-eslint"));
    const errors = entry.kind === "valid" ? 0 : typeof entry.test.errors === "number" ? entry.test.errors : entry.test.errors.length;
    const index = counters[entry.kind]++;
    cases.push({
      rule,
      directory: rule.slice("react-hooks/".length),
      kind: entry.kind,
      index,
      test: entry.test,
      code: entry.test.code,
      errors,
      origin: entry.origin,
      upstreamTest: `${TEST_DIRECTORY}/${file}`,
      label: entry.label,
      filename: entry.test.filename,
      ...(babel ? { extension: "jsx" } : typescript ? { extension: "tsx" } : { drop: "Flow syntax (hermes-eslint runs only)" }),
    });
  }
}

const result = await finishFixtures(require, TOOL, DIRECTORY, cases, tools);
writeLicense(TOOL, upstream, copied, result.kept);
const testFiles = checkoutFiles(upstream, UPSTREAM[TOOL].tests);
const read = TESTS.map(({ file }) => `${TEST_DIRECTORY}/${file}`).sort();
if (testFiles.join() !== read.join()) throw new Error(`upstream's ESLint rule tests are ${testFiles.join(", ")}; this recorder reads ${read.join(", ")}`);
recordSource(TOOL, DIRECTORY, tools, upstream, runs, result.unscored);
if (!argv.includes("--no-corpora")) {
  const summary = await recordCorpora(require, TOOL, DIRECTORY, corpora);
  for (const entry of summary) console.log(`corpus ${entry.corpus}: ${entry.files} files, ${entry.skipped} skipped, ${entry.findings} findings`);
}
