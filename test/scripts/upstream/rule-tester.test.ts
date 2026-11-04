/**
 * The static RuleTester case diff the nightly report gives for an ESLint-family test file (scripts/upstream/rule-tester.ts).
 * Two real release pairs are recorded in fixtures/host/upstream-watch/: ESLint's no-loss-of-precision test at v10.11.0 and
 * v10.12.0 (10 cases added), and typescript-eslint's no-extraneous-class test at v8.71.2 and v8.72.0 (2 added, and 2
 * edited in place). Both counts are what a full clone's analysis found for those releases.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "vitest";

import { compareCases, extractCases, type TestCase } from "../../../scripts/upstream/rule-tester.js";
import { fixturesPath } from "../../support/oracles.js";

const RECORDED = fixturesPath("host", "upstream-watch");

/** The cases of a recorded test file. */
function recordedCases(name: string, rule: string): TestCase[] {
  return extractCases(readFileSync(join(RECORDED, name), "utf8"), name, rule);
}

test("ESLint v10.11.0 → v10.12.0 adds 10 no-loss-of-precision cases", () => {
  const before = recordedCases("eslint-no-loss-of-precision.v10.11.0.js.txt", "no-loss-of-precision");
  const after = recordedCases("eslint-no-loss-of-precision.v10.12.0.js.txt", "no-loss-of-precision");
  assert.deepEqual([before.length, after.length], [131, 141]);
  assert.ok(before.every((entry) => !entry.unresolved));
  assert.deepEqual(compareCases(before, after), { added: 10, removed: 0, changed: 0, unresolved: 0 });
});

test("typescript-eslint v8.71.2 → v8.72.0 splits a no-extraneous-class case and adds another: 2 added, 2 changed", () => {
  const before = recordedCases("typescript-eslint-no-extraneous-class.v8.71.2.ts.txt", "no-extraneous-class");
  const after = recordedCases("typescript-eslint-no-extraneous-class.v8.72.0.ts.txt", "no-extraneous-class");
  assert.deepEqual([before.length, after.length], [28, 30]);
  assert.deepEqual(compareCases(before, after), { added: 2, removed: 0, changed: 2, unresolved: 0 });
});

test("cases are read through bindings, helpers, spreads and tags, and the generators' filters apply", () => {
  const source = `
const { RuleTester } = require("../../../lib/rule-tester");
const ruleTester = new RuleTester();
const prefix = "var ";
const wrap = (code) => \`(\${code})\`;
const shared = ["a;", "b;"];
ruleTester.run("no-demo", rule, {
  valid: [
    prefix + "x;",
    wrap("y"),
    ...shared,
    String.raw\`\\d\`,
    { code: "skipped;", skip: true },
    { code: "typed;", languageOptions: { parserOptions: { project: "./tsconfig.json" } } },
    { code: "vue;", languageOptions: { parser: require("vue-eslint-parser") } },
    { code: "ts;", languageOptions: { parser: require("@typescript-eslint/parser") } },
    { code: "x".repeat(70000) },
    { code: computed() },
  ],
  invalid: [{ code: ["c", "d"].join(";"), errors: 1 }],
});
ruleTester.run("helper-rule", other, { valid: ["ignored;"], invalid: [] });
`;
  const cases = extractCases(source, "tests/lib/rules/no-demo.js", "no-demo");
  assert.deepEqual(cases.map(({ kind, key }) => `${kind} ${key}`), [
    "valid var x;",
    "valid (y)",
    "valid a;",
    "valid b;",
    "valid \\d",
    "valid ts;",
    "valid UNRESOLVED:computed()",
    "invalid c;d",
  ]);
});

test("a file without RuleTester runs is one unresolved case, so an edit to it still shows", () => {
  const [before, after] = ["test.snapshot({ valid: [a] });", "test.snapshot({ valid: [a, b] });"].map((source) => extractCases(source, "test/no-null.js", "no-null"));
  assert.deepEqual(compareCases(before!, after!), { added: 0, removed: 0, changed: 1, unresolved: 2 });
});

test("a reorder is no change; a removal not followed by an addition is a removal", () => {
  const cases = (keys: string[]) => keys.map((key) => ({ kind: "valid" as const, key, unresolved: false }));
  assert.deepEqual(compareCases(cases(["a", "b", "c"]), cases(["c", "a", "b"])), { added: 0, removed: 0, changed: 0, unresolved: 0 });
  assert.deepEqual(compareCases(cases(["a", "b", "c"]), cases(["a", "c", "d"])), { added: 1, removed: 1, changed: 0, unresolved: 0 });
  assert.deepEqual(compareCases(cases(["a", "b", "c"]), cases(["a", "B", "c"])), { added: 0, removed: 0, changed: 1, unresolved: 0 });
});
