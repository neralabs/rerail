import { describe, expect, it } from "vitest";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerPiCosineRulesTool } from "../src/pi-tool.js";
import { MUTATION_ENTRY_TYPE } from "../src/session-state.js";

interface RegisteredTool {
  execute(toolCallId: string, input: Record<string, unknown>, signal?: AbortSignal): Promise<unknown>;
}

function createHarness() {
  const entries: Array<{ customType: string; data: unknown }> = [];
  let tool: RegisteredTool | undefined;
  const registry = {
    upsert: (rule: { id: string }) => rule,
    remove: (ids: string[]) => ids.length,
    inspect: () => [],
    fingerprint: () => ({ model: "test" }),
    match: () => [],
  };
  const pi = {
    registerTool(next: RegisteredTool) {
      tool = next;
    },
    appendEntry(customType: string, data: unknown) {
      entries.push({ customType, data });
    },
  } as unknown as ExtensionAPI;
  registerPiCosineRulesTool(pi, { registry: () => registry as never, turn: () => 4 });
  if (tool === undefined) throw new Error("tool was not registered");
  return { entries, tool };
}

describe("Pi conversation_rules tool", () => {
  it("persists an idempotent deduplicated removal mutation", async () => {
    const harness = createHarness();

    await expect(harness.tool.execute("call-1", { action: "remove", ids: ["a", "a", "b"] })).resolves.toMatchObject({
      details: { operation: "remove", removed: 2, ids: ["a", "b"] },
    });
    expect(harness.entries).toEqual([{
      customType: MUTATION_ENTRY_TYPE,
      data: { operation: "remove", ids: ["a", "b"] },
    }]);
  });

  it("rejects malformed mutations and forwards host cancellation", async () => {
    const harness = createHarness();
    const controller = new AbortController();
    const reason = new Error("cancelled by Pi");
    controller.abort(reason);

    await expect(harness.tool.execute("call-2", { action: "remove", ids: [] })).rejects.toThrow("at least one rule id");
    await expect(harness.tool.execute("call-3", { action: "list" }, controller.signal)).rejects.toThrow(reason);
    expect(harness.entries).toEqual([]);
  });
});
