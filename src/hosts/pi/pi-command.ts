import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { ConversationRuleRegistry } from "../../engine/catalog/registry.js";
import { describeMatch } from "../../engine/catalog/rule-schema.js";
import { internalTesting } from "../../engine/internal-testing.js";

export interface PiConversationRulesCommandRuntime {
  registry?: () => ConversationRuleRegistry;
  turn?: () => number;
  reload?: () => Promise<void> | void;
  remoteCommand?: () => ((args: string) => Promise<unknown>) | undefined;
}

/** Internal testing: `/cosine-rules`. */
export function registerPiConversationRulesCommand(pi: ExtensionAPI, runtime: PiConversationRulesCommandRuntime): void {
  pi.registerCommand("cosine-rules", {
    description: internalTesting("Inspect, test, or reload cosine-similarity trigger rules"),
    async handler(args, ctx) {
      const remoteCommand = runtime.remoteCommand?.();
      if (remoteCommand !== undefined) {
        const result = await remoteCommand(args);
        ctx.ui.notify(typeof result === "string" ? result : JSON.stringify(result, null, 2), "info");
        return;
      }
      const { reload, turn } = runtime;
      const registry = runtime.registry?.();
      if (registry === undefined || turn === undefined || reload === undefined) {
        throw new Error("cosine-rules extension received an event before session_start");
      }
      const trimmed = args.trim();
      const separator = trimmed.indexOf(" ");
      const operation = separator === -1 ? trimmed : trimmed.slice(0, separator);
      const remainder = separator === -1 ? "" : trimmed.slice(separator + 1).trim();

      switch (operation || "list") {
        case "list": {
          const lines = registry.inspect().map(item => {
            const rule = item.rule;
            return `${rule.id}: ${describeMatch(rule.match)} on ${rule.sample.source}/${rule.sample.boundary}`;
          });
          ctx.ui.notify(lines.length === 0 ? "No cosine rules are active." : lines.join("\n"), "info");
          return;
        }
        case "model":
          ctx.ui.notify(JSON.stringify(registry.fingerprint(), null, 2), "info");
          return;
        case "reload":
          await reload();
          ctx.ui.notify(`Reloaded ${registry.inspect().length} cosine rules.`, "info");
          return;
        case "test": {
          const ruleSeparator = remainder.indexOf(" ");
          if (ruleSeparator === -1) {
            ctx.ui.notify("Usage: /cosine-rules test <rule-id> <text>", "error");
            return;
          }
          const ruleId = remainder.slice(0, ruleSeparator);
          const text = remainder.slice(ruleSeparator + 1).trim();
          const inspection = registry.inspect().find(item => item.rule.id === ruleId);
          if (inspection === undefined) {
            ctx.ui.notify(`Unknown cosine rule: ${ruleId}`, "error");
            return;
          }
          const matches = registry.match({
            source: inspection.rule.sample.source,
            boundary: inspection.rule.sample.boundary,
            text,
          }, { turn: turn(), respectRepeat: false }).filter(match => match.ruleId === ruleId);
          ctx.ui.notify(JSON.stringify({ ruleId, matches }, null, 2), matches.length === 0 ? "warning" : "info");
          return;
        }
        default:
          ctx.ui.notify("Usage: /cosine-rules [list|model|reload|test <rule-id> <text>]", "error");
      }
    },
  });
}
