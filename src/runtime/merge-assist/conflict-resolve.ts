/**
 * Settling an in-flight conflict (CONTRACT: Resolve, Precedent). A person keeps their side, takes theirs, writes a new
 * decision, or says it is not a conflict; a verdict in the ledger view on the same dispute settles it on its own, as
 * local state only. A person's verdict writes a ledger entry (`.rerail/decisions/<id>.json`) and the decision this
 * branch now follows as a candidate, active and authored by the person, both into the working tree for the person to
 * commit: Rerail commits nothing. Lane 1 then carries the decision to every agent. Taking theirs on a contradiction follows their decision by reference. A
 * dismissal writes nothing but the conflict's status.
 */
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import type { CheckoutContext } from "../../engine/catalog/repository-model.js";
import {
  CANDIDATE_DIRECTORY,
  CANDIDATE_FORMAT,
  candidateFileName,
  candidateHash,
  cleanDecision,
  MAX_DECISION_LENGTH,
  MAX_SCOPE_PATTERNS,
  scopePatternSchema,
  unusedCandidateId,
  type Candidate,
} from "../../engine/merge-assist/candidate.js";
import { readWorktreeCandidates, writeWorktreeCandidate } from "../../engine/merge-assist/candidate-files.js";
import { refLabel, undimmed } from "../../engine/merge-assist/conflicts.js";
import {
  DECISION_ENTRY_FORMAT,
  type DecisionDraft,
  type DecisionSide,
  type DecisionVerdict,
  type Follows,
  MAX_RESULT_TEXT,
  sealDecisionEntry,
} from "../../engine/merge-assist/decision-entry.js";
import { readWorktreeDecisions, writeWorktreeDecision } from "../../engine/merge-assist/decision-files.js";
import { decidePrecedent, type LedgerView } from "../../engine/merge-assist/ledger.js";
import type { ConflictAction, ConflictResolution } from "../../engine/merge-assist/model.js";
import type { SideId } from "../../engine/merge-assist/conflict-record.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems } from "../../engine/storage/schema.js";
import type { StoredConflict } from "./conflict-store.js";
import { isoSeconds, itemId } from "./state.js";

export interface SettleContext {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  readonly now: () => number;
}

/** The person's answer: what they picked, the decision's text (required for `new`), and their name. */
export interface ConflictAnswer {
  readonly action: ConflictAction;
  readonly text?: string | undefined;
  readonly name: string;
}

/** The entry's verdict per action that decides: this branch's side (a), theirs (b), or a new decision. */
const VERDICTS: Readonly<Record<Exclude<ConflictAction, "dismiss">, DecisionVerdict>> = { mine: "a", theirs: "b", new: "new" };

/** A candidate's scope for the conflict's files: each file that is a valid scope pattern as it is, else its folder. */
function scopeOf(files: readonly string[]): string[] {
  const patterns = files.map(file => {
    if (scopePatternSchema.safeParse(file).success) return file;
    const folder = dirname(file);
    return folder === "." ? "**" : `${folder}/**`;
  });
  const scope = [...new Set(patterns)].slice(0, MAX_SCOPE_PATTERNS);
  return scope.length === 0 ? ["**"] : scope;
}

/** A new active candidate on this branch for `text`, with its rule path; nothing is written yet. */
async function decisionCandidate(
  context: SettleContext,
  input: { text: string; files: readonly string[]; author: string; supersedes: string | null; follows: Follows | null },
): Promise<{ candidate: Candidate; rule: string }> {
  const { candidates } = await readWorktreeCandidates(context.checkout.checkoutPath);
  const taken = new Set(candidates.map(candidate => candidate.id));
  const decision = cleanDecision(input.text).slice(0, MAX_DECISION_LENGTH);
  const supersedes = input.supersedes !== null && taken.has(input.supersedes) ? input.supersedes : null;
  const candidate: Candidate = {
    format: CANDIDATE_FORMAT,
    id: unusedCandidateId(decision, id => taken.has(id)),
    decision,
    generality: "repository",
    scope: scopeOf(input.files),
    author: input.author,
    createdAt: isoSeconds(context.now()),
    state: "active",
    supersedes,
    ...(input.follows === null ? {} : { follows: input.follows }),
  };
  return { candidate, rule: `${CANDIDATE_DIRECTORY}/${candidateFileName(candidate.id)}` };
}

