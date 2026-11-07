/** F632 must suggest value comparison without reversing `is not`, including inside string annotations. */
import { readFile } from "node:fs/promises";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { readRuleDocument } from "../../../../src/engine/catalog/rule-documents.js";
import { loadNativeBinding } from "../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../support/oracles.js";

const capabilities = await readFile(repositoryPath("native", "src", "code_rule_dsl", "capabilities.json"), "utf8");
const document = readRuleDocument(repositoryPath("packs", "python", "rules", "ruff-f632-is-literal.json"));
const runtime = loadNativeBinding().CodeRuleDslRuntime;
if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const compiled = runtime.compile(JSON.stringify(document.rule), "json", capabilities);
const evaluationSchema = z.object({
  violations: z.array(z.object({ message: z.string() })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function messages(source: string): string[] {
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "src/status.py", source: `${source}\n` }],
  }), "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.message);
}

describe("F632 steer preserves comparison polarity", () => {
  it.each([
    ['value is "ready"', "=="],
    ['"ready" is value', "=="],
    ['value is not "ready"', "!="],
    ['"ready" is not value', "!="],
    ["value is not (1, 2)", "!="],
    ['def f(value: "x is not 1"): ...', "!="],
    ['def f(value: "x is 1"): ...', "=="],
  ])("gives the correct replacement for %s", (source, replacement) => {
    const findings = messages(source);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toContain(`\`${replacement}\``);
  });

  it("keeps operators distinct when a chain has both polarities", () => {
    const findings = messages("x is 1 is not y");
    expect(findings).toHaveLength(2);
    expect(findings.filter(message => message.includes("`==`"))).toHaveLength(1);
    expect(findings.filter(message => message.includes("`!=`"))).toHaveLength(1);
  });

  it.each(['value == "ready"', 'value != "ready"', "value is None", 'value is not "ready"  # noqa: F632'])(
    "keeps existing non-findings for %s", source => {
      expect(messages(source)).toEqual([]);
    },
  );
});
