/**
 * The Merge Assist notes a session's coding agent gets (docs/in-session-surfaces.md, "Merge Assist notes"): the
 * decision it saved from the person's correction in that session; a decision the person settled a conflict with; a
 * teammate's decision that is a note, once absorbed or adopted; a trusted teammate's decision that contradicts this
 * branch's (Talk); an ambient in-flight overlap; and how many wait for the person's review. Each goes to a session at
 * most once per version of its candidate (an overlap once per conflict): what a session was handed is kept under
 * `delivered:<session>`, and only once a delivery has taken it. A note about a conflict carries the person's line
 * for it (`conflict`).
 */
import { eq } from "drizzle-orm";
import { z } from "zod";
import type { ConflictNotice } from "../../engine/adapter-contract.js";
import { conflictNote, decidedNote, mergeAssistNote } from "../../engine/code-rule/rerail-steer.js";
import type { Conflict } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems, type MergeAssistItemRow } from "../../engine/storage/schema.js";
import { activationFailures, itemRuleId } from "./activation.js";
import { readConflicts, type StoredConflict } from "./conflict-store.js";
import { STATE_KEYS, readState, writeState, type StateQueries } from "./state.js";

const deliveredSchema = z.array(z.string());

/**
 * A note not yet handed to the session, the marks (`<item id>:<candidate hash>:<news>`, or `<conflict id>:open`) of
 * what it speaks for, and the conflict whose line the person sees with it.
 */
export interface PendingNote {
  readonly content: string;
  readonly marks: readonly string[];
  readonly conflict?: ConflictNotice;
}

function evidenceOf(item: MergeAssistItemRow): { mine?: { decision?: unknown }; ruleProblem?: unknown; branch?: unknown; resolves?: unknown; peer?: unknown } {
  return item.evidence as { mine?: { decision?: unknown }; ruleProblem?: unknown; branch?: unknown; resolves?: unknown; peer?: unknown };
}

/** `<branch>` or `PR #<n>`, as a note names where a decision came from. */
function peerLabel(item: MergeAssistItemRow): string {
  if (item.peerKind === "pull-request") return `PR #${item.peerNumber}`;
  const branch = evidenceOf(item).branch;
  return typeof branch === "string" ? branch : item.peerRef;
}

/** Who wrote a teammate's decision: GitHub's author of the pull request, else the name the candidate gives itself. */
function authorOf(item: MergeAssistItemRow): string {
  return item.peerKind === "pull-request" && item.peerAuthor !== null ? item.peerAuthor : item.candidate.author;
}

/** What an item can be news as for a session: its saved decision, a settled conflict, an applied note, a contradiction, or a review. */
type News = "saved" | "decided" | "applied" | "talk" | "review";

/** What one of this checkout's own decisions is news as: saved in this session, or settling a conflict (for every session). */
function ownNews(item: MergeAssistItemRow, sessionId: string): News | null {
  if (item.status !== "applied") return null;
  if (item.sessionId === sessionId) return "saved";
  return item.sessionId === null && typeof evidenceOf(item).resolves === "string" ? "decided" : null;
}

/** What an item is news as for `sessionId`, or null when it is no news for the agent. */
function newsOf(item: MergeAssistItemRow, sessionId: string, failed: Readonly<Record<string, string>>): News | null {
  if (item.origin === "own") return ownNews(item, sessionId);
  if (item.status === "applied" && (item.tier === "absorb" || item.tier === "review")) {
    // Absorbed by the scan, or adopted from review by the person. A decision with a working code rule steers like any
    // rule; only a note reaches the agent as text.
    const note = item.candidate.rule === undefined || typeof evidenceOf(item).ruleProblem === "string" || failed[itemRuleId(item)] !== undefined;
    return note ? "applied" : null;
  }
  if (item.status !== "open") return null;
  // Only a trusted teammate's text is quoted to the agent; an untrusted one waits in review, counted.
  if (item.tier === "talk") return item.trusted && typeof evidenceOf(item).mine?.decision === "string" ? "talk" : null;
  return item.tier === "review" ? "review" : null;
}

/** The prefix every mark of an item's version starts with. */
const versionOf = (item: MergeAssistItemRow): string => `${item.id}:${item.candidateHash}:`;
/** The prefix every mark of a conflict starts with. */
const conflictMark = (conflict: Conflict): string => `${conflict.id}:`;

const who = (side: Conflict["theirs"]): string => (side.author === null ? side.label : `${side.label} (${side.author})`);

/** The person's line for an ambient open conflict, as the note that surfaces it carries it. */
function noticeOf(conflict: Conflict): ConflictNotice {
  const text = conflict.kind === "overlap"
    ? `your edit overlaps ${who(conflict.theirs)}: ${conflict.title}`
    : `${who(conflict.theirs)} decided differently: ${conflict.title}`;
  return { id: conflict.id, text };
}

/** The open conflicts the session and the person hear about: ambient ones (logged ones stay on the page). */
function ambientConflicts(db: StateQueries, checkoutPath: string): StoredConflict[] {
  return readConflicts(db, checkoutPath).filter(({ conflict }) => conflict.status === "open" && conflict.level === "ambient");
}

