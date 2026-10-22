import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { readRuleDraft, testRuleDraft } from "../../../src/cli/rule-draft.js";
import { ConversationRuleRegistry } from "../../../src/engine/catalog/registry.js";
import { fixturesPath } from "../../support/oracles.js";

const draftPath = fixturesPath("host", "task-completion", "rule-draft.json");

describe("unfinished-task observation rule", () => {
  it("catches admitted unfinished work and passes completion, blocker, and boundary cases", async () => {
    const report = await testRuleDraft(readRuleDraft(draftPath), process.cwd());
    expect(report.valid).toBe(true);
    expect(report.ok).toBe(true);
    expect(report.cases).toHaveLength(11);
  });

  it("keeps matches in Log mode and respects delivery once per session", () => {
    const draft = readRuleDraft(draftPath);
    if (draft.kind !== "conversation") throw new Error("Expected a conversation rule");
    const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
    registry.replace([draft.rule]);
    const sample = { source: "assistant-text" as const, boundary: "turn" as const, text: "I have not completed the task." };
    const matches = registry.match(sample, { turn: 1 });
    expect(matches).toMatchObject([{ ruleId: "local:check-unfinished-task", mode: "log" }]);
    registry.markDelivered(matches.map(match => match.ruleId), 1);
    expect(registry.match(sample, { turn: 2 })).toEqual([]);
  });
});
