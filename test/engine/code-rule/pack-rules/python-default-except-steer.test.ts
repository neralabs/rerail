/** F707 must tell the agent how to fix a misplaced bare handler, rather than only reporting it (NER-2036). */
import { readFile } from "node:fs/promises";
import { z } from "zod";
import { expect, it } from "vitest";
import { readRuleDocument } from "../../../../src/engine/catalog/rule-documents.js";
import { loadNativeBinding } from "../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../support/oracles.js";

const document = readRuleDocument(repositoryPath("packs", "python", "rules", "ruff-f707-default-except-not-last.json"));
const capabilities = await readFile(repositoryPath("native", "src", "code_rule_dsl", "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(JSON.stringify(document.rule), "json", capabilities);
const evaluationSchema = z.object({
  violations: z.array(z.object({ message: z.string() })).default([]),
  failures: z.array(z.unknown()).default([]),
});

it("explains the required order of bare and typed handlers", () => {
  const source = "try:\n    run()\nexcept:\n    recover()\nexcept ValueError:\n    fallback()\n";
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "src/handlers.py", source }],
  }), "report")));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(1);
  expect(result.violations[0]?.message).toMatch(/move.*bare.*except.*after/i);
  expect(result.violations[0]?.message).not.toContain("as not the last");
});
