import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import type { CosineRuleRegistry } from "./registry.js";
import { MUTATION_ENTRY_TYPE, type RuleMutation } from "./session-state.js";
import { parseSampleSource, SAMPLE_BOUNDARIES } from "./rule-schema.js";

export interface PiCosineRulesToolRuntime {
  registry?: () => CosineRuleRegistry;
  turn?: () => number;
  remoteExecute?: () => ((input: unknown) => Promise<unknown>) | undefined;
}

const parameters = Type.Object({
  action: Type.Union([
    Type.Literal("add"),
    Type.Literal("remove"),
    Type.Literal("list"),
    Type.Literal("test"),
  ]),
  rule: Type.Optional(Type.Unknown()),
  replace: Type.Optional(Type.Boolean()),
  ids: Type.Optional(Type.Array(Type.String())),
  sample: Type.Optional(Type.Object({
    source: Type.String(),
    boundary: Type.Union(SAMPLE_BOUNDARIES.map(boundary => Type.Literal(boundary))),
    text: Type.String(),
  })),
});

function toolResult(details: Record<string, unknown>) {
  return {
    content: [{ type: "text" as const, text: JSON.stringify(details, null, 2) }],
    details,
  };
}

export function registerPiCosineRulesTool(pi: ExtensionAPI, runtime: PiCosineRulesToolRuntime): void {
  pi.registerTool({
    name: "conversation_rules",
    label: "Cosine Rules",
    description: "Add, remove, inspect, or test session-scoped cosine-similarity trigger rules.",
    parameters,
    async execute(_toolCallId, input, signal) {
      if (signal?.aborted) throw signal.reason;
      const remoteExecute = runtime.remoteExecute?.();
      if (remoteExecute !== undefined) {
        const details = await remoteExecute(input);
        if (details === null || typeof details !== "object" || Array.isArray(details)) {
          throw new Error("Rerail development cosine response has an invalid result");
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
          return toolResult({ rules: registry.inspect(), modelFingerprint: registry.fingerprint() });
        case "test": {
          if (input.sample === undefined) throw new TypeError("sample is required for action=test");
          const sample = { ...input.sample, source: parseSampleSource(input.sample.source, "sample.source") };
          const matches = registry.match(sample, { turn: runtime.turn(), respectRepeat: false });
          return toolResult({ matches, modelFingerprint: registry.fingerprint() });
        }
      }
    },
  });
}
