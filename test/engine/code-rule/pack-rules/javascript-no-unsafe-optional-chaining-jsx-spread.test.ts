/**
 * eslint-no-unsafe-optional-chaining must not report a JSX spread attribute such as `<X {...a?.b} />` (#278).
 *
 * ESLint 10.11.0 checks a SpreadElement outside an object literal; a JSX spread attribute is a JSXSpreadAttribute, which
 * it does not check, and spreading undefined into props does not throw. tree-sitter represents `{...x}` in JSX as a
 * `spread_element` inside a `jsx_expression`, so the port excludes that as well as a spread in an object literal. An
 * array spread, a call-argument spread and a spread of an optional chain nested inside a JSX attribute's value are
 * still reported, where ESLint reports them.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "eslint-no-unsafe-optional-chaining";
const document = await readFile(repositoryPath("packs", "javascript", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The sorted 1-based line:column positions the rule reports in `source`, as ESLint prints them. */
function reported(source: string, language = "javascript", path = "src/component.jsx"): string[] {
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

describe(RULE_ID, () => {
  it("reports the array and argument spreads but not the JSX spread attribute, as ESLint does (issue repro)", () => {
    const source = ["const A = (a) => <X {...a?.b} />;", "const b = [...a?.b];", "f(...a?.b);", ""].join("\n");
    expect(reported(source)).toEqual(["2:15", "3:6"]);
  });

  it("does not report a JSX spread attribute in a .tsx file", () => {
    expect(reported("export const A = (a: any) => <X {...a?.b} y={1} />;\n", "tsx", "src/component.tsx")).toEqual([]);
  });

  it("still reports an unsafe spread inside a JSX attribute's value", () => {
    expect(reported("const A = (a) => <X y={[...a?.b]} />;\n")).toEqual(["1:28"]);
  });
});
