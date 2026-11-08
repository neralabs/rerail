/**
 * Settling an in-flight conflict (CONTRACT: Resolve, Precedent). A person keeps their side, takes theirs, writes a new
 * decision, or says it is not a conflict; an earlier verdict on the same question settles it on its own. Every verdict
 * appends one entry to the decision log (`refs/rerail/decisions`) and, where this branch has to follow a decision,
 * writes it as a candidate on this branch, active and authored by the person, which lane 1 then carries to every
 * agent. A dismissal writes nothing but the conflict's status.
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
import {
  appendDecision,
  decidePrecedent,
  type DecisionEntry,
  decisionInForce,
  DECISION_ENTRY_FORMAT,
  type LoggedDecision,
  newDecisionId,
  personDecisionEntry,
} from "../../engine/merge-assist/decision-log.js";
import type { ConflictAction, ConflictResolution } from "../../engine/merge-assist/model.js";
import { otherSide, type SideId } from "../../engine/merge-assist/conflict-record.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems } from "../../engine/storage/schema.js";
import type { StoredConflict } from "./conflict-store.js";
import { isoSeconds, itemId } from "./state.js";

/** The longest text a log entry's result keeps. */
const MAX_RESULT_TEXT = 2_000;

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

/** A side's verdict and what the losing side gets, per action that decides. */
const VERDICTS: Readonly<Record<Exclude<ConflictAction, "dismiss">, { verdict: SideId; kind: "decision" | "amendment" }>> = {
  mine: { verdict: "a", kind: "decision" },
  theirs: { verdict: "b", kind: "decision" },
  new: { verdict: "a", kind: "amendment" },
};

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
  input: { text: string; files: readonly string[]; author: string; supersedes: string | null },
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

function settled(stored: StoredConflict, resolution: ConflictResolution): StoredConflict {
  return { ...stored, conflict: { ...stored.conflict, status: resolution.action === "dismiss" ? "dismissed" : "resolved", resolution } };
}

/** The decision's text: the person's, else the side they picked's summary. */
function answerText(stored: StoredConflict, answer: ConflictAnswer): string {
  const given = answer.text?.trim() ?? "";
  if (given !== "") return given;
  return answer.action === "theirs" ? stored.conflict.theirs.summary : stored.conflict.mine.summary;
}

function appended(root: string, entry: DecisionEntry): void {
  const outcome = appendDecision(root, entry);
  if (!outcome.ok) throw new Error(`Rerail could not append to the decision log: ${outcome.reason}`);
}

/** A person's verdict: the log entry, then this branch's candidate for it. */
async function decide(context: SettleContext, stored: StoredConflict, answer: ConflictAnswer & { action: Exclude<ConflictAction, "dismiss"> }): Promise<StoredConflict> {
  const { conflict } = stored;
  const { verdict, kind } = VERDICTS[answer.action];
  const text = answerText(stored, answer);
  const supersedes = conflict.kind === "decisions" ? conflict.mine.decisionId : null;
  const planned = await decisionCandidate(context, { text, files: stored.files, author: answer.name, supersedes });
  const picked = answer.action === "new" ? "a new decision" : `${verdict === "a" ? conflict.mine.label : conflict.theirs.label}'s side`;
  const entry = personDecisionEntry(stored.record, {
    verdict,
    name: answer.name,
    reason: `${answer.name} chose ${picked} on the Merge Assist page.`,
    result: { kind, rule: planned.rule, text: text.slice(0, MAX_RESULT_TEXT) },
  }, new Date(context.now()));
  appended(context.checkout.checkoutPath, entry);
  await saveCandidate(context, planned.candidate, { conflictId: conflict.id, peer: conflict.theirs.label });
  return settled(stored, { action: answer.action, by: "person", name: answer.name, logId: entry.id, rule: planned.rule, text, at: isoSeconds(context.now()) });
}

/** Settle `stored` with the person's answer. The caller checks the conflict is open and that `new` carries text. */
export async function answerConflict(context: SettleContext, stored: StoredConflict, answer: ConflictAnswer): Promise<StoredConflict> {
  if (answer.action !== "dismiss") return await decide(context, stored, { ...answer, action: answer.action });
  return settled(stored, { action: "dismiss", by: "person", name: answer.name, logId: null, rule: null, text: null, at: isoSeconds(context.now()) });
}

/** The policy entry recording that precedent settled `stored` for `verdict`, after `earlier`. */
function precedentEntry(stored: StoredConflict, verdict: SideId, earlier: DecisionEntry, result: { rule: string | null; text: string }, now: number): DecisionEntry {
  return {
    format: DECISION_ENTRY_FORMAT,
    id: newDecisionId(),
    recordedAt: new Date(now).toISOString(),
    conflict: stored.record,
    step: "precedent",
    reason: `Decision ${earlier.id} already chose ${stored.record.sides[verdict].ref} on this question.`,
    evidence: { precedent: { id: earlier.id } },
    verdict,
    decidedBy: { kind: "policy", step: "precedent" },
    result: { kind: "decision", side: otherSide(verdict), rule: result.rule, text: result.text.slice(0, MAX_RESULT_TEXT) },
  };
}

/**
 * The decision this branch follows after precedent picked theirs: the earlier entry's rule file when this checkout
 * has it, else a candidate written now with the earlier decision's text.
 */
async function followTheirs(context: SettleContext, stored: StoredConflict, text: string, earlier: DecisionEntry): Promise<string> {
  const rule = earlier.result?.rule ?? null;
  if (rule !== null && existsSync(join(context.checkout.checkoutPath, rule))) return rule;
  const planned = await decisionCandidate(context, { text, files: stored.files, author: "Rerail", supersedes: null });
  await saveCandidate(context, planned.candidate, { conflictId: stored.conflict.id, peer: stored.conflict.theirs.label });
  return planned.rule;
}

/** `stored` settled by an earlier verdict on its question, or unchanged when there is none for its sides. */
async function byPrecedent(context: SettleContext, stored: StoredConflict, log: readonly LoggedDecision[]): Promise<StoredConflict> {
  const root = context.checkout.checkoutPath;
  const found = decidePrecedent(stored.record, log, entry => decisionInForce(root, stored.record.base, entry));
  const earlier = log.find(logged => logged.entry.id === found.entry?.id)?.entry;
  if (found.winner === null || earlier === undefined) return stored;
  const side = found.winner === "a" ? stored.conflict.mine : stored.conflict.theirs;
  const text = earlier.result?.text ?? side.summary;
  const rule = found.winner === "b" ? await followTheirs(context, stored, text, earlier) : earlier.result?.rule ?? null;
  const entry = precedentEntry(stored, found.winner, earlier, { rule, text }, context.now());
  appended(root, entry);
  const action = found.winner === "a" ? "mine" : "theirs";
  return settled(stored, { action, by: "precedent", name: "precedent", logId: entry.id, rule, text, at: isoSeconds(context.now()) });
}

/** Each of `fresh` (new open conflicts) settled by precedent where an earlier verdict decides it. */
export async function settleByPrecedent(context: SettleContext, fresh: readonly StoredConflict[], log: readonly LoggedDecision[]): Promise<StoredConflict[]> {
  const settledList: StoredConflict[] = [];
  for (const stored of fresh) settledList.push(await byPrecedent(context, stored, log));
  return settledList;
}
