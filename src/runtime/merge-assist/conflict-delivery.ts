/**
 * Where conflicts reach the person (docs/merge-assist-v1-build.md §12.6): the person's `conflictDelivery` setting for a
 * repository, kept with Merge Assist's state on this machine (`settings:<repository>`), never in a committed file, with
 * the conflict hook's command; the status line's words for a conflict; and the delivery handler, which hands every
 * conflict event to the checkout's sessions. `chat` delivers through the coding agent (its note, the checkpoint card),
 * `terminal` through the status line and the person's line only, `both` both; a stop before an edit to a contested
 * file goes out in every mode.
 */
import { basename } from "node:path";
import { eq } from "drizzle-orm";
import {
  DEFAULT_MERGE_ASSIST_SETTINGS,
  mergeAssistSettingsSchema,
  type Conflict,
  type ConflictDelivery,
  type ConflictSummary,
  type MergeAssistSettings,
} from "../../engine/merge-assist/model.js";
import { checkouts } from "../../engine/storage/schema.js";
import type { ConflictHandler } from "./conflict-hook.js";
import type { StoredConflict } from "./conflict-store.js";
import { STATE_KEYS, isoSeconds, readState, writeState, type StateQueries } from "./state.js";

/** The person's settings for the repository: the defaults (both, no hook) until they change one. */
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

/** Whether the mode delivers through the coding agent: its note and the checkpoint card. */
export function viaChat(mode: ConflictDelivery): boolean {
  return mode !== "terminal";
}

/** Whether the mode delivers to the status line. */
export function viaTerminal(mode: ConflictDelivery): boolean {
  return mode !== "chat";
}

/**
 * The status line's words for a conflict until the quick call answers, or when it cannot:
 * `<their label> (<author>) is changing <symbol or file>`.
 */
export function fallbackSummary({ conflict, files }: Pick<StoredConflict, "conflict" | "files">, now: number): ConflictSummary {
  const who = conflict.theirs.author === null ? conflict.theirs.label : `${conflict.theirs.label} (${conflict.theirs.author})`;
  // An overlap's title is `<file> · <symbol>` when Git names the symbol, else the file's name.
  const [, symbol] = conflict.title.split(" · ");
  const what = conflict.kind !== "overlap" ? conflict.title : symbol ?? files[0] ?? basename(conflict.title);
  return { text: `${who} is changing ${what}`, via: "fallback", at: isoSeconds(now), reason: null };
}

/** A conflict's line on the status line: the hook's `status` when it gave one, else its summary. */
export function statusText(entry: Pick<StoredConflict, "conflict" | "files">, now: number): string {
  const { conflict } = entry;
  return conflict.hook?.ok === true && conflict.hook.status !== null ? conflict.hook.status : (conflict.summary ?? fallbackSummary(entry, now)).text;
}

/**
 * `conflict`, with the fallback summary when it has none yet, marked `pending` until the quick call answers. A process
 * that stops before it answers leaves the fallback, still pending, until the conflict is detected afresh.
 */
export function withSummary<Entry extends { conflict: Conflict; files: string[] }>(entry: Entry, now: number): Entry {
  return entry.conflict.summary !== null ? entry : { ...entry, conflict: { ...entry.conflict, summary: { ...fallbackSummary(entry, now), reason: "pending" } } };
}

/**
 * The delivery handler: every event is something the checkout's sessions may have to hear (a note, a card, a line),
 * so it offers them what is pending, per the mode, at their next boundary. What each mode delivers is read when the
 * session takes its notes (`notes.ts`) and when the footer is drawn.
 */
export const deliveryHandler: ConflictHandler = {
  name: "delivery",
  on: async (_event, _conflict, context) => {
    context.deliver();
  },
};
