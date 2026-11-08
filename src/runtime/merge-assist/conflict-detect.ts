/**
 * Finding the checkout's in-flight conflicts (CONTRACT: Detection). Mine is this checkout's working changes, committed
 * and not, against the merge base of HEAD with the base branch. Theirs are the live branches: local branches,
 * remote-tracking branches, and open pull requests whose head this repository holds. An overlap is one conflict per
 * (their ref, file); a Talk finding (a teammate's decision contradicting one of this branch's) is one conflict too.
 * New conflicts an earlier verdict answers are settled by precedent. Reads Git only; never merges.
 */
import { basename } from "node:path";
import { and, eq, ne } from "drizzle-orm";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { gitText } from "../../engine/git.js";
import { baseBranch, commitExists, currentBranch, isAncestor, localBranches, mergeBase } from "../../engine/merge-assist/branch-git.js";
import { readWorktreeCandidates } from "../../engine/merge-assist/candidate-files.js";
import { CANDIDATE_DIRECTORY, candidateFileName } from "../../engine/merge-assist/candidate.js";
import { conflictId, conflictLevel, conflictRecord } from "../../engine/merge-assist/conflicts.js";
import { readDecisions } from "../../engine/merge-assist/decision-log.js";
import type { Conflict, ConflictSide } from "../../engine/merge-assist/model.js";
import { ADJACENT_LINES, clipLines, type GitConflict, HUNK_LINES, identify, ownBase, refHunks, type RefHunk, workspaceRanges } from "../../engine/merge-assist/overlap.js";
import { questionOf } from "../../engine/merge-assist/conflict-record.js";
import { mergeAssistItems, type MergeAssistItemRow } from "../../engine/storage/schema.js";
import { type SettleContext, settleByPrecedent } from "./conflict-resolve.js";
import { mergeDetected, newConflicts, readConflicts, type StoredConflict, writeConflicts } from "./conflict-store.js";
import type { PullRequestSummary } from "./github.js";
import { personName } from "./people.js";
import { isoSeconds } from "./state.js";

export interface DetectContext extends SettleContext {
  /** The open pull requests, as last listed; none without `gh`. */
  readonly pulls: readonly PullRequestSummary[];
}

/** This checkout's side: where its work starts, its head and branch, the person, and its changed hunks per file. */
interface MineView {
  readonly head: string;
  readonly branch: string | null;
  readonly base: { readonly ref: string; readonly name: string };
  /** The merge base of HEAD with the base branch. */
  readonly start: string;
  readonly person: string;
  readonly ranges: ReadonlyMap<string, readonly RefHunk[]>;
}

/** A live branch or pull request as the person knows it. */
interface LiveRef {
  readonly ref: string;
  readonly label: string;
  readonly sha: string;
  readonly author: string | null;
  readonly url: string | null;
  readonly title: string | null;
}

