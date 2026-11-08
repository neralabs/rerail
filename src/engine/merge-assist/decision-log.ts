/**
 * The decision log: an append-only Git ref, `refs/rerail/decisions`, one commit per entry, each commit's tree holding
 * the entry as `entry.json`. An entry records a conflict (both sides' refs, commits, intents and anchors), the policy
 * step that decided it with its evidence, the verdict (side a, side b, or escalated), who decided (a policy step or a
 * named person), and the decision that results for the losing side. Written and read with plain plumbing
 * (`hash-object`, `mktree`, `commit-tree`, `update-ref` with the old tip, `rev-list`, `cat-file`); nothing touches the
 * index, the working tree or a branch. Rerail never pushes the ref: a person shares it with
 * `git push <remote> refs/rerail/decisions`.
 */
import { randomBytes } from "node:crypto";
import { z } from "zod";
import { readGitBlobs, runGit } from "../git.js";
import { conflictRecordSchema, type ConflictRecord, otherSide, POLICY_STEPS, SIDES, type SideId, type StepResult } from "./policy.js";

export const DECISION_LOG_REF = "refs/rerail/decisions";
export const DECISION_ENTRY_FORMAT = "rerail.decision/v1";
const ENTRY_FILE = "entry.json";
const MAX_ENTRY_BYTES = 4 * 1024 * 1024;
const LOG_OPTIONS = { maxBuffer: 64 * 1024 * 1024, timeout: 60_000 };
/** Attempts at moving the ref when another writer moved it first. */
const APPEND_ATTEMPTS = 3;

const DECIDING_STEPS = [...POLICY_STEPS, "escalate"] as const;

export const decisionEntrySchema = z.object({
  format: z.literal(DECISION_ENTRY_FORMAT),
  id: z.string().regex(/^[0-9a-f]{32}$/u),
  recordedAt: z.iso.datetime(),
  conflict: conflictRecordSchema,
  /** The step that settled it; `escalate` when it went to a person, whether or not the person has answered. */
  step: z.enum(DECIDING_STEPS),
  reason: z.string().max(4_000),
  /** What the step read: check results, contract breaks, measures, every judge's vote and reasons. */
  evidence: z.record(z.string(), z.unknown()),
  verdict: z.enum([...SIDES, "escalated"]),
  decidedBy: z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("policy"), step: z.enum(DECIDING_STEPS) }).strict(),
    z.object({ kind: z.literal("person"), name: z.string().min(1).max(200) }).strict(),
  ]),
  /**
   * The decision that results: the losing side gets an amendment (its design changes) or a decision to follow (the
   * winner's design, as a rule). `rule` is the rule file once one is written.
   */
  result: z.object({
    kind: z.enum(["amendment", "decision"]),
    side: z.enum(SIDES),
    rule: z.string().min(1).max(500).nullable(),
    text: z.string().min(1).max(2_000),
  }).strict().nullable(),
}).strict()
  .refine(entry => (entry.verdict === "escalated") === (entry.result === null), "an escalated entry has no result, and a verdict has one")
  .refine(entry => entry.result === null || entry.verdict === "escalated" || entry.result.side === otherSide(entry.verdict), "the result goes to the losing side")
  .refine(entry => entry.decidedBy.kind === "person" || (entry.decidedBy.step === entry.step && (entry.step === "escalate") === (entry.verdict === "escalated")), "a policy entry names the step that decided it");

export type DecisionEntry = z.infer<typeof decisionEntrySchema>;

/** A fresh entry id. */
export function newDecisionId(): string {
  return randomBytes(16).toString("hex");
}

export interface LoggedDecision {
  /** The log commit that holds the entry. */
  readonly commit: string;
  readonly entry: DecisionEntry;
}

export type AppendOutcome = { readonly ok: true; readonly commit: string } | { readonly ok: false; readonly reason: string };