/** One note per ambient overlap the session was not handed: who, what they change, information only. */
function overlapNotes(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean): PendingNote[] {
  return conflicts.filter(({ conflict }) => conflict.kind === "overlap" && !taken(`${conflictMark(conflict)}open`)).map(({ conflict, files }) => ({
    content: conflictNote({
      file: files[0] ?? conflict.title,
      peer: conflict.theirs.label,
      author: conflict.theirs.author,
      summary: conflict.theirs.summary,
      pullRequest: conflict.theirs.label.startsWith("PR #"),
    }),
    marks: [`${conflictMark(conflict)}open`],
    conflict: noticeOf(conflict),
  }));
}

/**
 * The notes for `sessionId` in the checkout that it has not been handed, in the order saved, decided, applied, talk,
 * overlap, review; items whose marks are in `inFlight` (queued for a delivery that has not taken them yet) are left
 * out. An item's mark is `<item id>:<candidate hash>:<news>`, so an item is news again when it moves, as from review to
 * adopted. Reads only: {@link markNotesDelivered} records a note once it is handed over.
 */
export function pendingNotes(db: StateQueries, sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string>): PendingNote[] {
  const given = new Set(readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? []);
  const taken = (mark: string): boolean => given.has(mark) || inFlight.has(mark);
  const items = db.select().from(mergeAssistItems).where(eq(mergeAssistItems.checkoutPath, checkoutPath)).all();
  const failed = activationFailures(db, checkoutPath);
  const fresh: Record<News, Array<{ item: MergeAssistItemRow; mark: string }>> = { saved: [], decided: [], applied: [], talk: [], review: [] };
  for (const item of items) {
    const news = newsOf(item, sessionId, failed);
    const mark = `${versionOf(item)}${news}`;
    if (news !== null && !taken(mark)) fresh[news].push({ item, mark });
  }
  const conflicts = ambientConflicts(db, checkoutPath);
  return [...itemNotes(fresh, conflicts.map(entry => entry.conflict)), ...overlapNotes(conflicts, taken), ...reviewNote(fresh.review)];
}

/** The notes for fresh items, but the review count: each of saved, decided, applied and talk on its own. */
function itemNotes(fresh: Record<News, Array<{ item: MergeAssistItemRow; mark: string }>>, conflicts: readonly Conflict[]): PendingNote[] {
  return [
    ...fresh.saved.map(({ item, mark }) => ({ content: mergeAssistNote({ kind: "saved", decision: item.candidate.decision, candidateId: item.candidateId }), marks: [mark] })),
    ...fresh.decided.map(({ item, mark }) => ({ content: decidedNote(item.candidate.decision, String(evidenceOf(item).peer ?? "another branch")), marks: [mark] })),
    ...fresh.applied.map(({ item, mark }) => ({ content: mergeAssistNote({ kind: "absorbed", decision: item.candidate.decision, author: authorOf(item), peer: peerLabel(item) }), marks: [mark] })),
    ...fresh.talk.map(({ item, mark }) => talkNote(item, mark, conflicts)),
  ];
}

/** A Talk note, with the person's line for the contradiction it is when that conflict is open. */
function talkNote(item: MergeAssistItemRow, mark: string, conflicts: readonly Conflict[]): PendingNote {
  const mine = String(evidenceOf(item).mine?.decision);
  const content = mergeAssistNote({ kind: "talk", author: authorOf(item), peer: peerLabel(item), theirs: item.candidate.decision, mine });
  const conflict = conflicts.find(entry => entry.kind === "decisions" && entry.theirs.decisionId === item.candidateId);
  return conflict === undefined ? { content, marks: [mark] } : { content, marks: [mark], conflict: noticeOf(conflict) };
}

function reviewNote(review: ReadonlyArray<{ mark: string }>): PendingNote[] {
  if (review.length === 0) return [];
  return [{ content: mergeAssistNote({ kind: "review", count: review.length }), marks: review.map(({ mark }) => mark) }];
}

/**
 * Record that `marks` were handed to the session. Only marks of items and conflicts still in the checkout are kept: a
 * withdrawn decision that comes back is news again.
 */
export function markNotesDelivered(db: RerailDatabase, sessionId: string, checkoutPath: string, marks: readonly string[], now: number): void {
  if (marks.length === 0) return;
  db.transaction(tx => {
    const key = STATE_KEYS.delivered(sessionId);
    const given = readState(tx, key, deliveredSchema) ?? [];
    const current = [
      ...tx.select().from(mergeAssistItems).where(eq(mergeAssistItems.checkoutPath, checkoutPath)).all().map(versionOf),
      ...readConflicts(tx, checkoutPath).map(entry => conflictMark(entry.conflict)),
    ];
    writeState(tx, key, [...new Set([...given, ...marks])].filter(entry => current.some(version => entry.startsWith(version))), now);
  }, { behavior: "immediate" });
}
