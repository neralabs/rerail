/**
 * typescript-eslint-no-namespace (packs/typescript/) must not report a global augmentation. Inside
 * `declare module 'x' { … }`, TypeScript accepts `global { … }` without `declare`, and the patched grammar reads it as
 * an `internal_module` named `global` (Patch 26 in `native/grammars/tree-sitter-typescript/PATCHES.md`).
 * typescript-eslint's selector, `TSModuleDeclaration[global!=true][id.type!='Literal']`, never reports one, so only an
 * `internal_module` with a `namespace` keyword is a namespace to the port.
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
const DOCUMENT = repositoryPath("packs", "typescript", "rules", "typescript-eslint-no-namespace.json");

const evaluationSchema = z.object({
  violations: z.array(z.object({ span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

async function compile(): Promise<NativeCompiledCodeRuleDsl> {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  return runtime.compile(await readFile(DOCUMENT, "utf8"), "json", capabilities);
}

function reportedLines(compiled: NativeCompiledCodeRuleDsl, source: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path: "src/augment.ts", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1);
}

describe("typescript-eslint-no-namespace and global augmentations", () => {
  it.each([
    ["inside declare module", "declare module 'x' {\n  global {\n    interface String {}\n  }\n}\n"],
    ["with declare", "declare global {\n  interface String {}\n}\n"],
  ])("does not report a global augmentation %s", async (_name, source) => {
    expect(reportedLines(await compile(), source)).toEqual([]);
  });

  it("still reports a namespace, one named global included, inside declare module or not", async () => {
    const source = "namespace global {}\ndeclare module 'x' {\n  namespace inner {}\n  global {}\n}\nmodule foo {}\n";
    expect(reportedLines(await compile(), source)).toEqual([1, 3, 6]);
  });
});
