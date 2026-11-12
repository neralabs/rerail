/**
 * The dimmer: how loud an open conflict gets, decided from what Git measures (docs/merge-assist-v1-build.md §9.1).
 * Interrupt only when continuing costs more than stopping: the expected rework if the conflict is left to merge is
 * `pLose × growthPerHour × hoursToMerge × lockIn × (1 + log2(1 + blastRadius))` lines, against what one interruption of
 * the person costs (`interruptCost`). Pure: the runtime measures the signals, keeps the budgets and stores the result.
 */
import { CONFLICT_LEVELS, type ConflictLevel, type ConflictSignals, type LockInClass } from "./model.js";
import type { DimmerConfig } from "./policy.js";

/** A level rises only when the score clears the next threshold by this factor, so it doesn't flicker. */
export const RISE_MARGIN = 1.2;
/** Below `interruptCost` times this, a conflict is only logged. */
export const AMBIENT_SHARE = 0.25;
/** At or above `interruptCost` times this (and only with lock-in ×4 and fast growth), the line stops. */
export const STOP_MULTIPLE = 4;
/** Stops one session gets. */
export const STOPS_PER_SESSION = 1;

/** The level an open conflict showed at the last pass; null for a conflict the dimmer has not seen. */
export interface DimmerPrevious {
  readonly level: ConflictLevel;
  /** The level it waits for while over budget; null when it is not queued. */
  readonly queuedFor: ConflictLevel | null;
}

/** What is left of the interruption budgets for this conflict: stops in the session, checkpoint prompts to the person today. */
export interface DimmerBudget {
  readonly stops: number;
  readonly checkpoints: number;
}

export interface DimmerInput {
  readonly signals: ConflictSignals;
  readonly previous: DimmerPrevious | null;
  readonly budget: DimmerBudget;
  readonly config: DimmerConfig;
}

export interface DimmerResult {
  readonly level: ConflictLevel;
  /** Expected rework lines if left to merge. */
  readonly score: number;
  /** One plain sentence per signal. */
  readonly reasons: string[];
  /** The level it waits for while over budget, shown below it (ambient, or a checkpoint for a stop) until the budget allows; null when it is not queued. */
  readonly queuedFor: ConflictLevel | null;
}

const rank = (level: ConflictLevel): number => CONFLICT_LEVELS.indexOf(level);

/** Expected rework lines if the conflict is left to merge. */
export function expectedRework(signals: ConflictSignals): number {
  const { pLose, growthPerHour, hoursToMerge, lockIn, blastRadius } = signals;
  return pLose * growthPerHour * hoursToMerge * lockIn * (1 + Math.log2(1 + blastRadius));
}

/**
 * The level the score calls for. Each level above the one the conflict already showed needs its threshold cleared by
 * {@link RISE_MARGIN}; a conflict seen for the first time by its threshold alone.
 */
function wantedLevel(signals: ConflictSignals, score: number, previous: DimmerPrevious | null, config: DimmerConfig): ConflictLevel {
  if (signals.quiet) return "logged";
  const margin = (level: ConflictLevel): number => (previous === null || rank(level) <= rank(previous.level) ? 1 : RISE_MARGIN);
  const stops = signals.lockIn === 4 && signals.growthPerHour > config.stopGrowthPerHour;
  if (stops && score >= config.interruptCost * STOP_MULTIPLE * margin("stop")) return "stop";
  if (score >= config.interruptCost * margin("checkpoint")) return "checkpoint";
  if (score >= config.interruptCost * AMBIENT_SHARE * margin("ambient")) return "ambient";
  return "logged";
}

/**
 * An open conflict's level: what the score calls for, never below the level it already showed (hysteresis), and
 * within the budgets. One over budget is queued for that level and keeps waiting for it, whatever the score does
 * meanwhile, until the budget frees (a new session for a stop, the next day for a checkpoint), then rises to it. While
 * it waits it shows as a checkpoint when it waits for a stop and a checkpoint fits today, else at ambient (or the
 * higher level it had).
 */
export function dimConflict({ signals, previous, budget, config }: DimmerInput): DimmerResult {
  const score = expectedRework(signals);
  const reasons = dimmerReasons(signals);
  const floor = previous?.level ?? "logged";
  const measured = wantedLevel(signals, score, previous, config);
  const waiting = previous?.queuedFor ?? "logged";
  const wanted = rank(waiting) > rank(measured) ? waiting : measured;
  if (rank(wanted) <= rank(floor)) return { level: floor, score, reasons, queuedFor: null };
  const room = wanted === "stop" ? budget.stops : wanted === "checkpoint" ? budget.checkpoints : 1;
  if (room >= 1) return { level: wanted, score, reasons, queuedFor: null };
  const fallback: ConflictLevel = wanted === "stop" && budget.checkpoints >= 1 ? "checkpoint" : "ambient";
  return { level: rank(floor) > rank(fallback) ? floor : fallback, score, reasons, queuedFor: wanted };
}

const LOCK_IN_TEXT: Readonly<Record<LockInClass, string>> = {
  persisted: "persisted schema or file format",
  "public-api": "public API",
  retirement: "retires a file or an exported symbol",
  exported: "exported signature",
  local: "local code",
};

/** `count` with its noun: `1 caller`, `3 callers`. */
export const plural = (count: number, one: string, many: string): string => `${count} ${count === 1 ? one : many}`;
const hours = (value: number): string => `${Math.max(1, Math.round(value))} h`;

/** One plain sentence per signal, as the conflict's card shows them. */
export function dimmerReasons(signals: ConflictSignals): string[] {
  if (signals.quiet) return ["only lockfiles, changelogs, generated output or docs"];
  const growth = signals.linesMine > 0
    ? `+${signals.linesTheirs} lines on theirs and +${signals.linesMine} on this branch in ${hours(signals.windowHours)}`
    : `+${signals.linesTheirs} lines on theirs in ${hours(signals.windowHours)}`;
  const blast = signals.otherRefs > 0
    ? `${plural(signals.callers, "caller", "callers")}, and ${plural(signals.otherRefs, "other branch touches", "other branches touch")} the same files`
    : plural(signals.callers, "caller", "callers");
  const merge = signals.pullRequest ? `pull request open: about ${hours(signals.hoursToMerge)} to merge` : `about ${hours(signals.hoursToMerge)} to merge`;
  return [`${LOCK_IN_TEXT[signals.lockInClass]}: ×${signals.lockIn}`, growth, blast, merge, "even odds either side loses"];
}
