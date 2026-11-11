/**
 * next-no-assign-module-variable must report `module` declared in the head of a for…of or for…in loop (#263).
 *
 * @next/eslint-plugin-next 16.3.8 visits every VariableDeclaration with a declarator named `module`. In ESTree a
 * for…of/for…in head such as `const module` is a VariableDeclaration, so upstream reports it at the declaration keyword.
 * tree-sitter has no declaration node there: the keyword is the `kind` field of the for_in_statement and the name its
 * `left` field. The port reports that keyword. A destructured `module` and a bare `for (module of …)` assign no
 * VariableDeclaration named `module`, and upstream does not report them.
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
const RULE_ID = "next-no-assign-module-variable";
const document = await readFile(repositoryPath("packs", "next", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The sorted 1-based line:column positions the rule reports in `source`, as ESLint prints them. */
function reported(source: string, language = "javascript", path = "lib/load.js"): string[] {
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

describe(RULE_ID, () => {
  it("reports module declared in a for…of or for…in head at its keyword, as upstream does (issue repro)", () => {
    const source = lines(
      "for (const module of mods) {}",
      "for (let module of mods) {}",
      "for (var module in obj) {}",
      "let module = 1;",
    );
    expect(reported(source)).toEqual(["1:6", "2:6", "3:6", "4:1"]);
  });

  it("reports a for…of head in TypeScript and an async for await…of head", () => {
    const source = lines("async function f(mods: string[]) {", "  for await (const module of mods) {}", "}");
    expect(reported(source, "typescript", "lib/load.ts")).toEqual(["2:14"]);
  });

  it("reports a classic for loop's declaration, which was already reported", () => {
    expect(reported(lines("for (let module = 0; module < 1; module++) {}"))).toEqual(["1:6"]);
  });

  it("does not report a destructured module, a bare assignment target or another name in a loop head", () => {
    const source = lines(
      "for (const { module } of mods) {}",
      "for (const [module] of mods) {}",
      "for (module of mods) {}",
      "for (const mod of mods) { module.exports = mod; }",
    );
    expect(reported(source)).toEqual([]);
  });
});
