/**
 * The checkout's in-flight conflicts in `merge_assist_state` under `conflicts:<checkout path>`: each as the status
 * shows it, with its files, its record, and each side's position with its fingerprint, which a ledger entry for it
 * keeps. A dismissal or a resolution holds for its id (its question and their ref) until Merge Assist is turned off for
 * the repository, which clears them all, or until the winner's position moves (the ledger reopens it).
 */
import { z } from "zod";
import { conflictSchema, type Conflict } from "../../engine/merge-assist/model.js";
import { conflictRecordSchema } from "../../engine/merge-assist/conflict-record.js";
import { positionSchema } from "../../engine/merge-assist/decision-entry.js";
import { STATE_KEYS, readState, writeState, type StateQueries } from "./state.js";

const sidePositionSchema = z.object({ fingerprint: z.string(), at: positionSchema });

export const storedConflictSchema = z.object({
  conflict: conflictSchema,
  /** The files the conflict is in: an overlap's file, or both decisions' scopes. */
  files: z.array(z.string()),
  record: conflictRecordSchema,
  /** Each side's position as detected: a = this branch, b = theirs. */
  positions: z.object({ a: sidePositionSchema, b: sidePositionSchema }),
});
export type StoredConflict = z.infer<typeof storedConflictSchema>;

const storeSchema = z.array(storedConflictSchema);

export function readConflicts(db: StateQueries, checkoutPath: string): StoredConflict[] {
  return readState(db, STATE_KEYS.conflicts(checkoutPath), storeSchema) ?? [];
}

export function writeConflicts(db: StateQueries, checkoutPath: string, conflicts: readonly StoredConflict[], now: number): void {
  writeState(db, STATE_KEYS.conflicts(checkoutPath), conflicts, now);
}

/**
 * What the store holds after a detection found `detected`: each found conflict, unless the person (or precedent)
 * already settled it, which then stays as it was; every settled conflict, found again or not; and no open conflict
 * that was not found again. A conflict found again keeps when it was first detected and, until the dimmer measures it
 * again, the level it showed; a snooze the person gave meanwhile stands.
 */
export function mergeDetected(stored: readonly StoredConflict[], detected: readonly StoredConflict[]): StoredConflict[] {
  const prior = new Map(stored.map(entry => [entry.conflict.id, entry]));
  const found = detected.map(entry => {
    const before = prior.get(entry.conflict.id);
    if (before === undefined) return entry;
    if (before.conflict.status !== "open") return before;
    const { level, signals, score, reasons, queued, queuedFor } = before.conflict;
    const dimmed = entry.conflict.signals === null ? { level, signals, score, reasons, queued, queuedFor } : {};
    const snooze = before.conflict.snooze !== null && before.conflict.snooze.at !== entry.conflict.snooze?.at ? before.conflict.snooze : entry.conflict.snooze;
    return { ...entry, conflict: { ...entry.conflict, ...dimmed, snooze, detectedAt: before.conflict.detectedAt } };
  });
  const ids = new Set(found.map(entry => entry.conflict.id));
  return [...found, ...stored.filter(entry => entry.conflict.status !== "open" && !ids.has(entry.conflict.id))];
}

/** The ids of conflicts that are new in `merged`: not in `stored` at all. */
export function newConflicts(stored: readonly StoredConflict[], merged: readonly StoredConflict[]): StoredConflict[] {
  const known = new Set(stored.map(entry => entry.conflict.id));
  return merged.filter(entry => !known.has(entry.conflict.id) && entry.conflict.status === "open");
}

/** `conflict` settled: replaced in the checkout's store. */
export function replaceConflict(db: StateQueries, checkoutPath: string, updated: StoredConflict, now: number): Conflict {
  const stored = readConflicts(db, checkoutPath);
  writeConflicts(db, checkoutPath, stored.map(entry => (entry.conflict.id === updated.conflict.id ? updated : entry)), now);
  return updated.conflict;
}
