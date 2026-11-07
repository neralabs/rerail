/**
 * typescript-eslint-class-literal-property-style (packs/typescript/candidates/) must not steer an agent into breaking
 * a class hierarchy without warning. Like typescript-eslint, the rule reports a getter that returns a
 * literal and advises a readonly field. When a subclass overrides that getter as an accessor
 * (`override get x() { ... }`), turning the base getter into a field fails to compile with TS2611 ("'x' is defined as
 * a property in class 'Base', but is overridden here in 'Derived' as an accessor"). Subclasses usually live in other
 * files, so the rule cannot tell; it keeps reporting, as upstream does, and its message tells the agent to check for
 * overriding accessors first.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../src/engine/native/native.js";
import { repositoryPath } from "../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_PATH = repositoryPath("packs", "typescript", "candidates", "rules", "typescript-eslint-class-literal-property-style.json");

const evaluationSchema = z.object({
  violations: z.array(z.object({ message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

async function compile(): Promise<NativeCompiledCodeRuleDsl> {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  return runtime.compile(await readFile(RULE_PATH, "utf8"), "json", capabilities);
}

/** The 1-based lines the rule reports in `source`, each with its message. */
function findings(compiled: NativeCompiledCodeRuleDsl, source: string): { line: number; message: string }[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path: "src/mesh.ts", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => ({ line: violation.span.startLine + 1, message: violation.message })).sort((a, b) => a.line - b.line);
}

const ts = (...lines: string[]) => [...lines, ""].join("\n");

describe("typescript-eslint-class-literal-property-style", () => {
  it("warns that a subclass accessor override keeps the base getter from becoming a field", async () => {
    const compiled = await compile();
    const source = ts(
      "class AbstractMesh {",
      "  public get _isGaussianSplatting(): boolean { return false; }",
      "}",
      "class GaussianSplattingMeshBase extends AbstractMesh {",
      "  public override get _isGaussianSplatting(): boolean { return true; }",
      "}",
    );
    const reported = findings(compiled, source);
    expect(reported.map(finding => finding.line)).toEqual([2]);
    expect(reported[0]?.message).toMatch(/readonly field/);
    expect(reported[0]?.message).toMatch(/unless a subclass overrides it with `override get`/);
  });

  it("still reports literal getters with no override, and still skips override and setter-paired getters", async () => {
    const compiled = await compile();
    const source = ts(
      "class A {",
      "  get kind() { return 'a'; }",
      "  static get size() { return 3; }",
      "  get flag() { return true; }",
      "  set flag(value) {}",
      "  get computed() { return this.kind + 'x'; }",
      "}",
      "class B extends A {",
      "  override get kind() { return 'b'; }",
      "}",
    );
    expect(findings(compiled, source).map(finding => finding.line)).toEqual([2, 3]);
  });
});
