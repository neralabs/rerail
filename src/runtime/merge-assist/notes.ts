/**
 * The Merge Assist notes a session's coding agent gets (docs/in-session-surfaces.md, "Merge Assist notes"): the
 * decision it saved from the person's correction in that session; a teammate's decision that is a note, once absorbed
 * or adopted; a trusted teammate's decision that contradicts this branch's (Talk); and how many wait for the person's
 * review. Each goes to a session at most once per version of its candidate: what a session was handed is kept under
 * `delivered:<session>`, and only once a delivery has taken it.
 */
import { eq } from "drizzle-orm";
import { z } from "zod";
import { mergeAssistItems, type MergeAssistItemRow } from "../../engine/storage/schema.js";
import { mergeAssistNote } from "../../engine/code-rule/rerail-steer.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { activationFailures, itemRuleId } from "./activation.js";
import { STATE_KEYS, readState, writeState, type StateQueries } from "./state.js";

const deliveredSchema = z.array(z.string());

/** A note not yet handed to the session, and the marks (`<item id>:<candidate hash>`) of the items it speaks for. */
export interface PendingNote {
  readonly content: string;
  readonly marks: readonly string[];
}

function evidenceOf(item: MergeAssistItemRow): { mine?: { decision?: unknown }; ruleProblem?: unknown; branch?: unknown } {
  return item.evidence as { mine?: { decision?: unknown }; ruleProblem?: unknown; branch?: unknown };
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

/** What an item can be news as for a session: its saved decision, an applied note, a contradiction, or a review. */
type News = "saved" | "applied" | "talk" | "review";

/** What an item is news as for `sessionId`, or null when it is no news for the agent. */
function newsOf(item: MergeAssistItemRow, sessionId: string, failed: Readonly<Record<string, string>>): News | null {
  if (item.origin === "own") return item.sessionId === sessionId && item.status === "applied" ? "saved" : null;
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

/**
 * The notes for `sessionId` in the checkout that it has not been handed, in the order saved, applied, talk, review;
 * items whose marks are in `inFlight` (queued for a delivery that has not taken them yet) are left out. A mark is
 * `<item id>:<candidate hash>:<news>`, so an item is news again when it moves, as from review to adopted. Reads only:
 * {@link markNotesDelivered} records a note once it is handed over.
 */
export function pendingNotes(db: StateQueries, sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string>): PendingNote[] {
  const given = new Set(readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? []);
  const items = db.select().from(mergeAssistItems).where(eq(mergeAssistItems.checkoutPath, checkoutPath)).all();
  const failed = activationFailures(db, checkoutPath);
  const fresh: Record<News, Array<{ item: MergeAssistItemRow; mark: string }>> = { saved: [], applied: [], talk: [], review: [] };
  for (const item of items) {
    const news = newsOf(item, sessionId, failed);
    if (news === null) continue;
    const mark = `${versionOf(item)}${news}`;
    if (!given.has(mark) && !inFlight.has(mark)) fresh[news].push({ item, mark });
  }
  const notes: PendingNote[] = [];
  for (const { item, mark } of fresh.saved) {
    notes.push({ content: mergeAssistNote({ kind: "saved", decision: item.candidate.decision, candidateId: item.candidateId }), marks: [mark] });
  }
  for (const { item, mark } of fresh.applied) {
    notes.push({ content: mergeAssistNote({ kind: "absorbed", decision: item.candidate.decision, author: authorOf(item), peer: peerLabel(item) }), marks: [mark] });
  }
  for (const { item, mark } of fresh.talk) {
    const mine = String(evidenceOf(item).mine?.decision);
    notes.push({ content: mergeAssistNote({ kind: "talk", author: authorOf(item), peer: peerLabel(item), theirs: item.candidate.decision, mine }), marks: [mark] });
  }
  if (fresh.review.length > 0) {
    notes.push({ content: mergeAssistNote({ kind: "review", count: fresh.review.length }), marks: fresh.review.map(({ mark }) => mark) });
  }
  return notes;
}

/** Record that `marks` were handed to the session. Only marks of items still in the checkout are kept: a withdrawn decision that comes back is news again. */
export function markNotesDelivered(db: RerailDatabase, sessionId: string, checkoutPath: string, marks: readonly string[], now: number): void {
  if (marks.length === 0) return;
  db.transaction(tx => {
    const key = STATE_KEYS.delivered(sessionId);
    const given = readState(tx, key, deliveredSchema) ?? [];
    const current = tx.select().from(mergeAssistItems).where(eq(mergeAssistItems.checkoutPath, checkoutPath)).all().map(versionOf);
    writeState(tx, key, [...new Set([...given, ...marks])].filter(entry => current.some(version => entry.startsWith(version))), now);
  }, { behavior: "immediate" });
}
