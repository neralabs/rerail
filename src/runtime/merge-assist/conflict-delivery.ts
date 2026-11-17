/**
 * How conflicts reach the person (docs/merge-assist-v1-build.md): the person's settings for a repository (the
 * conflict hook's command), kept with Merge Assist's state on this machine (`settings:<repository>`), never in a
 * committed file; a conflict's line; and the delivery handler, which hands every conflict event to the checkout's
 * sessions: the agent's note, the person's line and the footer.
 */
import { eq } from "drizzle-orm";
import { DEFAULT_MERGE_ASSIST_SETTINGS, mergeAssistSettingsSchema, type Conflict, type MergeAssistSettings } from "../../engine/merge-assist/model.js";
import { checkouts } from "../../engine/storage/schema.js";
import type { ConflictHandler } from "./conflict-hook.js";
import { STATE_KEYS, readState, writeState, type StateQueries } from "./state.js";

/** The person's settings for the repository: the defaults (no hook) until they change one. */
export function readSettings(db: StateQueries, repositoryKey: string): MergeAssistSettings {
  return readState(db, STATE_KEYS.settings(repositoryKey), mergeAssistSettingsSchema) ?? DEFAULT_MERGE_ASSIST_SETTINGS;
}

export function writeSettings(db: StateQueries, repositoryKey: string, settings: MergeAssistSettings, now: number): void {
  writeState(db, STATE_KEYS.settings(repositoryKey), settings, now);
}

/** The settings of the repository the recorded checkout at `checkoutPath` belongs to; the defaults for one not recorded. */
export function settingsAt(db: StateQueries, checkoutPath: string): MergeAssistSettings {
  const row = db.select({ repositoryKey: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).get();
  return row === undefined ? DEFAULT_MERGE_ASSIST_SETTINGS : readSettings(db, row.repositoryKey);
}

/** A conflict's line where it shows: the hook's `status` when it gave one, else its summary. */
export function statusText(conflict: Conflict): string {
  return conflict.hook?.ok === true && conflict.hook.status !== null ? conflict.hook.status : conflict.summary;
}

/**
 * The delivery handler: every event is something the checkout's sessions may have to hear (a note, a line),
 * so it offers them what is pending at their next boundary. What each takes is read when the session takes its notes
 * (`notes.ts`) and when the footer is drawn.
 */
export const deliveryHandler: ConflictHandler = {
  name: "delivery",
  on: async (_event, _conflict, context) => {
    context.deliver();
  },
};
