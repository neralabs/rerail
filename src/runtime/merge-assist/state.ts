/**
 * Merge Assist's bookkeeping in `merge_assist_state`, one JSON value per key. A value that does not match what this
 * build expects (another version wrote it, or it was edited) reads as absent.
 */
import { eq } from "drizzle-orm";
import type { z } from "zod";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistState } from "../../engine/storage/schema.js";

export type StateQueries = Pick<RerailDatabase, "select" | "insert" | "update" | "delete">;

/** The keys Merge Assist keeps state under. */
export const STATE_KEYS = {
  /** The conflict notes a session was handed: `<conflict id>:<surface>` each (see `pendingNotes`). */
  delivered: (sessionId: string) => `delivered:${sessionId}`,
  /** The checkout's last scan (`LastScan`). */
  scan: (checkoutPath: string) => `scan:${checkoutPath}`,
  /** A scan of the checkout running in some process, since when. */
  scanning: (checkoutPath: string) => `scanning:${checkoutPath}`,
  /** The repository's last `gh` readiness, and when it was read. */
  github: (repositoryKey: string) => `github:${repositoryKey}`,
  /** The checkout's in-flight conflicts, open and dismissed (`conflict-store.ts`). */
  conflicts: (checkoutPath: string) => `conflicts:${checkoutPath}`,
  /** The interruptions the dimmer granted a person: checkpoint prompts per day, stops per session (`conflict-level.ts`). */
  budget: (person: string) => `budget:${person}`,
  /** The person's Merge Assist settings for a repository: the conflict hook command (`conflict-delivery.ts`). */
  settings: (repositoryKey: string) => `settings:${repositoryKey}`,
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

/** `2026-11-03T14:02:11Z`: an ISO time to the second, as the status carries it. */
export function isoSeconds(epochMs: number): string {
  return new Date(epochMs).toISOString().replace(/\.\d{3}Z$/u, "Z");
}
