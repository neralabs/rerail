/**
 * typescript-eslint-no-invalid-void-type must not report `void` as a call's type argument while
 * allowInGenericTypeArguments is true (#246).
 *
 * typescript-eslint 8.70.1 lists CallExpression among the grandparents where `void` under a TSTypeParameterInstantiation
 * stays invalid, so `Promise.withResolvers<void>()` and `expectTypeOf(x).toEqualTypeOf<void>()` are reported while
 * `new Promise<void>()` is not. A call's type argument is a generic type argument like any other, and `void` there is
 * how TypeScript spells a void-resolving promise or a type test's void return, so the port allows it. Every other place
 * the rule reports keeps its answer: a call's type argument nested in a union or array, the array form of the option
 * (only the listed generic types), allowInGenericTypeArguments false, and `void` outside type arguments.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "typescript-eslint-no-invalid-void-type";
const RULE_PATH = repositoryPath("packs", "typescript", "rules", `${RULE_ID}.json`);

const documentSchema = z.object({ clauses: z.array(z.object({ id: z.string(), define: z.record(z.string(), z.unknown()).optional() }).passthrough()) }).passthrough();
const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

/** The rule compiled with `config` as its options' values (the defaults when empty). */
async function compile(config: Record<string, unknown> = {}): Promise<NativeCompiledCodeRuleDsl> {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = documentSchema.parse(JSON.parse(await readFile(RULE_PATH, "utf8")));
  const options = document.clauses.find(clause => clause.id === "options");
  if (options?.define === undefined) throw new Error(`${RULE_ID} has no options define`);
  options.define.config = config;
  return runtime.compile(JSON.stringify(document), "json", capabilities);
}

/** The sorted 1-based line:column positions the rule reports in `source`, as ESLint prints them. */
function reported(rule: NativeCompiledCodeRuleDsl, source: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path: "src/defer.ts", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => [violation.span.startLine + 1, violation.span.startColumn + 1] as const)
    .sort((a, b) => a[0] - b[0] || a[1] - b[1])
    .map(([line, column]) => `${line}:${column}`);
}

const lines = (...source: string[]) => [...source, ""].join("\n");

const CALLS = lines(
  "const submission = Promise.withResolvers<void>();",
  "expectTypeOf(fn()).toEqualTypeOf<void>();",
  "functionGeneric<void>(undefined);",
  "emitter?.emit<string, void>('done');",
  "const p = new Promise<void>(r => r());",
);

describe(RULE_ID, () => {
  it("does not report void as a call's type argument at the default options (issue repro)", async () => {
    expect(reported(await compile(), CALLS)).toEqual([]);
  });

  it("does not report void as a call's type argument with allowAsThisParameter on", async () => {
    expect(reported(await compile({ allowAsThisParameter: true }), CALLS)).toEqual([]);
  });

  it("does not report void as a tagged template's type argument, which upstream does not report either", async () => {
    expect(reported(await compile(), lines("tag<void>`x`;"))).toEqual([]);
  });

  it("still reports void nested in a union or an array inside a call's type arguments", async () => {
    const source = lines("f<string | void>();", "g<void[]>();");
    expect(reported(await compile(), source)).toEqual(["1:12", "2:3"]);
  });

  it("still reports void outside return types and type arguments", async () => {
    const source = lines("let a: void;", "function f(x: void): void {}", "type U = string | void;", "let m: Map<string, void>;");
    expect(reported(await compile(), source)).toEqual(["1:8", "2:15", "3:19"]);
  });

  it("still reports a call's void type argument when allowInGenericTypeArguments is false", async () => {
    expect(reported(await compile({ allowInGenericTypeArguments: false }), CALLS)).toEqual(["1:42", "2:34", "3:17", "4:23", "5:23"]);
  });

  it("still reports a call's void type argument under the array form, which allows only the listed generic types", async () => {
    expect(reported(await compile({ "allowInGenericTypeArguments.list": ["Promise"] }), CALLS)).toEqual([
      "1:42",
      "2:34",
      "3:17",
      "4:23",
      "5:23",
    ]);
  });
});
