/**
 * typescript-eslint-no-unused-expressions (packs/typescript/) on short-circuit calls. Upstream's message,
 * "Expected an assignment or function call and instead saw an expression.", gives no fix, and on rxjs two agents
 * rewrote `!seen && startTimer(…)` and `each! > 0 && startTimer(each!)` by dropping the condition, breaking the code.
 * A `&&`, `||` or `??` statement whose right operand is a call (or anything `allowShortCircuit` allows) is reported
 * at the same place with the same code, and its message says to rewrite it as an `if` that keeps the condition (with a
 * generic example: a message cuts a long or multi-line operand). With `allowShortCircuit` set, it is not reported.
 *
 * Each case is one statement starting on line 1.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { configureBundledCodeRule } from "../../../../src/engine/catalog/rule-config.js";
import type { RuleConfig } from "../../../../src/engine/catalog/repository-model.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const catalog = readRuleCatalog([repositoryPath("packs", "typescript")]);
const RULE_ID = "typescript-eslint-no-unused-expressions";
const CODE = "@typescript-eslint/no-unused-expressions";
const UPSTREAM = "Expected an assignment or function call and instead saw an expression.";

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

/** What the rule reports in `source`: line (1-based), code and message. */
function reported(compiled: NativeCompiledCodeRuleDsl, source: string): { line: number; code: string; message: string }[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path: "src/timer.ts", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.status).toBe("completed");
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => ({ line: violation.span.startLine + 1, code: violation.code, message: violation.message }))
    .sort((a, b) => a.line - b.line);
}

/** A short-circuit call: the message says to rewrite it as an `if`, names the condition to keep, and shows the rewrite. */
const SHORT_CIRCUIT: ReadonlyArray<[string, string, string]> = [
  ["`!seen && startTimer(\u2026)`, from the rxjs runs", "!seen && startTimer(delay);", "Rewrite this as an `if` that keeps the condition: `startTimer(delay)` runs only when `!seen` is truthy, so `a && f()` becomes `if (a) { f(); }`."],
  ["`each! > 0 && startTimer(each!)`, from the rxjs runs", "each! > 0 && startTimer(each!);", "Rewrite this as an `if` that keeps the condition: `startTimer(each!)` runs only when `each! > 0` is truthy, so `a && f()` becomes `if (a) { f(); }`."],
  ["an `||` call", "cache.has(key) || load(key);", "Rewrite this as an `if` that keeps the condition: `load(key)` runs only when `cache.has(key)` is falsy, so `a || f()` becomes `if (!(a)) { f(); }`."],
  ["an `||` call on a negation", "!ready || start();", "Rewrite this as an `if` that keeps the condition: `start()` runs only when `!ready` is falsy, so `a || f()` becomes `if (!(a)) { f(); }`."],
  ["a `??` call", "handler ?? fallback();", "Rewrite this as an `if` that keeps the condition: `fallback()` runs only when `handler` is null or undefined, so `a ?? f()` becomes `if (a === null || a === undefined) { f(); }`."],
  ["a chain of `&&`", "a && b && f();", "Rewrite this as an `if` that keeps the condition: `f()` runs only when `a && b` is truthy, so `a && f()` becomes `if (a) { f(); }`."],
  ["an `||` whose right side is an `&&` call", "a || b && f();", "Rewrite this as an `if` that keeps the condition: `b && f()` runs only when `a` is falsy, so `a || f()` becomes `if (!(a)) { f(); }`."],
  ["an `&&` whose right side is a parenthesized `||` call", "a && (b || f());", "Rewrite this as an `if` that keeps the condition: `(b || f())` runs only when `a` is truthy, so `a && f()` becomes `if (a) { f(); }`."],
  ["an `&&` assignment", "ok && (count = 1);", "Rewrite this as an `if` that keeps the condition: `(count = 1)` runs only when `ok` is truthy, so `a && f()` becomes `if (a) { f(); }`."],
  ["an `&&` await", "ok && await flush();", "Rewrite this as an `if` that keeps the condition: `await flush()` runs only when `ok` is truthy, so `a && f()` becomes `if (a) { f(); }`."],
  ["an `&&` with a comment", "ok && /* once */ f();", "Rewrite this as an `if` that keeps the condition: `f()` runs only when `ok` is truthy, so `a && f()` becomes `if (a) { f(); }`."],
  ["a multi-line call, cut in the message", "ok && f(\n  a,\n);", "Rewrite this as an `if` that keeps the condition: `f(\u2026` runs only when `ok` is truthy, so `a && f()` becomes `if (a) { f(); }`."],
];

/** Upstream's message, unchanged: not a short-circuit call, or one whose right operand is itself unused. */
const PLAIN: ReadonlyArray<[string, string]> = [
  ["an identifier", "value;"],
  ["an `&&` with no call", "a && b;"],
  ["an `||` with no call", "a || b;"],
  ["a ternary of calls", "a ? f() : g();"],
  ["an `&&` whose right side is an unused `&&`", "a && (b && c);"],
  ["an `&&` whose right side is an unused `&&`, with a comment", "a && (/* x */ b && c);"],
  ["a parenthesized `&&` call", "(a && f());"],
  ["a call under another operator", "a + f();"],
  ["a comparison with a call", "f() === g();"],
];

describe(`${RULE_ID} on short-circuit calls`, () => {
  const compiled = compile();

  it.each(SHORT_CIRCUIT)("tells the agent to keep the condition of %s", (_name, source, message) => {
    expect(reported(compiled, `${source}\n`)).toEqual([{ line: 1, code: CODE, message }]);
  });

  it.each(PLAIN)("keeps upstream's message for %s", (_name, source) => {
    expect(reported(compiled, `${source}\n`)).toEqual([{ line: 1, code: CODE, message: UPSTREAM }]);
  });

  it("honours an eslint-disable comment for a short-circuit call", () => {
    const source = "// eslint-disable-next-line @typescript-eslint/no-unused-expressions\n!seen && startTimer(delay);\nok && f();\n";
    expect(reported(compiled, source).map(violation => violation.line)).toEqual([3]);
  });

  describe("with allowShortCircuit", () => {
    const allowed = compile({ allowShortCircuit: true });

    it.each(SHORT_CIRCUIT)("does not report %s", (_name, source) => {
      expect(reported(allowed, `${source}\n`)).toEqual([]);
    });

    it("still reports a short-circuit with no call, and a ternary", () => {
      expect(reported(allowed, "a && b;\na ? f() : g();\n")).toEqual([
        { line: 1, code: CODE, message: UPSTREAM },
        { line: 2, code: CODE, message: UPSTREAM },
      ]);
    });
  });
});