/**
 * Write the candidate to this branch and record it as this checkout's own decision, applied, with the conflict it
 * settles: every session in the checkout is told of it once (lane 1's notes).
 */
async function saveCandidate(context: SettleContext, candidate: Candidate, settles: { conflictId: string; peer: string }): Promise<void> {
  const root = context.checkout.checkoutPath;
  await writeWorktreeCandidate(root, candidate);
  const now = context.now();
  context.database.insert(mergeAssistItems).values({
    id: itemId(root, "own", "", candidate.id),
    checkoutPath: root,
    repositoryKey: context.checkout.repositoryKey,
    candidateId: candidate.id,
    candidateHash: candidateHash(candidate),
    origin: "own",
    peerRef: "",
    trusted: true,
    status: "applied",
    candidate,
    evidence: { resolves: settles.conflictId, peer: settles.peer },
    sessionId: null,
    eventSequence: null,
    createdAt: now,
    updatedAt: now,
  }).onConflictDoNothing().run();
}

/** `stored` settled with `resolution`; resolution resets the dimmer, so a reopened conflict starts afresh. */
function settled(stored: StoredConflict, resolution: ConflictResolution): StoredConflict {
  const status = resolution.action === "dismiss" ? "dismissed" : "resolved";
  return { ...stored, conflict: { ...stored.conflict, ...undimmed(stored.conflict.id), status, resolution } };
}

/**
 * The decision's text: the person's, else the side they picked's: its summary (on an overlap, the subject of the commit
 * that touched the contested file) and, on an overlap, where.
 */
function answerText(stored: StoredConflict, answer: ConflictAnswer): string {
  const given = answer.text?.trim() ?? "";
  if (given !== "") return given;
  const side = answer.action === "theirs" ? stored.conflict.theirs : stored.conflict.mine;
  if (stored.conflict.kind !== "overlap" || side.location === null) return side.summary;
  // A side with no commit touching the file yet has only its edit to show for it, named by its branch: the text travels.
  const uncommitted = stored.files.some(file => side.summary === `edits ${file}`);
  return `${uncommitted ? `The change on ${side.label === "This branch" ? side.ref : side.label}` : side.summary} (${side.location})`;
}

/** One side of the conflict as its ledger entry records it: a = this branch, b = theirs. */
function entrySide(stored: StoredConflict, side: SideId): DecisionSide {
  const shown = side === "a" ? stored.conflict.mine : stored.conflict.theirs;
  const { fingerprint, at } = stored.positions[side];
  return {
    ref: shown.ref,
    label: side === "a" ? refLabel(shown.ref) : shown.label,
    sha: shown.sha,
    author: shown.author === null || shown.author === "" ? null : shown.author.slice(0, 200),
    fingerprint,
    at,
  };
}

/** The ledger entry for `stored`, written to this branch's working tree. */
async function writeEntry(context: SettleContext, stored: StoredConflict, fields: Pick<DecisionDraft, "verdict" | "decidedBy" | "result" | "supersedes">): Promise<string> {
  const entry = sealDecisionEntry({
    format: DECISION_ENTRY_FORMAT,
    question: stored.conflict.question,
    title: stored.conflict.title.slice(0, 300),
    base: stored.record.base,
    sides: { a: entrySide(stored, "a"), b: entrySide(stored, "b") },
    ...fields,
    result: { ...fields.result, text: fields.result.text.slice(0, MAX_RESULT_TEXT) },
    recordedAt: new Date(context.now()).toISOString(),
  });
  await writeWorktreeDecision(context.checkout.checkoutPath, entry);
  return entry.id;
}

/** Their decision, which taking theirs follows by reference: their candidate on a contradiction, none on an overlap. */
function theirDecision(stored: StoredConflict): Follows | null {
  const { theirs } = stored.conflict;
  return stored.conflict.kind === "overlap" || theirs.decisionId === null ? null : { ref: theirs.ref, candidateId: theirs.decisionId };
}

