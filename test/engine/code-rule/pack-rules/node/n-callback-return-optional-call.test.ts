/**
 * n-callback-return must report an optional callback call `cb?.()` that is a function's last statement or the statement
 * just before a block's final return.
 *
 * eslint-plugin-n 18.4.1 exempts those two positions only when the ExpressionStatement's expression is the call itself
 * (or the right side of a binary or logical expression). ESTree wraps an optional call in a ChainExpression, so the
 * exemptions never apply to `cb?.()` and upstream reports it there. A plain `cb()` in the same positions, and an optional
 * call inside a return or an arrow function's expression body, stay exempt. Every expectation below is the position
 * eslint-plugin-n 18.4.1 reports on the same source.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "n-callback-return";
const document = await readFile(repositoryPath("packs", "node", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The sorted 1-based line:column positions the rule reports in `source`, as ESLint prints them. */
function reported(source: string, language: string, path: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language, path, source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => [violation.span.startLine + 1, violation.span.startColumn + 1] as const)
    .sort((a, b) => a[0] - b[0] || a[1] - b[1])
    .map(([line, column]) => `${line}:${column}`);
}

const lines = (...source: string[]) => [...source, ""].join("\n");

const GRAMMARS = [
  ["javascript", "lib/callbacks.js"],
  ["typescript", "lib/callbacks.ts"],
] as const;

describe.each(GRAMMARS)(`${RULE_ID} (%s)`, (language, path) => {
  it("reports an optional call that is the last statement or just before the final return (issue repro)", () => {
    const source = lines(
      "function a(cb) { cb?.(); return true; }",
      "function c(cb) { doIt(); cb?.(); }",
      "const e = (cb) => { cb?.(); };",
      "function b(cb) { cb(); return true; }",
    );
    expect(reported(source, language, path)).toEqual(["1:18", "2:26", "3:21"]);
  });

  it("reports an optional call on the right of a logical expression, in a nested block and in a method", () => {
    const source = lines(
      "function f(cb, x) { x && cb?.(); }",
      "function g(cb, x) { if (x) { cb?.(); return; } other(); }",
      "class K { m(callback) { doIt(); callback?.(null, 1); } }",
      "function r(next, x) { x && next?.(); return 1; }",
    );
    expect(reported(source, language, path)).toEqual(["1:26", "2:30", "3:33", "4:28"]);
  });

  it("keeps the exemptions for a plain call, and for an optional call in a return or an arrow's expression body", () => {
    const source = lines(
      "function b(cb) { cb(); return true; }",
      "function k(cb) { doIt(); cb(); }",
      "function l(cb, x) { x && cb(); }",
      "function h(cb) { return cb?.(); }",
      "const i = (cb) => cb?.();",
      "function q(cb) { cb?.call(this); }",
    );
    expect(reported(source, language, path)).toEqual([]);
  });

  it("still reports calls that were already reported", () => {
    const source = lines("function j(cb) { cb?.(); other(); }", "function p(cb, x) { if (x) { cb(); } }");
    expect(reported(source, language, path)).toEqual(["1:18", "2:30"]);
  });
});