function mineView(root: string): MineView | null {
  const head = gitText(root, ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
  const base = baseBranch(root);
  const start = head === undefined || base === null ? null : mergeBase(root, base.ref, head);
  if (head === undefined || base === null || start === null) return null;
  // Rerail's own candidate files are decisions, not work that overlaps.
  const ranges = new Map([...workspaceRanges(root, start)].filter(([path]) => !path.startsWith(`${CANDIDATE_DIRECTORY}/`)));
  return { head, branch: currentBranch(root), base, start, person: personName(root), ranges };
}

/** Open pull requests whose head this repository holds and this branch does not already contain. */
function pullRefs(root: string, view: MineView, pulls: readonly PullRequestSummary[]): LiveRef[] {
  return pulls
    .filter(pull => pull.headRefName !== view.branch && commitExists(root, pull.headCommit) && !isAncestor(root, pull.headCommit, view.head))
    .map(pull => ({ ref: `refs/pull/${pull.number}/head`, label: `PR #${pull.number}`, sha: pull.headCommit, author: pull.author, url: pull.url, title: pull.title }));
}

/** Remote-tracking branches: name and tip, without `<remote>/HEAD`. */
function remoteBranches(root: string): Array<{ name: string; commit: string }> {
  const output = gitText(root, ["for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)", "refs/remotes"]) ?? "";
  return output.split("\n").map(line => line.split("\0")).filter(([name, , symref]) => name !== undefined && name !== "" && symref === "")
    .map(([name, commit]) => ({ name: name!, commit: commit ?? "" }));
}

/** Local and remote-tracking branches with work of their own: not this branch, its upstream or the base, not merged. */
function branchRefs(root: string, view: MineView, taken: Set<string>): LiveRef[] {
  const upstream = gitText(root, ["rev-parse", "--symbolic-full-name", "@{upstream}"]);
  const skip = new Set([view.base.ref, upstream, view.branch === null ? undefined : `refs/heads/${view.branch}`]);
  const candidates = [
    ...localBranches(root).map(branch => ({ name: `refs/heads/${branch.name}`, commit: branch.commit })),
    ...remoteBranches(root),
  ];
  const refs: LiveRef[] = [];
  for (const { name, commit } of candidates) {
    if (skip.has(name) || taken.has(commit) || isAncestor(root, commit, view.base.ref) || isAncestor(root, commit, view.head)) continue;
    taken.add(commit);
    refs.push({ ref: name.replace(/^refs\/(?:heads|remotes)\//u, ""), label: name.replace(/^refs\/(?:heads|remotes)\//u, ""), sha: commit, author: null, url: null, title: null });
  }
  return refs;
}

/** A ref's tip author and commit time, as `git log` gives them. */
function tipOf(root: string, sha: string): { author: string | null; at: string | null } {
  const [author, at] = (gitText(root, ["log", "-1", "--format=%an%x00%cI", sha, "--"]) ?? "").split("\0");
  return { author: author === undefined || author === "" ? null : author, at: at === undefined || at === "" ? null : at };
}

/** The symbol git names in a hunk's header (`@@ … @@ fn compile(…) {`), without its opening brace, if any. */
function hunkSymbol(hunks: readonly RefHunk[]): string | null {
  const context = /^@@ [^@]* @@ ?(.*)$/u.exec(hunks[0]?.text.split("\n")[0] ?? "")?.[1]?.replace(/\s*\{\s*$/u, "").trim() ?? "";
  return context === "" ? null : context.slice(0, 60);
}

const lineOf = (hunks: readonly { lo: number }[]): number => Math.max(1, Math.floor(hunks[0]?.lo ?? 1));
const hunkText = (hunks: readonly RefHunk[]): string => clipLines(hunks.flatMap(hunk => hunk.text.split("\n")), HUNK_LINES);

/** Their side of an overlap in `file`. */
function theirSide(root: string, ref: LiveRef, own: string, overlap: GitConflict): ConflictSide {
  const tip = tipOf(root, ref.sha);
  const subject = gitText(root, ["log", "--no-merges", "-1", "--format=%s", `${own}..${ref.sha}`, "--"]);
  return {
    label: ref.label, ref: ref.ref, sha: ref.sha, author: ref.author ?? tip.author, pushedAt: tip.at, url: ref.url,
    summary: ref.title ?? (subject === undefined || subject === "" ? `edits ${overlap.file}` : subject),
    location: `${overlap.file}:${lineOf(overlap.theirs)}`, hunk: hunkText(overlap.theirs), decisionId: null,
  };
}

/** This branch's side of an overlap in `file`. */
function mySide(view: MineView, file: string, ours: readonly RefHunk[]): ConflictSide {
  return {
    label: "This branch", ref: view.branch ?? view.head, sha: view.head, author: view.person, pushedAt: null, url: null,
    summary: `edits ${file}`, location: `${file}:${lineOf(ours)}`, hunk: hunkText(ours), decisionId: null,
  };
}

/** One overlap with one ref as a stored conflict: the status's view of it, its file, and its log record. */
function overlapConflict(context: DetectContext, view: MineView, ref: LiveRef, own: string, overlap: GitConflict & { ours: readonly RefHunk[] }): StoredConflict {
  const { file } = overlap;
  // The question is the file and the base lines their overlapping change covers: another overlap in the same file, or
  // another change of theirs, is another question, so an earlier verdict on one never settles the other.
  const lines = overlap.theirs.map(hunk => `${file}:${Math.floor(hunk.lo)}-${Math.floor(hunk.hi)}`);
  const question = questionOf({ files: [file], symbols: lines }, { files: [file], symbols: lines });
  const mine = mySide(view, file, overlap.ours);
  const theirs = theirSide(context.checkout.checkoutPath, ref, own, overlap);
  const symbol = hunkSymbol(overlap.theirs);
  const conflict: Conflict = {
    id: conflictId(question, ref.ref), kind: "overlap", status: "open", level: conflictLevel("overlap", [file]), question,
    title: symbol === null ? basename(file) : `${basename(file)} · ${symbol}`, mine, theirs,
    evidence: [
      `Both sides change ${file} within ${ADJACENT_LINES} lines of each other: yours near line ${lineOf(overlap.ours)}, theirs near line ${lineOf(overlap.theirs)} of the base.`,
      `Only ${ref.label}'s own work since it left ${view.base.name} counts; Rerail merged nothing to find this.`,
    ],
    detectedAt: isoSeconds(context.now()), resolution: null,
  };
  const record = conflictRecord({ repository: context.checkout.repositoryKey, base: view.start, question }, { a: { side: mine, files: [file] }, b: { side: theirs, files: [file] } });
  return { conflict, files: [file], record };
}

/** The overlaps of this branch's changes with each live ref. */
function overlapConflicts(context: DetectContext, view: MineView): StoredConflict[] {
  if (view.ranges.size === 0) return [];
  const root = context.checkout.checkoutPath;
  const pulls = pullRefs(root, view, context.pulls);
  const refs = [...pulls, ...branchRefs(root, view, new Set(pulls.map(pull => pull.sha)))];
  return refs.flatMap(ref => {
    const own = ownBase(root, ref.sha, view.base.ref, view.start);
    if (own === null) return [];
    const files = refHunks(root, view.start, own, ref.sha, [...view.ranges.keys()]);
    return identify(view.ranges, [{ ref: ref.ref, files }]).map(overlap => overlapConflict(context, view, ref, own, overlap));
  });
}

interface TalkEvidence {
  readonly mine?: { id?: unknown; decision?: unknown };
  readonly judgment?: { explanation?: unknown } | null;
  readonly branch?: unknown;
}

/** A Talk finding's other side: its ref and how the person knows it. */
function talkRef(item: MergeAssistItemRow): { ref: string; label: string } {
  if (item.peerKind === "pull-request") return { ref: `refs/pull/${item.peerNumber}/head`, label: `PR #${item.peerNumber}` };
  const branch = (item.evidence as TalkEvidence).branch;
  const name = typeof branch === "string" ? branch : item.peerRef;
  return { ref: name, label: name };
}

/** A decision's side of a contradiction. */
function decisionSide(side: Pick<ConflictSide, "label" | "ref" | "sha" | "author" | "url">, decision: { id: string; decision: string }): ConflictSide {
  return { ...side, pushedAt: null, summary: decision.decision, location: `${CANDIDATE_DIRECTORY}/${candidateFileName(decision.id)}`, hunk: null, decisionId: decision.id };
}

/** One Talk finding as a stored conflict, or null when it lacks this branch's decision or their commit. */
function talkConflict(context: DetectContext, view: MineView, item: MergeAssistItemRow, scopes: ReadonlyMap<string, readonly string[]>): StoredConflict | null {
  const evidence = item.evidence as TalkEvidence;
  const mineId = evidence.mine?.id;
  const mineDecision = evidence.mine?.decision;
  if (typeof mineId !== "string" || typeof mineDecision !== "string" || item.peerCommit === null) return null;
  const peer = talkRef(item);
  const mineFiles = [...(scopes.get(mineId) ?? [])];
  // Their decision is the question, not this branch's version of its answer: a verdict that amends this branch's
  // decision leaves the same contradiction settled instead of raising a new one.
  const symbols = [item.candidateId];
  const question = questionOf({ files: mineFiles, symbols }, { files: item.candidate.scope, symbols });
  const author = item.peerKind === "pull-request" && item.peerAuthor !== null ? item.peerAuthor : item.candidate.author;
  const mine = decisionSide({ label: "This branch", ref: view.branch ?? view.head, sha: view.head, author: view.person, url: null }, { id: mineId, decision: mineDecision });
  const theirs = decisionSide({ label: peer.label, ref: peer.ref, sha: item.peerCommit, author, url: item.peerUrl }, item.candidate);
  const explanation = typeof evidence.judgment?.explanation === "string" ? evidence.judgment.explanation : "Your agent's compare call found they contradict.";
  const conflict: Conflict = {
    id: conflictId(question, peer.ref), kind: "decisions", status: "open", level: conflictLevel("decisions", []), question,
    title: item.candidate.decision.length > 80 ? `${item.candidate.decision.slice(0, 79).trimEnd()}…` : item.candidate.decision,
    mine, theirs,
    evidence: [explanation, "Both decisions are in force on their branches, and no decision in force answers which one holds."],
    detectedAt: isoSeconds(context.now()), resolution: null,
  };
  const files = [...new Set([...mineFiles, ...item.candidate.scope])];
  const record = conflictRecord({ repository: context.checkout.repositoryKey, base: view.start, question }, { a: { side: mine, files: mineFiles }, b: { side: theirs, files: item.candidate.scope } });
  return { conflict, files, record };
}

/** Every Talk finding the person has not dismissed, as a conflict of kind `decisions`. */
async function decisionConflicts(context: DetectContext, view: MineView): Promise<StoredConflict[]> {
  const root = context.checkout.checkoutPath;
  const items = context.database.select().from(mergeAssistItems).where(and(
    eq(mergeAssistItems.checkoutPath, root), eq(mergeAssistItems.origin, "peer"), eq(mergeAssistItems.tier, "talk"), ne(mergeAssistItems.status, "dismissed"),
  )).all();
  if (items.length === 0) return [];
  const scopes = new Map((await readWorktreeCandidates(root)).candidates.map(candidate => [candidate.id, candidate.scope]));
  return items.flatMap(item => talkConflict(context, view, item, scopes) ?? []);
}

/**
 * Find the checkout's conflicts and store them with the ones already settled; new ones an earlier verdict answers are
 * settled by precedent. Returns whether precedent wrote a decision on this branch, which activation then applies.
 * A detection that finishes after Merge Assist was turned off stores nothing.
 */
export async function detectConflicts(context: DetectContext): Promise<boolean> {
  const root = context.checkout.checkoutPath;
  const view = mineView(root);
  if (view === null) return false;
  const detected = [...overlapConflicts(context, view), ...await decisionConflicts(context, view)];
  const stored = readConflicts(context.database, root);
  const merged = mergeDetected(stored, detected);
  const fresh = newConflicts(stored, merged);
  const log = fresh.length === 0 ? [] : (await readDecisions(root)).entries;
  const settled = new Map((await settleByPrecedent(context, fresh, log)).map(entry => [entry.conflict.id, entry]));
  const result = merged.map(entry => settled.get(entry.conflict.id) ?? entry);
  context.database.transaction(tx => {
    if (!mergeAssistEnabled(tx, context.checkout.repositoryKey)) return;
    // A person's answer given while this ran stands.
    writeConflicts(tx, root, mergeDetected(readConflicts(tx, root), result), context.now());
  }, { behavior: "immediate" });
  return [...settled.values()].some(entry => entry.conflict.resolution?.by === "precedent");
}
