/**
 * typescript-eslint-no-unused-expressions (packs/typescript/) must not flag a type test written as an `as` or
 * `satisfies` expression statement whose type is a dotted name with two or more dots (NER-1778). tree-sitter reads
 * `e satisfies A.B.C` as a member access `.C` on `e satisfies A.B`, and a member access is an unused expression, so the
 * rule reported 39 `z.x() satisfies z.core.$ZodX;` type tests in one zod file and steered agents to delete them.
 * Upstream typescript-eslint 8.70.1 never reports a `satisfies` statement and judges an `as` statement by the
 * expression it wraps.
 *
 * Each case is one statement on line 1; the test asserts exactly which lines the shipped rule reports.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { loadBundledRules } from "../../../../src/engine/catalog/bundled-rule-documents.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "typescript-eslint-no-unused-expressions";

const evaluationSchema = z.object({
  status: z.string(),
  violations: z.array(z.object({ path: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  // A candidate since the rule gates (NER-1900): it fails engine-reads-goldens on vscode files the engine cannot fully read.
  const document = loadBundledRules([repositoryPath("packs", "typescript", "candidates")]).find(candidate => candidate.rule.id === RULE_ID);
  if (document === undefined) throw new Error(`${RULE_ID} is not in packs/typescript/candidates`);
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

/** The 1-based lines the rule reports in `source`. */
function reportedLines(compiled: NativeCompiledCodeRuleDsl, source: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path: "src/types.test.ts", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.status).toBe("completed");
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

/** Type tests upstream does not report: a `satisfies` statement, or an `as` around a call. */
const NOT_REPORTED: ReadonlyArray<[string, string]> = [
  ["zod's satisfies type test (ticket repro)", "z.currencyCode() satisfies z.core.$ZodStringFormatTypes;"],
  ["a call `as` a two-dot type", "x() as A.B.C;"],
  ["an identifier satisfying a two-dot type", "x satisfies A.B.C;"],
  ["a call satisfying a one-dot type", "x() satisfies A.B;"],
  ["a call satisfying a three-dot type", "x() satisfies A.B.C.D;"],
  ["a call `as` a three-dot type", "x() as A.B.C.D;"],
  ["an assignment `as` a two-dot type", "(a = b) as A.B.C;"],
  ["a call on a parenthesized `as`", "(x as A.B.C).run();"],
];

/** Statements upstream still reports: an `as` around a non-call, and plain member accesses. */
const REPORTED: ReadonlyArray<[string, string]> = [
  ["an identifier `as` a two-dot type", "x as A.B.C;"],
  ["an identifier `as` a one-dot type", "x as A.B;"],
  ["an identifier `as` a three-dot type", "x as A.B.C.D;"],
  ["a member access `as` a two-dot type", "a.b as A.B.C;"],
  ["a plain member access", "a.b.c;"],
  ["a member access on a parenthesized `as`", "(x as A.B).c;"],
  ["a member access on a parenthesized `satisfies`", "(x satisfies A.B).c;"],
];

describe(`${RULE_ID} with dotted type names (NER-1778)`, () => {
  const compiled = compile();

  it.each(NOT_REPORTED)("does not report %s", (_name, source) => {
    expect(reportedLines(compiled, `${source}\n`)).toEqual([]);
  });

  it.each(REPORTED)("reports %s", (_name, source) => {
    expect(reportedLines(compiled, `${source}\n`)).toEqual([1]);
  });

  it("reports only the unused expressions in a mixed zod-style file", () => {
    const source = [
      "z.string() satisfies z.core.$ZodType;",
      "value as z.core.$ZodType;",
      "z.number() as z.core.$ZodNumber;",
      "schema.shape;",
      "z.email() satisfies z.core.$ZodStringFormatTypes;",
      "",
    ].join("\n");
    expect(reportedLines(compiled, source)).toEqual([2, 4]);
  });
});
