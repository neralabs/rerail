/**
 * The checkout's in-flight conflicts in `merge_assist_state` under `conflicts:<checkout path>`: each as the status
 * shows it, with its file, the base lines it contests and their content anchor. A dismissal holds for its id (its
 * question and their ref) until Merge Assist is turned off for the repository, which clears them all.
 */
import { z } from "zod";
import { conflictSchema, type Conflict } from "../../engine/merge-assist/model.js";
import { undimmed } from "../../engine/merge-assist/conflicts.js";
import type { HandlerResult } from "./conflict-hook.js";
import { STATE_KEYS, isoSeconds, readState, writeState, type StateQueries } from "./state.js";

export const storedConflictSchema = z.object({
  conflict: conflictSchema,
  /** The file the conflict is in. */
  files: z.array(z.string()),
  /** The base lines their overlapping change covers, which the dimmer reads both sides' changes at. */
  lines: z.array(z.object({ lo: z.number(), hi: z.number() })),
  /** The content anchor its id is built on (`contestedAnchor`). */
  anchor: z.string(),
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
 * it, which then stays as it was; every dismissed conflict, found again or not; and no open conflict
 * that was not found again. A conflict found again keeps when it was first detected and the hook's last answer, and,
 * until the dimmer measures it again, the level it showed.
 */
export function mergeDetected(stored: readonly StoredConflict[], detected: readonly StoredConflict[]): StoredConflict[] {
  const prior = new Map(stored.map(entry => [entry.conflict.id, entry]));
  const found = detected.map(entry => {
    const before = prior.get(entry.conflict.id);
    if (before === undefined) return entry;
    if (before.conflict.status !== "open") return before;
    const { level, signals, score, reasons, queued, queuedFor, hook } = before.conflict;
    const dimmed = entry.conflict.signals === null ? { level, signals, score, reasons, queued, queuedFor } : {};
    return { ...entry, conflict: { ...entry.conflict, ...dimmed, detectedAt: before.conflict.detectedAt, hook } };
  });
  const ids = new Set(found.map(entry => entry.conflict.id));
  return [...found, ...stored.filter(entry => entry.conflict.status !== "open" && !ids.has(entry.conflict.id))];
}

/** `updated` replaced in the checkout's store. */
export function replaceConflict(db: StateQueries, checkoutPath: string, updated: StoredConflict, now: number): Conflict {
  const stored = readConflicts(db, checkoutPath);
  writeConflicts(db, checkoutPath, stored.map(entry => (entry.conflict.id === updated.conflict.id ? updated : entry)), now);
  return updated.conflict;
}

/**
 * `stored` dismissed by the person as not a conflict: it stays dismissed for its question and their ref. Dismissing
 * resets the dimmer.
 */
export function dismissedConflict(stored: StoredConflict, name: string, now: number): StoredConflict {
  return { ...stored, conflict: { ...stored.conflict, ...undimmed(stored.conflict.id), status: "dismissed", dismissal: { name, at: isoSeconds(now) } } };
}

/** What the conflict hook command answered, recorded on the conflict while the store still holds it. */
export function recordHandlerResult(db: StateQueries, checkoutPath: string, conflictId: string, result: HandlerResult, now: number): void {
  const stored = readConflicts(db, checkoutPath);
  const entry = stored.find(item => item.conflict.id === conflictId);
  if (entry === undefined) return;
  replaceConflict(db, checkoutPath, { ...entry, conflict: { ...entry.conflict, hook: result.hook } }, now);
}
