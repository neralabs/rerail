/**
 * The checkout's side of the decision ledger: what it saw of the ledger (`ledger:<checkout path>`), which the view's
 * void status needs and which caches what Git reads, refreshed with the merge check at every scan and detection pass
 * and after a verdict; status reads show the view it stored; and the latest text of each of
 * this branch's decisions that follows another branch's by reference, which delivery hands the agent.
 */
import { and, eq } from "drizzle-orm";
import { createHash } from "node:crypto";
import {
  EMPTY_LEDGER_MEMORY,
  followedDecision,
  ledgerMemorySchema,
  type LedgerMemory,
  type LedgerRef,
  type LedgerView,
  type LedgerPlace,
  readLedger,
  storedLedger,
} from "../../engine/merge-assist/ledger.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems } from "../../engine/storage/schema.js";
import { readConflicts } from "./conflict-store.js";
import type { PullRequestSummary } from "./github.js";
import { STATE_KEYS, readState, writeState, type StateQueries } from "./state.js";

export function readLedgerMemory(db: StateQueries, checkoutPath: string): LedgerMemory {
  return readState(db, STATE_KEYS.ledger(checkoutPath), ledgerMemorySchema) ?? EMPTY_LEDGER_MEMORY;
}

/** Open pull requests as the ledger reads them: their heads. */
export function pullLedgerRefs(pulls: readonly PullRequestSummary[]): LedgerRef[] {
  return pulls.map(pull => ({ ref: `refs/pull/${pull.number}/head`, label: `PR #${pull.number}`, sha: pull.headCommit }));
}

/**
 * The ledger view now, with the merge check: every entry this checkout saw before is placed again, and one whose ref
 * merged or went away without it is void. What it saw is then kept for the next pass. `pulls` null: the pull request
 * heads the last refresh read.
 */
export async function refreshLedger(db: RerailDatabase, checkoutPath: string, pulls: readonly PullRequestSummary[] | null, now: number): Promise<LedgerView> {
  const memory = readLedgerMemory(db, checkoutPath);
  const view = await readLedger(checkoutPath, { memory, pulls: pulls === null ? memory.pulls : pullLedgerRefs(pulls) });
  writeState(db, STATE_KEYS.ledger(checkoutPath), view.memory, now);
  return view;
}

/** The ledger view the last refresh stored, for a status read: no Git work. Empty before the first refresh. */
export function viewLedger(db: StateQueries, checkoutPath: string): Pick<LedgerView, "items" | "problems"> {
  return storedLedger(readLedgerMemory(db, checkoutPath));
}

/** A short digest of a followed decision's text, which marks its delivery so a change is news again. */
export function followedMark(text: string): string {
  return createHash("sha256").update(text).digest("hex").slice(0, 8);
}

/**
 * Resolve each of this branch's own decisions that follows another branch's (`follows`) to that decision's latest
 * text, kept on its item as `followed` for delivery; one Rerail cannot read keeps the text it had.
 */
export async function refreshFollowed(db: RerailDatabase, checkoutPath: string, now: number): Promise<void> {
  const items = db.select().from(mergeAssistItems).where(and(eq(mergeAssistItems.checkoutPath, checkoutPath), eq(mergeAssistItems.origin, "own"))).all();
  for (const item of items) {
    const follows = item.candidate.follows;
    if (follows === undefined) continue;
    const text = await followedDecision(checkoutPath, follows);
    const evidence = item.evidence as { followed?: unknown };
    if (text === null || text === evidence.followed) continue;
    db.update(mergeAssistItems).set({ evidence: { ...evidence, followed: text }, updatedAt: now }).where(eq(mergeAssistItems.id, item.id)).run();
  }
}

/** A decision that is the result of a ledger entry not in force: where the entry is read, and its question's state. */
export interface ProvisionalResult {
  readonly question: string;
  /** `branch`: this branch's own verdict; `live`/`gone`: another branch's. */
  readonly place: LedgerPlace;
  /** Whether a `verdicts` conflict on the question is open here: then neither side's decision is guidance. */
  readonly contested: boolean;
}

/**
 * The decisions (candidate ids) that are results of provisional or reopened ledger entries, from the stored view: they
 * reach agents only as information, and not at all while their question has an open `verdicts` conflict. None while
 * the experimental decision ledger is off. Reads only.
 */
export function provisionalResults(db: StateQueries, checkoutPath: string, decisionLedger: boolean): Map<string, ProvisionalResult> {
  if (!decisionLedger) return new Map();
  const contested = new Set(readConflicts(db, checkoutPath)
    .filter(({ conflict }) => conflict.status === "open" && conflict.kind === "verdicts")
    .map(({ conflict }) => conflict.question));
  const results = new Map<string, ProvisionalResult>();
  for (const item of viewLedger(db, checkoutPath).items) {
    const id = item.entry.result.candidateId;
    if (id === null || (item.status !== "provisional" && item.status !== "reopened")) continue;
    const known = results.get(id);
    results.set(id, { question: item.entry.question, place: known?.place === "branch" ? "branch" : item.place, contested: (known?.contested ?? false) || contested.has(item.entry.question) });
  }
  return results;
}
