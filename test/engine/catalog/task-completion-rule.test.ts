import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { ConversationRuleRegistry, parseConversationRule } from "../../../src/engine/index.js";
import type { NativeSample } from "../../../src/engine/native/native.js";
import { fixturesPath } from "../../support/oracles.js";

const draftPath = fixturesPath("host", "task-completion", "rule-draft.json");
const draft: { rule: unknown; cases: { caught: NativeSample[]; passed: NativeSample[] } } = JSON.parse(await readFile(draftPath, "utf8"));

function registry(): ConversationRuleRegistry {
  const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
  registry.replace([parseConversationRule(draft.rule)]);
  return registry;
}

describe("unfinished-task observation rule", () => {
  it("catches admitted unfinished work and passes completion, blocker, and boundary cases", () => {
    const rules = registry();
    for (const expected of ["caught", "passed"] as const) {
      for (const sample of draft.cases[expected]) {
        expect(rules.match(sample, { respectRepeat: false }).length > 0, sample.text).toBe(expected === "caught");
      }
    }
  });

  it("keeps matches in Log mode and respects delivery once per session", () => {
    const rules = registry();
    const sample = { source: "assistant-text" as const, boundary: "turn" as const, text: "I have not completed the task." };
    const matches = rules.match(sample, { turn: 1 });
    expect(matches).toMatchObject([{ ruleId: "local:check-unfinished-task", mode: "log" }]);
    rules.markDelivered(matches.map(match => match.ruleId), 1);
    expect(rules.match(sample, { turn: 2 })).toEqual([]);
  });
});
