/**
 * The Merge Assist notes a session's coding agent gets (docs/in-session-surfaces.md, "Merge Assist notes"): an open
 * conflict that is not quiet, as information, with what the person's conflict hook command added. A conflict goes to a
 * session once: what a session was handed is kept under `delivered:<session>`, and only once a delivery has taken it.
 * A note carries the person's line for its conflict (`conflict`).
 */
import { z } from "zod";
import type { ConflictNotice } from "../../engine/adapter-contract.js";
import { cappedLine, conflictNote, hookFollowUpNote, hookNote, sideName, STEER_LINE_CHARS } from "../../engine/code-rule/rerail-steer.js";
import { mergeAssistExperimentEnabled } from "../../engine/experimental-features.js";
import type { Conflict } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { statusText } from "./conflict-delivery.js";
import { readConflicts, type StoredConflict } from "./conflict-store.js";
import { STATE_KEYS, readState, writeState, type StateQueries } from "./state.js";

const deliveredSchema = z.array(z.string());

/** A note not yet handed to the session, the marks (`<conflict id>:<surface>`) of what it speaks for, and the conflict whose line the person sees with it. */
export interface PendingNote {
  readonly content: string;
  readonly marks: readonly string[];
  readonly conflict?: ConflictNotice;
}

/** The prefix every mark of a conflict starts with. */
const conflictMark = (conflict: Conflict): string => `${conflict.id}:`;

/** The mark of a conflict's own note: once per conflict and session. */
const openMark = (conflict: Conflict): string => `${conflictMark(conflict)}open`;

/** How a session takes a checkout's conflict notes: whether its host draws a status line or widget (the footer). */
export interface ConflictNoteOptions {
  readonly statusLine: boolean;
}

export interface PendingNoteOptions extends ConflictNoteOptions {
  /** Conflicts whose agent note waits for a conflict hook handler (the person's command) to answer. */
  readonly awaiting: ReadonlySet<string>;
}

/**
 * The person's line for an open conflict: what it is. On a host with no status line (Codex), the line carries the
 * conflict's line too. Every part from the other side is one line and cut short.
 */
function noticeOf(conflict: Conflict, options: ConflictNoteOptions): ConflictNotice {
  const summary = options.statusLine ? "" : ` — ${cappedLine(statusText(conflict), STEER_LINE_CHARS)}`;
  return { id: conflict.id, text: `your edit overlaps ${sideName(conflict.theirs, cappedLine)}: ${cappedLine(conflict.title)}${summary}` };
}

/**
 * `notice` as the person's line reads at delivery time: from the conflict as stored now, so a status the conflict
 * hook gave after the note was queued is the one it carries. Unchanged when the conflict is no longer open here.
 */
export function currentNotice(db: StateQueries, checkoutPath: string, notice: ConflictNotice, options: ConflictNoteOptions): ConflictNotice {
  const entry = readConflicts(db, checkoutPath).find(({ conflict }) => conflict.id === notice.id && conflict.status === "open");
  return entry === undefined ? notice : noticeOf(entry.conflict, options);
}

/** The open conflicts the session and the person hear about: every one but the quiet ones (those stay on the page and in the CLI). */
function surfacedConflicts(db: StateQueries, checkoutPath: string): StoredConflict[] {
  return readConflicts(db, checkoutPath).filter(({ conflict }) => conflict.status === "open" && !conflict.quiet);
}

/** What the person's conflict hook command added for the agent, when it answered. */
const hookNoteOf = (conflict: Conflict): string | null => (conflict.hook?.ok === true && conflict.hook.note?.trim() ? conflict.hook.note : null);

/** The mark of the hook's note in its latest answer, when it gave one: given once per conflict, event and session. */
const hookMark = (conflict: Conflict): string[] =>
  (hookNoteOf(conflict) === null ? [] : [`${conflictMark(conflict)}hook:${conflict.hook!.event}@${conflict.hook!.at}`]);

/**
 * One note per open conflict the session was not handed, once no hook handler it waits for is still to answer: who,
 * what they did, information only, with the hook's note and the person's line.
 */
