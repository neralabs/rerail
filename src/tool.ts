import type { AgentToolResult, ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { CosineRuleRegistry } from "./registry.js";
import { MUTATION_ENTRY_TYPE, type RuleMutation } from "./session-state.js";
import { parseSampleSource, SAMPLE_BOUNDARIES } from "./rule-schema.js";

export interface CosineRulesToolRuntime {
  registry?: () => CosineRuleRegistry;
  turn?: () => number;
  remoteExecute?: () => ((input: unknown) => Promise<unknown>) | undefined;
}

function toolResult(details: Record<string, unknown>): AgentToolResult<Record<string, unknown>> {
  return {
    content: [{ type: "text", text: JSON.stringify(details, null, 2) }],
    details,
  };
}

export function registerCosineRulesTool(pi: ExtensionAPI, runtime: CosineRulesToolRuntime): void {
  const z = pi.zod;
  const sampleSchema = z.object({
    source: z.string(),
    boundary: z.enum(SAMPLE_BOUNDARIES),
    text: z.string(),
  });
  const parameters = z.object({
    action: z.enum(["add", "remove", "list", "test"]),
    rule: z.unknown().optional(),
    replace: z.boolean().optional(),
    ids: z.array(z.string()).optional(),
    sample: sampleSchema.optional(),
  });


  pi.registerTool({
    name: "cosine_rules",
    label: "Cosine Rules",
    description: "Add, remove, inspect, or test session-scoped cosine-similarity trigger rules.",
    parameters,
    approval: "write",
    loadMode: "discoverable",
    strict: true,
    async execute(_toolCallId, params, signal) {
      if (signal?.aborted) throw signal.reason;
      const input = parameters.parse(params);
      const remoteExecute = runtime.remoteExecute?.();
      if (remoteExecute !== undefined) {
        const details = await remoteExecute(input);
        if (details === null || typeof details !== "object" || Array.isArray(details)) {
          throw new Error("Rerail cosine response has an invalid result");
        }
        return toolResult(details as Record<string, unknown>);
      }
      const registry = runtime.registry?.();
      if (registry === undefined || runtime.turn === undefined) {
        throw new Error("cosine-rules extension received an event before session_start");
      }

      switch (input.action) {
        case "add": {
          if (input.rule === undefined) throw new TypeError("rule is required for action=add");
          const replace = input.replace === true;
          const registered = registry.upsert(input.rule, { replace });
          const mutation: RuleMutation = { operation: "upsert", rule: registered, replace };
          pi.appendEntry(MUTATION_ENTRY_TYPE, mutation);
          return toolResult({ operation: "add", ruleId: registered.id, replaced: replace });
        }

        case "remove": {
          if (input.ids === undefined || input.ids.length === 0) {
            throw new TypeError("ids must contain at least one rule id for action=remove");
          }
          const ids = [...new Set(input.ids)];
          const removed = registry.remove(ids);
          const mutation: RuleMutation = { operation: "remove", ids };
          pi.appendEntry(MUTATION_ENTRY_TYPE, mutation);
          return toolResult({ operation: "remove", removed, ids });
        }

        case "list":
          return toolResult({
            rules: registry.inspect(),
            modelFingerprint: registry.fingerprint(),
          });

        case "test": {
          if (input.sample === undefined) throw new TypeError("sample is required for action=test");
          const sample = { ...input.sample, source: parseSampleSource(input.sample.source, "sample.source") };
          const matches = registry.match(sample, {
            turn: runtime.turn(),
            respectRepeat: false,
          });
          return toolResult({ matches, modelFingerprint: registry.fingerprint() });
        }
      }
      throw new TypeError(`unsupported cosine_rules action: ${String(input.action)}`);
    },
  });
}
