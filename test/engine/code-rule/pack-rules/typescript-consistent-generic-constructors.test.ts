/**
 * typescript-eslint-consistent-generic-constructors (packs/typescript/ candidate) must not steer an agent into breaking
 * a project that compiles with `isolatedDeclarations`. With that compiler option an exported declaration
 * needs an explicit type annotation, so `export const s: Map<string, X> = new Map();` is required, and the rule's fix
 * (`export const s = new Map<string, X>();`) fails tsc with TS9010. typescript-eslint skips this report when the
 * `isolatedDeclarations` parser option is set; Rerail has no such fact yet, so the steer says when not to follow it.
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
const DOCUMENT = repositoryPath("packs", "typescript", "candidates", "rules", "typescript-eslint-consistent-generic-constructors.json");

const evaluationSchema = z.object({
  violations: z.array(z.object({ message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

async function compile(): Promise<NativeCompiledCodeRuleDsl> {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  return runtime.compile(await readFile(DOCUMENT, "utf8"), "json", capabilities);
}

function findings(compiled: NativeCompiledCodeRuleDsl, source: string): { line: number; message: string }[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path: "src/state.ts", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => ({ line: violation.span.startLine + 1, message: violation.message }));
}

describe("typescript-eslint-consistent-generic-constructors and isolatedDeclarations", () => {
  it.each([
    ["an exported const with arguments", "export const state: Map<string, Set<Chunk>> = new Map(entries);"],
    ["an exported const without arguments", "export const state: Map<string, Set<Chunk>> = new Map;"],
    ["an exported const with empty arguments", "export const state: Map<string, Set<Chunk>> = new Map();"],
  ])("still reports %s, and tells the agent to keep the annotation under isolatedDeclarations", async (_name, source) => {
    const reported = findings(await compile(), source);
    expect(reported).toHaveLength(1);
    expect(reported[0]?.line).toBe(1);
    expect(reported[0]?.message).toMatch(/^Move the type arguments from the annotation onto the `new` call/);
    expect(reported[0]?.message).toContain("isolatedDeclarations");
  });
});
