/**
 * Merge Assist's bookkeeping in `merge_assist_state`, one JSON value per key, and the ids of its items. A value that
 * does not match what this build expects (another version wrote it, or it was edited) reads as absent.
 */
import { createHash } from "node:crypto";
import { eq } from "drizzle-orm";
import type { z } from "zod";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistState, type MergeAssistOrigin } from "../../engine/storage/schema.js";

export type StateQueries = Pick<RerailDatabase, "select" | "insert" | "update" | "delete">;

/** The keys Merge Assist keeps state under. */
export const STATE_KEYS = {
  /** The last event of a session that capture read, and whether it has read the session's first prompt (the task). */
  cursor: (sessionId: string) => `cursor:${sessionId}`,
  /** The notes a session was handed: `<item id>:<candidate hash>:<news>` each (see `pendingNotes`). */
  delivered: (sessionId: string) => `delivered:${sessionId}`,
  /** The checkout's last scan (`LastScan`). */
  scan: (checkoutPath: string) => `scan:${checkoutPath}`,
  /** A scan of the checkout running in some process, since when. */
  scanning: (checkoutPath: string) => `scanning:${checkoutPath}`,
  /** The candidate rules the last reconciliation could not store, by rule id, with why. */
  activation: (checkoutPath: string) => `activation:${checkoutPath}`,
  /** The repository's last `gh` readiness, and when it was read. */
  github: (repositoryKey: string) => `github:${repositoryKey}`,
  /** The checkout's in-flight conflicts, open, resolved and dismissed (`conflict-store.ts`). */
  conflicts: (checkoutPath: string) => `conflicts:${checkoutPath}`,
} as const;

export function readState<T>(db: StateQueries, key: string, schema: z.ZodType<T>): T | undefined {
  const row = db.select({ value: mergeAssistState.valueJson }).from(mergeAssistState).where(eq(mergeAssistState.key, key)).get();
  if (row === undefined) return undefined;
  let value: unknown;
  try {
    value = JSON.parse(row.value);
  } catch {
    return undefined;
  }
  const parsed = schema.safeParse(value);
  return parsed.success ? parsed.data : undefined;
}

export function writeState(db: StateQueries, key: string, value: unknown, now: number): void {
  const valueJson = JSON.stringify(value);
  db.insert(mergeAssistState).values({ key, valueJson, updatedAt: now })
    .onConflictDoUpdate({ target: mergeAssistState.key, set: { valueJson, updatedAt: now } })
    .run();
}

export function deleteState(db: StateQueries, key: string): void {
  db.delete(mergeAssistState).where(eq(mergeAssistState.key, key)).run();
}

/**
 * An item's id, from what makes it unique (checkout, origin, peer and candidate): short enough to type after
 * `rerail merge-assist adopt`, and the same each time the same candidate is found again.
 */
export function itemId(checkoutPath: string, origin: MergeAssistOrigin, peerRef: string, candidateId: string): string {
  const digest = createHash("sha256").update([checkoutPath, origin, peerRef, candidateId].join("\0")).digest("hex");
  return `ma-${digest.slice(0, 12)}`;
}

/** `2026-11-03T14:02:11Z`: an ISO time to the second, as candidate files and the status carry it. */
export function isoSeconds(epochMs: number): string {
  return new Date(epochMs).toISOString().replace(/\.\d{3}Z$/u, "Z");
}