function fail(step: string, result: { stderr: string; error?: Error }): AppendOutcome {
  return { ok: false, reason: `git ${step} failed: ${(result.error?.message ?? result.stderr).trim().split("\n")[0] ?? ""}` };
}

/** The log's tip, or null when the ref does not exist yet. */
function logTip(root: string, ref: string): string | null {
  const result = runGit(root, ["rev-parse", "--verify", "--quiet", `${ref}^{commit}`]);
  return result.status === 0 ? result.stdout.trim() : null;
}

/** Identity flags for `commit-tree` in a repository with none configured; none when Git already has one. */
function identityArgs(root: string): string[] {
  return runGit(root, ["var", "GIT_COMMITTER_IDENT"]).status === 0 ? [] : ["-c", "user.name=Rerail", "-c", "user.email=rerail@localhost"];
}

function commitMessage(entry: DecisionEntry): string {
  const outcome = entry.verdict === "escalated" ? "escalated" : `side ${entry.verdict} (${entry.conflict.sides[entry.verdict].ref})`;
  const by = entry.decidedBy.kind === "person" ? entry.decidedBy.name : `policy step ${entry.decidedBy.step}`;
  return `Decision ${entry.id}: ${outcome}, by ${by}\n\n${entry.conflict.sides.a.ref} against ${entry.conflict.sides.b.ref}\nQuestion ${entry.conflict.question}\n`;
}

/**
 * Appends `entry` as one commit on the log, whose parent is the current tip. The ref moves only from the tip this read
 * (`update-ref <new> <old>`), so two writers cannot drop each other's entries; a writer that lost the race builds on
 * the new tip and tries again.
 */
export function appendDecision(root: string, entry: DecisionEntry, ref = DECISION_LOG_REF): AppendOutcome {
  const parsed = decisionEntrySchema.safeParse(entry);
  if (!parsed.success) return { ok: false, reason: `invalid entry: ${parsed.error.issues.map(issue => `${issue.path.join(".")}: ${issue.message}`).join("; ")}` };
  const blob = runGit(root, ["hash-object", "-w", "--stdin"], { input: `${JSON.stringify(parsed.data, null, 2)}\n` });
  if (blob.status !== 0) return fail("hash-object", blob);
  const tree = runGit(root, ["mktree"], { input: `100644 blob ${blob.stdout.trim()}\t${ENTRY_FILE}\n` });
  if (tree.status !== 0) return fail("mktree", tree);
  const identity = identityArgs(root);
  for (let attempt = 0; attempt < APPEND_ATTEMPTS; attempt += 1) {
    const parent = logTip(root, ref);
    const commit = runGit(root, [...identity, "commit-tree", tree.stdout.trim(), ...(parent === null ? [] : ["-p", parent])], { input: commitMessage(parsed.data) });
    if (commit.status !== 0) return fail("commit-tree", commit);
    const sha = commit.stdout.trim();
    const update = runGit(root, ["update-ref", "-m", `rerail: decision ${entry.id}`, ref, sha, parent ?? "0".repeat(sha.length)]);
    if (update.status === 0) return { ok: true, commit: sha };
  }
  return { ok: false, reason: `${ref} kept moving while the entry was written; nothing was appended` };
}

export interface DecisionLog {
  /** Every readable entry, oldest first. */
  readonly entries: LoggedDecision[];
  /** Commits on the log whose entry is missing or invalid, with why; they are skipped. */
  readonly problems: string[];
}

