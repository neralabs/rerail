/** E711 guidance must preserve the condition's polarity, including reversed and parenthesized operands. */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { readRuleDocument } from "../../../../src/engine/catalog/rule-documents.js";
import { loadNativeBinding } from "../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../support/oracles.js";

const capabilities = await readFile(repositoryPath("native", "src", "code_rule_dsl", "capabilities.json"), "utf8");
const document = readRuleDocument(join(repositoryPath("packs", "python", "rules"), "ruff-e711-none-comparison.json"));
const runtime = loadNativeBinding().CodeRuleDslRuntime;
if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const compiled = runtime.compile(JSON.stringify(document.rule), "json", capabilities);
const evaluationSchema = z.object({
  violations: z.array(z.object({ message: z.string() })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function messages(expression: string): string[] {
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "src/presence.py", source: `result = ${expression}\n` }],
  }), "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.message);
}

describe("E711 steer preserves comparison polarity", () => {
  it.each([
    ["value == None", "is None"],
    ["None == value", "is None"],
    ["(None) == (value)", "is None"],
    ["value != None", "is not None"],
    ["None != value", "is not None"],
    ["((None)) != (value)", "is not None"],
  ])("gives the correct replacement for %s", (expression, replacement) => {
    const findings = messages(expression);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toContain(replacement);
  });

  it("keeps the operators distinct in a chained comparison", () => {
    const findings = messages("None == value != None");
    expect(findings).toHaveLength(2);
    expect(findings.filter(message => message.includes("is not None"))).toHaveLength(1);
    expect(findings.filter(message => message.includes("is None"))).toHaveLength(1);
  });

  it.each(["value is None", "value is not None", "value != None  # noqa: E711", "1 == None"])(
    "keeps existing non-findings for %s", expression => {
      expect(messages(expression)).toEqual([]);
    },
  );
});
