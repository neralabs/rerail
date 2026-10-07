import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { CosineRuleRegistry } from "../src/registry.js";
import type { CosineRule } from "../src/rule-schema.js";

function rule(overrides: Partial<CosineRule> = {}): CosineRule {
  return {
    id: "checkout",
    mode: "steer",
    reference: "Guest checkout remains available.",
    match: { operator: "gte", threshold: 0.99 },
    sample: {
      source: "assistant-text",
      boundary: "message",
      minimumTokens: 1,
    },
    action: { content: "Check checkout requirements." },
    repeat: { mode: "always" },
    ...overrides,
  };
}

describe("CosineRuleRegistry", () => {
  it("returns configured actions with native cosine match facts", () => {
    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    registry.replace([rule()]);

    const matches = registry.match({
      source: "assistant-text",
      boundary: "message",
      text: "Guest checkout remains available.",
    });

    expect(matches).toEqual([
      expect.objectContaining({
        ruleId: "checkout",
        similarity: 1,
        mode: "steer",
        action: { content: "Check checkout requirements." },
      }),
    ]);
  });

  it("keeps the active registry when replacement validation fails", () => {
    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    registry.replace([rule()]);

    expect(() =>
      registry.replace([
        rule({
          id: "invalid",
          match: { operator: "gte", threshold: 2 },
        }),
      ]),
    ).toThrow(/threshold/i);

    expect(registry.inspect().map(item => item.rule.id)).toEqual(["checkout"]);
  });

  it("applies once and after-gap repeat policies only after delivery", () => {
    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    registry.replace([
      rule({ id: "once", repeat: { mode: "once" } }),
      rule({ id: "gap", repeat: { mode: "after-gap", gapTurns: 3 } }),
    ]);
    const sample = {
      source: "assistant-text",
      boundary: "message",
      text: "Guest checkout remains available.",
    } as const;

    expect(registry.match(sample, { turn: 1 }).map(match => match.ruleId)).toEqual(["gap", "once"]);
    registry.markDelivered(["gap", "once"], 1);
    expect(registry.match(sample, { turn: 2 })).toEqual([]);
    expect(registry.match(sample, { turn: 4 }).map(match => match.ruleId)).toEqual(["gap"]);
    expect(registry.match(sample, { turn: 2, respectRepeat: false }).map(match => match.ruleId)).toEqual([
      "gap",
      "once",
    ]);
  });
});
