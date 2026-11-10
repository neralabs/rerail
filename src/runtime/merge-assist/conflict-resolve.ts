/**
 * Settling an in-flight conflict with a verdict (CONTRACT: Resolve), the experimental verdicts handler's work
 * (`experiment-handlers.ts`): a person keeps their side, takes theirs or writes a new decision. A person's verdict writes
 * the decision this branch now follows as a candidate, active and authored by the person, into the working tree for the
 * person to commit (`.rerail/candidates/`): Rerail commits nothing. The conflict is settled as local state in this
 * checkout, and lane 1 then carries the decision to every agent. A dismissal is not a verdict: `conflict-store.ts`
 * records it.
 */
import { dirname } from "node:path";
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
import type { ConflictAction } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems } from "../../engine/storage/schema.js";
import { settledConflict, type StoredConflict } from "./conflict-store.js";
import { isoSeconds, itemId } from "./state.js";

export interface SettleContext {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  readonly now: () => number;
}

/** The person's verdict: what they picked, the decision's text (required for `new`), and their name. */
export interface ConflictAnswer {
  readonly action: Exclude<ConflictAction, "dismiss">;
  readonly text?: string | undefined;
  readonly name: string;
}

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

/**
 * A person's verdict on an open conflict: the decision this branch follows as a candidate, and the conflict settled as
 * local state. The caller checks the conflict is open and that `new` carries text.
 */
export async function decideConflict(context: SettleContext, stored: StoredConflict, answer: ConflictAnswer): Promise<StoredConflict> {
  const { conflict } = stored;
  const text = answerText(stored, answer);
  const planned = await decisionCandidate(context, {
    text, files: stored.files, author: answer.name, supersedes: conflict.kind === "overlap" ? null : conflict.mine.decisionId,
  });
  await saveCandidate(context, planned.candidate, { conflictId: conflict.id, peer: conflict.theirs.label });
  return settledConflict(stored, { action: answer.action, name: answer.name, rule: planned.rule, text, at: isoSeconds(context.now()) });
}
