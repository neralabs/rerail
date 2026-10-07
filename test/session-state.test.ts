import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { CosineRuleRegistry } from "../src/registry.js";
import { replaySessionState } from "../src/session-state.js";
import { MATCH_ENTRY_TYPE, MUTATION_ENTRY_TYPE, TURN_ENTRY_TYPE } from "../src/extension.js";
import type { CosineRule } from "../src/rule-schema.js";

const rule: CosineRule = {
  id: "checkout",
  reference: "Guest checkout remains available.",
  match: { operator: "gte", threshold: 0.99 },
  sample: { source: "assistant-text", boundary: "message" },
  action: { delivery: "steer", content: "Check checkout." },
  repeat: { mode: "once" },
};

describe("replaySessionState", () => {
  it("replays mutations and delivered repeat state in branch order", () => {
    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    registry.replace([]);

    const turn = replaySessionState(registry, [
      {
        type: "custom",
        customType: MUTATION_ENTRY_TYPE,
        data: { operation: "upsert", rule, replace: false },
      },
      {
        type: "custom",
        customType: MATCH_ENTRY_TYPE,
        data: { ruleId: "checkout", turn: 3 },
      },
      {
        type: "custom",
        customType: TURN_ENTRY_TYPE,
        data: { turn: 7 },
      },
    ]);
    expect(turn).toBe(7);
    expect(registry.inspect()).toEqual([
      expect.objectContaining({
        rule: expect.objectContaining({ id: "checkout" }),
        lastDeliveredTurn: 3,
      }),
    ]);
  });
});
