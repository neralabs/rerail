import { describe, expect, it } from "vitest";
import { parseGeneratedScenarios } from "../src/generation-contract.js";

const rule = {
  id: "logging",
  kind: "file" as const,
  definition: "{}",
  digest: "saved-rule",
  updatedAt: "1",
  source: "library" as const,
};

function generatedPair() {
  return (["match", "no-match"] as const).map((expectation) => ({
    id: expectation,
    title: expectation,
    expectation,
    explanation: "Inspect contrasting source fixtures.",
    prompt: "Read source.ts and describe its logging.",
    fixtures: [
      { path: "source.ts", content: expectation === "match" ? 'console.log("hello");' : 'logger.info("hello");' },
    ],
    steps: [
      { text: "Inspecting the source.", toolCalls: [{ name: "read", arguments: { path: "source.ts" } }] },
      { text: expectation === "match" ? "The source uses console logging." : "The source uses the project logger." },
    ],
  }));
}

describe("generated replay steps", () => {
  it("accepts text-only steps in both scenarios without dropping declared tool calls", () => {
    const scenarios = parseGeneratedScenarios(generatedPair(), rule);
    for (const scenario of scenarios) {
      expect(scenario.steps.map((step) => step.toolCalls)).toEqual([
        [{ name: "read", arguments: { path: "source.ts" } }],
        [],
      ]);
    }
  });

  it("rejects a malformed tool-call list instead of silently dropping it", () => {
    const pair = generatedPair();
    const malformed = pair.map((scenario) => ({
      ...scenario,
      steps: [{ text: "Inspecting the source.", toolCalls: { name: "read", arguments: { path: "source.ts" } } }],
    }));
    expect(() => parseGeneratedScenarios(malformed, rule)).toThrow();
  });
});