function conflictNotes(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean, options: PendingNoteOptions): PendingNote[] {
  return conflicts.filter(({ conflict }) => !taken(openMark(conflict)) && !options.awaiting.has(conflict.id))
    .map(({ conflict, files }) => {
      const content = conflictNote({
        file: files[0] ?? conflict.title, peer: conflict.theirs.label, author: conflict.theirs.author, summary: conflict.theirs.summary,
        pullRequest: conflict.theirs.label.startsWith("PR #"),
      });
      return { content: hookNote(content, hookNoteOf(conflict)), marks: [openMark(conflict), ...hookMark(conflict)], conflict: noticeOf(conflict, options) };
    });
}

/**
 * The hook's note on its own, through the agent, for each conflict whose note went out before the person's command
 * answered (Rerail's note waits for it a second at most): once per conflict, event and session, attributed to the hook.
 */
function hookFollowUps(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean): PendingNote[] {
  return conflicts.flatMap(({ conflict }) => {
    const marks = hookMark(conflict);
    const note = hookNoteOf(conflict);
    if (note === null || marks.some(taken) || !taken(openMark(conflict))) return [];
    return [{ content: hookFollowUpNote({ title: conflict.title, peer: conflict.theirs.label }, note), marks }];
  });
}

/**
 * The notes for `sessionId` in the checkout that it has not been handed, the conflicts' own notes before the hook's
 * late notes; notes whose marks are in `inFlight` (queued for a delivery that has not taken them yet) are left out.
 * Reads only: {@link markNotesDelivered} records a note once it is handed over.
 */
export function pendingNotes(db: StateQueries, sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string>, options: PendingNoteOptions): PendingNote[] {
  const given = new Set(readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? []);
  const taken = (mark: string): boolean => given.has(mark) || inFlight.has(mark);
  const conflicts = surfacedConflicts(db, checkoutPath);
  return [...conflictNotes(conflicts, taken, options), ...hookFollowUps(conflicts, taken)];
}

/** What the footer shows of the conflicts in a session's checkout (docs/in-session-surfaces.md, "What the footer means"). */
export interface ConflictFooter {
  /** Open conflicts that are not quiet: the newest one's line, and `(+N)` for the rest. */
  readonly conflicts: number;
  /** The newest of them: `◐ <line>`, the hook's status when it gave one, else its summary. */
  readonly summary: { readonly id: string; readonly text: string };
}

/**
 * The footer's Merge Assist segment for a session that started in `checkoutPath` (its checkout's physical root): from
 * that checkout's conflicts alone; undefined when none is open but quiet ones, and while the experiment is off
 * ({@link mergeAssistExperimentEnabled}), whatever conflicts an earlier run stored. Reads only.
 */
export function conflictFooter(db: StateQueries, checkoutPath: string): ConflictFooter | undefined {
  if (!mergeAssistExperimentEnabled()) return undefined;
  const open = surfacedConflicts(db, checkoutPath);
  const newest = open.toSorted((left, right) => right.conflict.detectedAt.localeCompare(left.conflict.detectedAt))[0];
  if (newest === undefined) return undefined;
  return { conflicts: open.length, summary: { id: newest.conflict.id, text: statusText(newest.conflict) } };
}

/**
 * Record that `marks` were handed to the session: a conflict reaches a session once. Only marks of conflicts still in
 * the checkout are kept, which bounds the record; a conflict pruned so (it went away before a later delivery) is told
 * again should it be found again.
 */
export function markNotesDelivered(db: RerailDatabase, sessionId: string, checkoutPath: string, marks: readonly string[], now: number): void {
  if (marks.length === 0) return;
  db.transaction(tx => {
    const key = STATE_KEYS.delivered(sessionId);
    const given = readState(tx, key, deliveredSchema) ?? [];
    const current = readConflicts(tx, checkoutPath).map(entry => conflictMark(entry.conflict));
    writeState(tx, key, [...new Set([...given, ...marks])].filter(entry => current.some(version => entry.startsWith(version))), now);
  }, { behavior: "immediate" });
}
