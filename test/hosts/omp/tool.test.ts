import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { ExtensionAPI, ToolDefinition } from "@oh-my-pi/pi-coding-agent";
import * as zod from "@oh-my-pi/omptype/zod";
import { ConversationRuleRegistry } from "../../../src/engine/index.js";
import { registerConversationRulesTool } from "../../../src/hosts/omp/tool.js";

const rule = {
  id: "checkout",
  mode: "steer",
  reference: "Guest checkout remains available.",
  match: { operator: "gte", threshold: 0.99 },
  sample: { source: "assistant-text", boundary: "message" },
  action: { content: "Check checkout." },
  repeat: { mode: "once" },
} as const;

describe("conversation_rules tool", () => {
  it("adds, tests, lists, and removes session rules", async () => {
    const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
    registry.replace([]);
    let definition: ToolDefinition | undefined;
    const entries: unknown[] = [];
    const pi = {
      zod,
      registerTool(tool: ToolDefinition) {
        definition = tool;
      },
      appendEntry(_customType: string, data: unknown) {
        entries.push(data);
      },
    } as unknown as ExtensionAPI;
    registerConversationRulesTool(pi, { registry: () => registry, turn: () => 0 });
    if (definition === undefined) throw new Error("tool was not registered");

    const add = await definition.execute("call-1", { action: "add", rule }, undefined, undefined, {} as never);
    expect(add.details).toEqual(expect.objectContaining({ operation: "add", ruleId: "checkout" }));

    const test = await definition.execute(
      "call-2",
      {
        action: "test",
        sample: {
          source: "assistant-text",
          boundary: "message",
          text: "Guest checkout remains available.",
        },
      },
      undefined,
      undefined,
      {} as never,
    );
    expect(test.details).toEqual(expect.objectContaining({ matches: [expect.objectContaining({ ruleId: "checkout" })] }));

    const list = await definition.execute("call-3", { action: "list" }, undefined, undefined, {} as never);
    expect(list.details).toEqual(expect.objectContaining({ rules: [expect.objectContaining({ rule: expect.objectContaining({ id: "checkout" }) })] }));

    await definition.execute("call-4", { action: "remove", ids: ["checkout"] }, undefined, undefined, {} as never);
    expect(registry.inspect()).toEqual([]);
    expect(entries).toHaveLength(2);
  });
});
