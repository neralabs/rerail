import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { ExtensionAPI, ExtensionCommandContext } from "@oh-my-pi/pi-coding-agent";
import { registerCosineRulesCommand } from "../src/command.js";
import { CosineRuleRegistry } from "../src/registry.js";

const rule = {
  id: "checkout",
  mode: "log",
  reference: "Guest checkout remains available.",
  match: { operator: "gte", threshold: 0.99 },
  sample: { source: "assistant-text", boundary: "message" },
  action: { content: "record" },
  repeat: { mode: "always" },
} as const;

describe("/cosine-rules", () => {
  it("inspects, tests, and reloads the active registry", async () => {
    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    registry.replace([rule]);
    let handler: ((args: string, context: ExtensionCommandContext) => Promise<void>) | undefined;
    const pi = {
      registerCommand(_name: string, options: { handler: typeof handler }) {
        handler = options.handler;
      },
    } as unknown as ExtensionAPI;
    const reload = vi.fn();
    registerCosineRulesCommand(pi, { registry: () => registry, turn: () => 0, reload });
    if (handler === undefined) throw new Error("command was not registered");
    const notifications: string[] = [];
    const context = {
      ui: { notify: (message: string) => notifications.push(message) },
    } as unknown as ExtensionCommandContext;

    await handler("list", context);
    await handler("test checkout Guest checkout remains available.", context);
    await handler("model", context);
    await handler("reload", context);

    expect(notifications[0]).toContain("checkout");
    expect(notifications[1]).toContain('"similarity": 1');
    expect(notifications[2]).toContain("potion-base-8M");
    expect(reload).toHaveBeenCalledOnce();
  });
});
