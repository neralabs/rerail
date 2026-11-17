/**
 * typescript-eslint-no-invalid-this (packs/typescript/) on a `function` passed as a call or `new` argument. Upstream
 * reports `this` there, as ESLint's isDefaultThisBinding treats the callee as calling it without a receiver, but
 * Mocha (tests, hooks), Cypress (commands, queries) and many libraries (nock replies, mongoose query helpers) bind
 * `this` for such callbacks. The port reports the same places with the same code; there its message says to keep the
 * `function` and declare `this` as its first parameter (which upstream and the port accept), or else pass the value in
 * as a parameter, instead of saying that `this` is undefined. Every other report keeps the rule's general message, and
 * a callback that declares `this` is not reported.
 *
 * Each case is a whole file; the test asserts the reported lines (1-based) and which message each carries.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../../src/engine/native/native.js";
import { readRuleCatalog } from "../../../../../src/engine/catalog/rule-catalog.js";
import { configureBundledCodeRule } from "../../../../../src/engine/catalog/rule-config.js";
import type { RuleConfig } from "../../../../../src/engine/catalog/repository-model.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "typescript-eslint-no-invalid-this";
const CODE = "@typescript-eslint/no-invalid-this";
const catalog = readRuleCatalog([repositoryPath("packs", "typescript")]);

const evaluationSchema = z.object({
  status: z.string(),
  violations: z.array(z.object({ code: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(config: RuleConfig = {}): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const configured = configureBundledCodeRule(catalog, RULE_ID, config);
  if (!configured.ok) throw new Error(configured.reason);
  return runtime.compile(JSON.stringify(configured.rule), "json", capabilities);
}

const rule = compile();

type Steer = "callback" | "general";

/** What the rule reports in a TypeScript module at `path`: the line (1-based) and which message, sorted by line. */
function reported(source: string, compiled = rule, path = "test/toolbox.test.ts"): [number, Steer][] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path, source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.status).toBe("completed");
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => {
      expect(violation.code).toBe(CODE);
      return [violation.span.startLine + 1, steer(violation.message)] as [number, Steer];
    })
    .sort((a, b) => a[0] - b[0]);
}

/** The callback message keeps the `function`, names the `this` parameter and never calls `this` undefined outright. */
function steer(message: string): Steer {
  if (message.includes("this: ") && message.includes("arrow function") && !message.includes("which is undefined here")) return "callback";
  expect(message).toBe("Pass the value in as a parameter instead of using `this`, which is undefined here.");
  return "general";
}

/** A `function` passed as an argument: reported where upstream reports it, with the callback message. */
const CALLBACKS: ReadonlyArray<[string, string, [number, Steer][]]> = [
  [
    "Mocha suite, test and hook (the issue's repro)",
    [
      "suite('toolbox', function () {",
      "  test('x', function () { this.toolbox.render(); });",
      "});",
      "before(async function () { if (cond) this.skip(); });",
      "",
    ].join("\n"),
    [[2, "callback"], [4, "callback"]],
  ],
  [
    "Cypress query (the issue's repro)",
    [
      "Commands.addQuery('contains', function contains(filter: string, text: string) {",
      "  return { type: this.hasPreviouslyLinkedCommand() ? 'child' : 'parent' };",
      "});",
      "",
    ].join("\n"),
    [[2, "callback"]],
  ],
  [
    "the same shape under unrelated names",
    ["register('k', function () { return this.value; });", "scope.on('x', async function* () { yield this.next; });", ""].join("\n"),
    [[1, "callback"], [2, "callback"]],
  ],
  [
    "a `new` argument",
    ["new SyntheticModule(['x'], function () {", "  this.setExport('x', 1);", "});", ""].join("\n"),
    [[2, "callback"]],
  ],
  [
    "a nested arrow function inside the callback",
    ["it('x', function () {", "  [1].forEach(() => this.skip());", "});", ""].join("\n"),
    [[2, "callback"]],
  ],
  ["`map` without `thisArg` inside a method", "class C { m() { return [1].map(function () { return this; }); } }\n", [[1, "callback"]]],
];

/** Neighbours that keep their answer: the general message where `this` is not in a callback argument, silence where valid. */
const NEIGHBOURS: ReadonlyArray<[string, string, [number, Steer][]]> = [
  ["a function declaration", "function plain() { return this; }\n", [[1, "general"]]],
  ["module top level", "export const x = this;\n", [[1, "general"]]],
  ["a function expression assigned to a variable", "const f = function () { return this; };\n", [[1, "general"]]],
  ["a function returned from an arrow inside a call argument", "run(() => { function g() { return this; } return g; });\n", [[1, "general"]]],
  ["a callback that declares `this`", "test('x', function (this: Mocha.Context) { this.skip(); });\n", []],
  ["an arrow function callback (lexical `this` at the top level)", "test('x', () => this.skip());\n", [[1, "general"]]],
  ["a callback with `thisArg`", "[1].map(function () { return this.y; }, ctx);\n", []],
  ["a capitalised function passed as an argument (a constructor)", "wrap(function Foo() { this.a = 1; });\n", []],
];

describe(RULE_ID, () => {
  it.each(CALLBACKS)("steers %s to declare `this`", (_name, source, expected) => {
    expect(reported(source)).toEqual(expected);
  });

  it.each(NEIGHBOURS)("keeps its answer on %s", (_name, source, expected) => {
    expect(reported(source)).toEqual(expected);
  });

  it("with capIsConstructor off, steers a capitalised callback and keeps the general message elsewhere", () => {
    const source = "wrap(function Foo() { this.a = 1; });\nfunction Bar() { this.b = 1; }\n";
    expect(reported(source, compile({ capIsConstructor: false }))).toEqual([[1, "callback"], [2, "general"]]);
  });
});
