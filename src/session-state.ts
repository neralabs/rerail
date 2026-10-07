import type { CosineRuleRegistry } from "./registry.js";
import { parseCosineRule, type CosineRule } from "./rule-schema.js";

export const MATCH_ENTRY_TYPE = "omp-cosine-rules.match";
export const MUTATION_ENTRY_TYPE = "omp-cosine-rules.mutation";
export const TURN_ENTRY_TYPE = "omp-cosine-rules.turn";

export type RuleMutation =
  | { operation: "upsert"; rule: CosineRule; replace: boolean }
  | { operation: "remove"; ids: string[] };

export interface SessionEntryLike {
  type: string;
  customType?: string;
  data?: unknown;
}

function entryData(entry: SessionEntryLike): Record<string, unknown> | undefined {
  if (entry.type !== "custom") return undefined;
  const data = entry.data;
  if (data === null || typeof data !== "object" || Array.isArray(data)) return undefined;
  return data as Record<string, unknown>;
}

/** A persisted rule that no longer parses is skipped with a notice, so it cannot block session restore. */
export function restorableRule(value: unknown, path: string): CosineRule | undefined {
  try {
    return parseCosineRule(value, path);
  } catch (error) {
    process.stderr.write(`rerail: skipped unreadable rule mutation: ${error instanceof Error ? error.message : String(error)}\n`);
    return undefined;
  }
}

function replayMutation(registry: CosineRuleRegistry, data: Record<string, unknown>): void {
  if (data.operation === "upsert") {
    const rule = restorableRule(data.rule, "session mutation rule");
    if (rule !== undefined) registry.upsert(rule, { replace: data.replace === true });
  } else if (data.operation === "remove" && Array.isArray(data.ids)) {
    registry.remove(data.ids.filter((id): id is string => typeof id === "string"));
  }
}

function replayEntry(
  registry: CosineRuleRegistry,
  entry: SessionEntryLike,
  currentTurn: number,
): number {
  const data = entryData(entry);
  if (data === undefined) return currentTurn;
  if (entry.customType === MUTATION_ENTRY_TYPE) {
    replayMutation(registry, data);
    return currentTurn;
  }
  if (entry.customType === TURN_ENTRY_TYPE) {
    if (!Number.isInteger(data.turn) || (data.turn as number) < 0) return currentTurn;
    return Math.max(currentTurn, data.turn as number);
  }
  if (entry.customType !== MATCH_ENTRY_TYPE) return currentTurn;
  if (typeof data.ruleId !== "string" || !Number.isInteger(data.turn) || (data.turn as number) < 0) {
    return currentTurn;
  }
  const deliveredTurn = data.turn as number;
  registry.restoreDelivery(data.ruleId, deliveredTurn);
  return Math.max(currentTurn, deliveredTurn);
}

export function replaySessionState(
  registry: CosineRuleRegistry,
  entries: readonly SessionEntryLike[],
): number {
  let currentTurn = 0;
  for (const entry of entries) currentTurn = replayEntry(registry, entry, currentTurn);
  return currentTurn;
}
