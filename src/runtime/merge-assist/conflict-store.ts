/**
 * The checkout's in-flight conflicts in `merge_assist_state` under `conflicts:<checkout path>`: each as the status
 * shows it, with its files, the base lines an overlap contests, and its record. A dismissal or a resolution holds for
 * its id (its question and their ref) until Merge Assist is turned off for the repository, which clears them all.
 */
import { z } from "zod";
import { conflictSchema, type Conflict, type ConflictResolution } from "../../engine/merge-assist/model.js";
import { conflictRecordSchema } from "../../engine/merge-assist/conflict-record.js";
import { undimmed } from "../../engine/merge-assist/conflicts.js";
import type { HandlerResult } from "./conflict-hook.js";
import { STATE_KEYS, isoSeconds, readState, writeState, type StateQueries } from "./state.js";

export const storedConflictSchema = z.object({
  conflict: conflictSchema,
  /** The files the conflict is in: an overlap's file, or both decisions' scopes. */
  files: z.array(z.string()),
  /** For an overlap, the base lines their overlapping change covers, which the dimmer reads both sides' changes at. */
  lines: z.array(z.object({ lo: z.number(), hi: z.number() })).default([]),
  /**
   * For an overlap, the content anchor its id is built on (`contestedAnchor`); null for a contradiction, and for an
   * overlap stored before ids were anchored on content, which the next detection pass anchors once.
   */
  anchor: z.string().nullable().default(null),
  record: conflictRecordSchema,
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
 * What the store holds after a detection found `detected`: each found conflict, unless the person already settled
 * it, which then stays as it was; every settled conflict, found again or not; and no open conflict
 * that was not found again. A conflict found again keeps when it was first detected, its summary and the hook's last
 * answer, and, until the dimmer measures it again, the level it showed; a snooze the person gave meanwhile stands.
 */
export function mergeDetected(stored: readonly StoredConflict[], detected: readonly StoredConflict[]): StoredConflict[] {
  const prior = new Map(stored.map(entry => [entry.conflict.id, entry]));
  const found = detected.map(entry => {
    const before = prior.get(entry.conflict.id);
    if (before === undefined) return entry;
    if (before.conflict.status !== "open") return before;
    const { level, signals, score, reasons, queued, queuedFor, summary, hook } = before.conflict;
    const dimmed = entry.conflict.signals === null ? { level, signals, score, reasons, queued, queuedFor } : {};
    const snooze = before.conflict.snooze !== null && before.conflict.snooze.at !== entry.conflict.snooze?.at ? before.conflict.snooze : entry.conflict.snooze;
    return { ...entry, conflict: { ...entry.conflict, ...dimmed, snooze, detectedAt: before.conflict.detectedAt, summary: summary ?? entry.conflict.summary, hook } };
  });
  const ids = new Set(found.map(entry => entry.conflict.id));
  return [...found, ...stored.filter(entry => entry.conflict.status !== "open" && !ids.has(entry.conflict.id))];
}

/** `conflict` settled: replaced in the checkout's store. */
export function replaceConflict(db: StateQueries, checkoutPath: string, updated: StoredConflict, now: number): Conflict {
  const stored = readConflicts(db, checkoutPath);
  writeConflicts(db, checkoutPath, stored.map(entry => (entry.conflict.id === updated.conflict.id ? updated : entry)), now);
  return updated.conflict;
}

/** `stored` settled with `resolution`; settling resets the dimmer, so a conflict that comes back starts afresh. */
export function settledConflict(stored: StoredConflict, resolution: ConflictResolution): StoredConflict {
  const status = resolution.action === "dismiss" ? "dismissed" : "resolved";
  return { ...stored, conflict: { ...stored.conflict, ...undimmed(stored.conflict.id), status, resolution } };
}

/** `stored` dismissed by the person as not a conflict: it stays dismissed for its question and their ref. */
export function dismissedConflict(stored: StoredConflict, name: string, now: number): StoredConflict {
  return settledConflict(stored, { action: "dismiss", name, rule: null, text: null, at: isoSeconds(now) });
}

/** What a conflict hook handler answered, recorded on the conflict while the store still holds it. */
export function recordHandlerResult(db: StateQueries, checkoutPath: string, conflictId: string, result: HandlerResult, now: number): void {
  const stored = readConflicts(db, checkoutPath);
  const entry = stored.find(candidate => candidate.conflict.id === conflictId);
  if (entry === undefined) return;
  const conflict = { ...entry.conflict, ...(result.summary === undefined ? {} : { summary: result.summary }), ...(result.hook === undefined ? {} : { hook: result.hook }) };
  replaceConflict(db, checkoutPath, { ...entry, conflict }, now);
}