/** The log's entries along its first-parent chain, oldest first; an empty log when the ref does not exist. */
export async function readDecisions(root: string, ref = DECISION_LOG_REF): Promise<DecisionLog> {
  if (logTip(root, ref) === null) return { entries: [], problems: [] };
  const list = runGit(root, ["rev-list", "--first-parent", "--reverse", ref], LOG_OPTIONS);
  if (list.status !== 0) return { entries: [], problems: [`git rev-list ${ref} failed: ${list.stderr.trim()}`] };
  const commits = list.stdout.split("\n").filter(Boolean);
  const blobs = await readGitBlobs(root, commits.map(commit => `${commit}:${ENTRY_FILE}`), { maxBlobBytes: MAX_ENTRY_BYTES, timeout: 60_000 });
  const entries: LoggedDecision[] = [];
  const problems: string[] = [];
  for (const commit of commits) {
    const blob = blobs.get(`${commit}:${ENTRY_FILE}`);
    if (blob === undefined) {
      problems.push(`${commit}: no ${ENTRY_FILE}`);
      continue;
    }
    let value: unknown;
    try {
      value = JSON.parse(blob.toString("utf8"));
    } catch {
      problems.push(`${commit}: ${ENTRY_FILE} is not JSON`);
      continue;
    }
    const parsed = decisionEntrySchema.safeParse(value);
    if (parsed.success) entries.push({ commit, entry: parsed.data });
    else problems.push(`${commit}: ${parsed.error.issues.map(issue => `${issue.path.join(".")}: ${issue.message}`).join("; ")}`);
  }
  return { entries, problems };
}

/** Whether `entry`'s resulting decision is in force at `base`: its rule file is in the base's tree. */
export function decisionInForce(root: string, base: string, entry: DecisionEntry): boolean {
  const rule = entry.result?.rule;
  if (rule === undefined || rule === null || base.startsWith("-") || /[\0\n:]/u.test(base)) return false;
  return runGit(root, ["cat-file", "-e", `${base}:${rule}`]).status === 0;
}

export interface PrecedentResult extends StepResult {
  /** The entry that decided, or the latest one on the question that could not be applied. */
  readonly entry: { readonly id: string; readonly commit: string } | null;
}

/**
 * Precedent: an earlier verdict on the same question decides when one of the sides is the ref it picked or rejected.
 * A verdict whose resulting decision is in force comes first, then a person's, then the latest. An escalation that
 * no one has answered is no precedent.
 */
export function decidePrecedent(record: ConflictRecord, log: readonly LoggedDecision[], inForce: (entry: DecisionEntry) => boolean): PrecedentResult {
  const verdicts = log.filter(({ entry }) => entry.conflict.question === record.question && entry.verdict !== "escalated");
  if (verdicts.length === 0) return { step: "precedent", winner: null, reason: "No earlier verdict on this question.", entry: null };
  const rank = ({ entry }: LoggedDecision) => (inForce(entry) ? 2 : 0) + (entry.decidedBy.kind === "person" ? 1 : 0);
  const ordered = verdicts.map((logged, index) => ({ logged, index })).sort((left, right) => rank(right.logged) - rank(left.logged) || right.index - left.index);
  for (const { logged } of ordered) {
    const { entry } = logged;
    const verdict = entry.verdict as SideId;
    const winnerRef = entry.conflict.sides[verdict].ref;
    const loserRef = entry.conflict.sides[otherSide(verdict)].ref;
    const side = SIDES.find(candidate => record.sides[candidate].ref === winnerRef) ?? SIDES.find(candidate => record.sides[otherSide(candidate)].ref === loserRef);
    if (side === undefined) continue;
    const by = entry.decidedBy.kind === "person" ? entry.decidedBy.name : `policy step ${entry.decidedBy.step}`;
    return {
      step: "precedent",
      winner: side,
      reason: `Decision ${entry.id} (${inForce(entry) ? "in force" : "earlier verdict"}, by ${by}) chose ${winnerRef} over ${loserRef} on this question.`,
      entry: { id: entry.id, commit: logged.commit },
    };
  }
  const latest = verdicts.at(-1)!;
  return {
    step: "precedent",
    winner: null,
    reason: `Earlier verdicts on this question concern other refs (latest: decision ${latest.entry.id}, for ${latest.entry.conflict.sides[latest.entry.verdict as SideId].ref}).`,
    entry: { id: latest.entry.id, commit: latest.commit },
  };
}
