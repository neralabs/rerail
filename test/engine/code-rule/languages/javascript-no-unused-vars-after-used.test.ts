/**
 * eslint-no-unused-vars (packs/javascript/candidates/) must apply ESLint's default `args: "after-used"`.
 * With after-used, ESLint does not report a parameter that comes before the last parameter the function uses: removing
 * it would shift the next argument into its slot. The port reported those leading parameters, and an agent told to
 * remove one breaks every call site. Parameters after the last used one are still reported, as is the unused function.
 *
 * Each case asserts exactly which names the candidate rule reports, as ESLint 10.11.0 does with its defaults.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_PATH = repositoryPath("packs", "javascript", "candidates", "rules", "eslint-no-unused-vars.json");

const evaluationSchema = z.object({
  violations: z.array(z.object({ path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

async function compile(): Promise<NativeCompiledCodeRuleDsl> {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  return runtime.compile(await readFile(RULE_PATH, "utf8"), "json", capabilities);
}

/** The names the rule reports in `source` (each message quotes its name), sorted. */
function reported(compiled: NativeCompiledCodeRuleDsl, source: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "javascript", path: "src/args.js", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => /'([^']+)'/.exec(violation.message)?.[1] ?? violation.message).sort();
}

const CASES: ReadonlyArray<[string, string, string[]]> = [
  ["an arrow function that uses only its second parameter", "const f = (cid, cm) => cm.on();\n", ["f"]],
  ["a function declaration that uses only its second parameter", "function g(a, b) { return b; }\n", ["g"]],
  ["a function expression that uses only its last parameter", "const k = function (a, b, c) { return c; };\n", ["k"]],
  ["a method that uses only its second parameter", "export class C { m(a, b) { return b; } }\n", []],
  ["a callback that uses only its second parameter", "export const xs = [1].map((x, i) => i);\n", []],
  ["a parameter after the last used one", "function h(a, b) { return a; }\n", ["b", "h"]],
  ["parameters on both sides of the last used one", "export function m(a, b, c) { return b; }\n", ["c"]],
  ["an arrow function with one unused parameter", "export const u = (a) => 1;\n", ["a"]],
  ["an unused parameter after a used destructured one", "export function d({ x }, y) { return x; }\n", ["y"]],
  ["a defaulted parameter before a used one (ESLint exempts only a plain identifier)", "export function e(a = 1, b) { return b; }\n", ["a"]],
  ["a parameter before a used destructured one", "export function p(a, { x }) { return x; }\n", []],
  ["a parameter before a used rest parameter", "export function q(a, ...xs) { return xs; }\n", []],
];

describe("eslint-no-unused-vars args after-used", async () => {
  const compiled = await compile();
  it.each(CASES)("%s", (_name, source, expected) => {
    expect(reported(compiled, source)).toEqual(expected);
  });
});