/**
 * A person's verdict: the decision this branch follows as a candidate, and the ledger entry for it. A new verdict
 * replaces this branch's latest entry on the question (`supersedes`), as when the person decides a reopened one again.
 */
async function decide(context: SettleContext, stored: StoredConflict, answer: ConflictAnswer & { action: Exclude<ConflictAction, "dismiss"> }): Promise<StoredConflict> {
  const { conflict } = stored;
  const root = context.checkout.checkoutPath;
  const verdict = VERDICTS[answer.action];
  const text = answerText(stored, answer);
  const follows = answer.action === "theirs" ? theirDecision(stored) : null;
  const planned = await decisionCandidate(context, {
    text, files: stored.files, author: answer.name, supersedes: conflict.kind === "overlap" ? null : conflict.mine.decisionId, follows,
  });
  const previous = (await readWorktreeDecisions(root)).entries
    .filter(entry => entry.question === conflict.question)
    .sort((left, right) => left.recordedAt.localeCompare(right.recordedAt))
    .at(-1);
  const logId = await writeEntry(context, stored, {
    verdict,
    decidedBy: { kind: "person", name: answer.name.slice(0, 200) },
    result: { candidateId: planned.candidate.id, kind: verdict === "new" ? "amendment" : "decision", text, rule: planned.rule, follows },
    supersedes: previous?.id ?? null,
  });
  await saveCandidate(context, planned.candidate, { conflictId: conflict.id, peer: conflict.theirs.label });
  return settled(stored, { action: answer.action, by: "person", name: answer.name, logId, rule: planned.rule, text, at: isoSeconds(context.now()) });
}

/** Settle `stored` with the person's answer. The caller checks the conflict is open and that `new` carries text. */
export async function answerConflict(context: SettleContext, stored: StoredConflict, answer: ConflictAnswer): Promise<StoredConflict> {
  if (answer.action !== "dismiss") return await decide(context, stored, { ...answer, action: answer.action });
  return settled(stored, { action: "dismiss", by: "person", name: answer.name, logId: null, rule: null, text: null, at: isoSeconds(context.now()) });
}

/**
 * `stored` settled by a verdict in the ledger view on the same dispute (`decidePrecedent`): local state naming that
 * entry, with no file written to the working tree. When a verdict on the question applies to another dispute, the
 * conflict stays open for the person with it as context ("Earlier: … chose … on …").
 */
function byPrecedent(context: SettleContext, stored: StoredConflict, ledger: Pick<LedgerView, "items">): StoredConflict {
  if (stored.conflict.kind === "verdicts") return stored;
  const found = decidePrecedent({
    question: stored.conflict.question,
    sides: {
      a: { ref: stored.conflict.mine.ref, fingerprint: stored.positions.a.fingerprint },
      b: { ref: stored.conflict.theirs.ref, fingerprint: stored.positions.b.fingerprint },
    },
  }, ledger);
  const earlier = ledger.items.find(item => item.entry.id === found.entry?.id);
  if (earlier === undefined) return stored;
  if (found.winner === null) return { ...stored, conflict: { ...stored.conflict, evidence: [...stored.conflict.evidence, found.reason] } };
  const { result } = earlier.entry;
  // The verdict's own decision file, where this checkout has it; Rerail writes none for precedent.
  const rule = result.rule !== null && existsSync(join(context.checkout.checkoutPath, result.rule)) ? result.rule : null;
  const action = found.winner === "a" ? "mine" : "theirs";
  return settled(stored, { action, by: "precedent", name: "precedent", logId: earlier.entry.id, rule, text: result.text, at: isoSeconds(context.now()) });
}

/** Each of `fresh` (new open conflicts) settled by precedent where a verdict in the ledger view decides it. */
export function settleByPrecedent(context: SettleContext, fresh: readonly StoredConflict[], ledger: Pick<LedgerView, "items">): StoredConflict[] {
  return fresh.map(stored => byPrecedent(context, stored, ledger));
}
