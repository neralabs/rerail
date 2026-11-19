/**
 * Detecting the checkout's in-flight conflicts (docs/merge-assist-v1-build.md). Mine is this checkout's working
 * changes, committed and not, against the merge base of HEAD with the base branch. Theirs are the work in flight: open
 * pull requests whose head this repository holds, and branches checked out in another worktree of this repository (or
 * being rebased there); when the open pull requests could not be read (no GitHub remote, no `gh`, signed out, a failed
 * listing), the {@link REMOTE_BRANCH_LIMIT} most recently committed remote-tracking branches stand in for them. A branch
 * (not a pull request) last committed over {@link STALE_BRANCH_DAYS} days ago is left out, as are this branch's own (its
 * local and remote-tracking copies, its upstream, its pull request, from a fork too; the branch being rebased while HEAD
 * is detached) and work the base branch already holds. A branch is one peer however it arrives
 * (a pull request, a local or a remote-tracking branch of the same name), so its conflicts keep their ids. An overlap is one conflict per (their
 * branch, file); one whose every file is a lockfile, a changelog, generated output or under `docs/` is quiet. Reads Git
 * only, without blocking, a few processes at a time; writes no file; never merges.
 */
import { access } from "node:fs/promises";
import { basename } from "node:path";
import type { CheckoutContext } from "../../engine/catalog/repository-model.js";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import type { GitReads } from "../../engine/git.js";
import {
  baseBranchAsync,
  branchBeingRebased,
  currentBranchAsync,
  gitReadAsync,
  isAncestorAsync,
  mergeBaseAsync,
  readsStoppedBy,
  resolveCommit,
  upstreamOf,
} from "../../engine/merge-assist/branch-git.js";
import { sideName } from "../../engine/merge-assist/conflict-note.js";
import { conflictId, overlapQuestion, quietFiles, shortConflictId } from "../../engine/merge-assist/conflicts.js";
import type { Conflict, ConflictSide } from "../../engine/merge-assist/model.js";
import { ADJACENT_LINES, clipLines, type GitConflict, HUNK_LINES, identify, OverlapReader, type RefHunk, workspaceRanges } from "../../engine/merge-assist/overlap.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { conflictEvents, type ConflictFire } from "./conflict-hook.js";
import { mergeDetected, readConflicts, type StoredConflict, writeConflicts } from "./conflict-store.js";
import { PULL_REQUEST_LIMIT, type PullRequestSummary } from "./github.js";
import { personName } from "./people.js";
import { isoSeconds, type StateQueries } from "./state.js";

/** What a pass that could compare nothing (no base branch, no shared history) says of the open conflicts it kept. */
export const UNCOMPARED_KEPT_PROBLEM = "The open conflicts found before stay as they were until this branch can be compared again.";

/** Why a checkout with no base branch has no conflicts. */
export const NO_BASE_PROBLEM = "No base branch found (origin/HEAD, main or master), so Rerail cannot tell this branch's work from the base's.";

/** Why a checkout whose base branch shares no history with it (an orphan branch, a shallow clone) has no conflicts. */
export function noSharedHistoryProblem(base: string): string {
  return `${base} shares no history with this branch, so Rerail cannot tell this branch's work from the base's.`;
}

/**
 * A branch whose tip commit is older than this many days, by its committer date, is not compared unless it has an open
 * pull request: work left behind, not in flight.
 */
export const STALE_BRANCH_DAYS = 14;
const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * The most remote-tracking branches a pass compares while open pull requests are unread: the most recently committed,
 * as many as a pull request listing holds.
 */
export const REMOTE_BRANCH_LIMIT = PULL_REQUEST_LIMIT;

/** How many Git processes a detection pass runs at once. */
const GIT_CONCURRENCY = 6;
const LIST_OPTIONS = { maxBuffer: 16 * 1024 * 1024 };

export interface DetectContext {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  readonly now: () => number;
  /** The open pull requests, as last listed; null when they could not be read, and remote-tracking branches stand in. */
  readonly pulls: readonly PullRequestSummary[] | null;
  /** The person's GitHub login, as `gh` is signed in; absent or null when unknown. */
  readonly login?: string | null;
  /**
   * The owner of the pass's Git reads: once it stops, the pass's Git read running ends and no other starts
   * (`readsStoppedBy`), and a pass that has not stored yet stores nothing and answers null.
   */
  readonly reads?: GitReads;
  /**
   * The conflict events the pass hands the hook for storing `after` over `before`, asked in the store's own transaction
   * (`tx`); the events between the two when absent.
   */
  readonly events?: (tx: StateQueries, before: readonly StoredConflict[], after: readonly StoredConflict[]) => ConflictFire[];
}

/** This checkout's side: where its work starts, its head and branch, the person, and its changed hunks per file. */
interface MineView {
  readonly head: string;
  /** The branch checked out; null while HEAD is detached. */
  readonly checkedOut: string | null;
  /** The branch checked out, or the one being rebased while HEAD is detached. */
  readonly branch: string | null;
  readonly base: { readonly ref: string; readonly name: string };
  /** The base branch's tip. */
  readonly baseTip: string;
  /** The merge base of HEAD with the base branch. */
  readonly start: string;
  readonly person: string;
  readonly ranges: ReadonlyMap<string, readonly RefHunk[]>;
  /** The branch keys ({@link branchKey}) that are this checkout's own work: its branch and its upstream. */
  readonly own: ReadonlySet<string>;
}

/** A live branch or pull request as the person knows it. */
interface LiveRef {
  readonly ref: string;
  /** The branch it is, however it arrived: its conflicts' ids are built on it. */
  readonly key: string;
  readonly label: string;
  readonly sha: string;
  readonly author: string | null;
  readonly url: string | null;
  readonly title: string | null;
  /** When a branch's tip was committed (ms); null for a pull request. */
  readonly committedAt: number | null;
  /** Whether it is a remote-tracking branch. */
  readonly remote: boolean;
}

/** `work` on each of `items`, at most {@link GIT_CONCURRENCY} at a time, the results in order. */
async function mapLimited<Item, Result>(items: readonly Item[], work: (item: Item) => Promise<Result>): Promise<Result[]> {
  const results: Result[] = [];
  let next = 0;
  const worker = async (): Promise<void> => {
    for (let index = next++; index < items.length; index = next++) results[index] = await work(items[index]!);
  };
  await Promise.all(Array.from({ length: Math.min(GIT_CONCURRENCY, items.length) }, worker));
  return results;
}

/**
 * A branch as one peer: its name without `refs/heads/` or `refs/remotes/<remote>/`, so the local and remote-tracking
 * copies of a branch, and its pull request, are one.
 */
function branchKey(ref: string): string {
  return ref.replace(/^refs\/(?:heads|remotes\/[^/]+)\//u, "");
}

/** A pull request's branch: its head branch, owner-qualified when it comes from another repository. */
function pullKey(pull: PullRequestSummary): string {
  return pull.crossRepository ? `${pull.headOwner ?? ""}:${pull.headRefName}` : pull.headRefName;
}

/**
 * The branch checked out, the checkout's branch (the one being rebased while HEAD is detached) and the keys of its
 * own work.
 */
async function ownBranches(root: string): Promise<{ checkedOut: string | null; branch: string | null; own: Set<string> }> {
  const checkedOut = await currentBranchAsync(root);
  const branch = checkedOut ?? await branchBeingRebased(root);
  if (branch === null) return { checkedOut, branch, own: new Set() };
  const upstream = await upstreamOf(root, branch);
  return { checkedOut, branch, own: new Set(upstream === null ? [branch] : [branch, branchKey(upstream)]) };
}

/** This checkout's side, or why it has none: no commit or base branch, or a base that shares no history with it. */
async function mineView(root: string): Promise<MineView | string> {
  const base = await baseBranchAsync(root);
  if (base === null) return NO_BASE_PROBLEM;
  const [head, baseTip] = await Promise.all([resolveCommit(root, "HEAD"), resolveCommit(root, base.ref)]);
  if (head === null || baseTip === null) return NO_BASE_PROBLEM;
  const start = await mergeBaseAsync(root, baseTip, head);
  if (start === null) return noSharedHistoryProblem(base.name);
  const [{ checkedOut, branch, own }, person, ranges] = await Promise.all([ownBranches(root), personName(root), workspaceRanges(root, start)]);
  return { head, checkedOut, branch, base, baseTip, start, person, ranges, own };
}

/** Whether a commit is work of its own: this repository holds it, and neither this branch nor the base branch does. */
async function liveCommit(root: string, view: MineView, sha: string): Promise<boolean> {
  if ((await resolveCommit(root, sha)) === null) return false;
  return !(await isAncestorAsync(root, sha, view.head)) && !(await isAncestorAsync(root, sha, view.baseTip));
}

/**
 * The pull requests from a fork that are this checkout's own work: their head branch is this checkout's branch (or its
 * upstream's), and either the fork is the person's (its owner is the login `gh` is signed in with) or their head commit
 * was once this branch's tip, as its reflog holds it (an amend or a rebase since moved it). Another person's fork with a
 * branch of the same name is neither.
 */
async function ownForkPulls(root: string, view: MineView, pulls: readonly PullRequestSummary[], login: string | null): Promise<Set<PullRequestSummary>> {
  const named = pulls.filter(pull => pull.crossRepository && view.own.has(pull.headRefName));
  if (named.length === 0 || view.branch === null) return new Set();
  const theirFork = named.filter(pull => login === null || pull.headOwner?.toLowerCase() !== login.toLowerCase());
  const reflog = theirFork.length === 0 ? "" : await gitReadAsync(root, ["reflog", "show", "--format=%H", `refs/heads/${view.branch}`, "--"], LIST_OPTIONS);
  const tips = new Set((reflog ?? "").split("\n"));
  return new Set(named.filter(pull => !theirFork.includes(pull) || tips.has(pull.headCommit)));
}

/**
 * Open pull requests whose head this repository holds and neither this branch nor the base branch already contains,
 * less this checkout's own from a fork ({@link ownForkPulls}).
 */
async function pullRefs(root: string, view: MineView, pulls: readonly PullRequestSummary[], login: string | null): Promise<LiveRef[]> {
  const own = await ownForkPulls(root, view, pulls, login);
  const theirs = pulls.filter(pull => !own.has(pull));
  const live = await mapLimited(theirs, pull => liveCommit(root, view, pull.headCommit));
  return theirs.filter((_, index) => live[index]).map(pull => ({
    ref: `refs/pull/${pull.number}/head`, key: pullKey(pull), label: `PR #${pull.number}`, sha: pull.headCommit, author: pull.author, url: pull.url, title: pull.title,
    committedAt: null, remote: false,
  }));
}

const exists = (path: string): Promise<boolean> => access(path).then(() => true, () => false);

/**
 * The branches checked out in a worktree of this repository (this checkout's own among them), as full ref names, and
 * the branch a worktree is rebasing while its HEAD is detached; null when Git cannot list the worktrees. A worktree
 * whose folder is gone (prunable) holds no work in flight. Line output, as `worktreeRegistry` reads it: `-z` needs Git
 * 2.36, and the `prunable` line 2.31.
 */
async function worktreeBranches(root: string): Promise<Set<string> | null> {
  const listed = await gitReadAsync(root, ["worktree", "list", "--porcelain"], LIST_OPTIONS);
  if (listed === undefined) return null;
  const worktrees = listed.split("\n\n").map(entry => entry.split("\n"));
  const branches = await mapLimited(worktrees, async lines => {
    const path = lines.find(line => line.startsWith("worktree "))?.slice("worktree ".length);
    if (path === undefined || !(await exists(path))) return null;
    const branch = lines.find(line => line.startsWith("branch "))?.slice("branch ".length);
    if (branch !== undefined) return branch;
    const rebased = lines.includes("detached") ? await branchBeingRebased(path) : null;
    return rebased === null ? null : `refs/heads/${rebased}`;
  });
  return new Set(branches.filter(branch => branch !== null));
}

/** Whether a listed ref is a branch compared: a local branch a worktree has checked out, or a remote-tracking one. */
function comparedBranch(name: string, symref: string, checkedOut: ReadonlySet<string>): boolean {
  return symref === "" && (name.startsWith("refs/remotes/") || checkedOut.has(name));
}

/** One `for-each-ref` line of {@link branchRefs} as a live ref, or none. */
function branchRef([name = "", sha = "", symref = "", committed = ""]: readonly string[], checkedOut: ReadonlySet<string>): LiveRef[] {
  if (name === "" || sha === "" || !comparedBranch(name, symref, checkedOut)) return [];
  const label = name.replace(/^refs\/(?:heads|remotes)\//u, "");
  const committedAt = /^\d+$/u.test(committed) ? Number(committed) * 1000 : null;
  return [{ ref: label, key: branchKey(name), label, sha, author: null, url: null, title: null, committedAt, remote: name.startsWith("refs/remotes/") }];
}

/** Why the branches were not compared, when Git could not list them. */
export const UNLISTED_BRANCHES_PROBLEM = "Could not list this repository's branches, so they were not compared.";

/**
 * The branches compared beside open pull requests, with work of their own (neither this branch nor the base branch
 * holds their tip): local branches checked out in a worktree ({@link worktreeBranches}; this checkout's own is left
 * out later), and, with `remotes`, remote-tracking branches; null when Git could not list them. One `for-each-ref` of
 * those the base branch does not hold, which also gives each tip's commit time, less those HEAD holds (a second
 * listing: Git before 2.29 takes one `--no-merged`).
 */
async function branchRefs(root: string, view: MineView, remotes: boolean): Promise<LiveRef[] | null> {
  const patterns = ["refs/heads", ...(remotes ? ["refs/remotes"] : [])];
  const [listed, merged, checkedOut] = await Promise.all([
    gitReadAsync(root, [
      "for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)%00%(committerdate:unix)", `--no-merged=${view.baseTip}`, ...patterns,
    ], LIST_OPTIONS),
    gitReadAsync(root, ["for-each-ref", "--format=%(refname)", `--merged=${view.head}`, ...patterns], LIST_OPTIONS),
    worktreeBranches(root),
  ]);
  if (listed === undefined || merged === undefined || checkedOut === null) return null;
  const held = new Set(merged.split("\n"));
  return listed.split("\n").flatMap(line => {
    const fields = line.split("\0");
    return held.has(fields[0] ?? "") ? [] : branchRef(fields, checkedOut);
  });
}

/** Whether a branch was last committed over {@link STALE_BRANCH_DAYS} days before `now`; one with an open pull request never is. */
function stale(ref: LiveRef, open: ReadonlySet<string>, now: number): boolean {
  return !open.has(ref.key) && ref.committedAt !== null && ref.committedAt < now - STALE_BRANCH_DAYS * DAY_MS;
}

/**
 * The remote-tracking branches past the {@link REMOTE_BRANCH_LIMIT} most recently committed (ties by name): they are not
 * compared, as pull requests past a listing's limit are not.
 */
function pastRemoteLimit(branches: readonly LiveRef[]): Set<LiveRef> {
  const remotes = branches.filter(ref => ref.remote)
    .sort((left, right) => (right.committedAt ?? 0) - (left.committedAt ?? 0) || left.label.localeCompare(right.label));
  return new Set(remotes.slice(REMOTE_BRANCH_LIMIT));
}

/** Why some remote-tracking branches were not compared. */
export const REMOTE_LIMIT_PROBLEM = `Only the ${REMOTE_BRANCH_LIMIT} most recently committed remote-tracking branches were compared.`;

/**
 * The live refs this checkout's work is compared with, open pull requests first, then branches checked out in another
 * worktree, then (open pull requests unread) remote-tracking branches, at most {@link REMOTE_BRANCH_LIMIT} of them
 * (`remotesCut` when there were more): none of this branch's own, none {@link stale}, and each branch and each commit
 * once, the first way it arrived. `branchesListed` is false when Git could not list the branches, and none is compared.
 */
async function liveRefs(
  root: string,
  view: MineView,
  context: DetectContext,
): Promise<{ pulls: LiveRef[]; branches: LiveRef[]; branchesListed: boolean; remotesCut: boolean }> {
  const pulls = context.pulls ?? [];
  const open = new Set(pulls.map(pullKey));
  const now = context.now();
  const [opened, listed] = await Promise.all([pullRefs(root, view, pulls, context.login ?? null), branchRefs(root, view, context.pulls === null)]);
  const keys = new Set(view.own);
  const shas = new Set<string>();
  const kept = [...opened, ...(listed ?? []).filter(ref => !stale(ref, open, now))].filter(ref => {
    if (keys.has(ref.key) || shas.has(ref.sha)) return false;
    keys.add(ref.key);
    shas.add(ref.sha);
    return true;
  });
  const branches = kept.filter(ref => !ref.ref.startsWith("refs/pull/"));
  const cut = pastRemoteLimit(branches);
  return {
    pulls: kept.filter(ref => ref.ref.startsWith("refs/pull/")), branches: branches.filter(ref => !cut.has(ref)), branchesListed: listed !== null,
    remotesCut: cut.size > 0,
  };
}

/**
 * A ref's tip as the card shows it: its author, when its commit was made, and whether it is pushed: a pull request's
 * head, or a commit some remote-tracking branch holds. A local branch never pushed is only committed.
 */
async function refTip(root: string, ref: LiveRef): Promise<{ author: string | null; tip: ConflictSide["tip"] }> {
  const [author = "", at = ""] = ((await gitReadAsync(root, ["log", "-1", "--format=%an%x00%cI", ref.sha, "--"])) ?? "").trim().split("\0");
  if (at === "") return { author: author === "" ? null : author, tip: null };
  const pushed = ref.ref.startsWith("refs/pull/")
    || ((await gitReadAsync(root, ["for-each-ref", "--count=1", `--contains=${ref.sha}`, "--format=%(refname)", "refs/remotes"])) ?? "").trim() !== "";
  return { author: author === "" ? null : author, tip: { at, pushed } };
}

/** A declaration that may stand indented (a method, a nested function, a table). */
const DECLARATION = /^\s*(?:export\s+)?(?:default\s+)?(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:function\b|class\b|interface\b|def\b|fn\b|func\b|struct\b|impl\b|trait\b|create\s+(?:table|index|type|view|function)\b)/iu;
/** A declaration that counts only at the top level: an indented one is a local. */
const TOP_DECLARATION = /^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:const|let|var|type|enum)\s+[\w$]/u;
const TITLE_SYMBOL = 60;

/**
 * The symbol enclosing the contested lines, as a title names it: a declaration the hunk itself changes, else the one
 * git names in its header (`@@ … @@ fn compile(…) {`); cut before its parameters or body, at most 60 characters, at a
 * word boundary.
 */
function enclosingSymbol(hunks: readonly RefHunk[]): string | null {
  const [header = "", ...body] = hunks[0]?.text.split("\n") ?? [];
  const declares = (line: string): boolean => DECLARATION.test(line) || TOP_DECLARATION.test(line);
  const changed = body.find(line => (line.startsWith("-") || line.startsWith("+")) && declares(line.slice(1)))?.slice(1);
  const raw = changed ?? /^@@ [^@]* @@ ?(.*)$/u.exec(header)?.[1] ?? "";
  const symbol = (raw.split(/[({=<]/u)[0] ?? "").trim().replace(/[\s:,;]+$/u, "");
  if (symbol === "") return null;
  if (symbol.length <= TITLE_SYMBOL) return symbol;
  const cut = symbol.slice(0, TITLE_SYMBOL);
  const space = cut.lastIndexOf(" ");
  return `${(space > TITLE_SYMBOL / 3 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/** The subject of the last commit in `range` that touched `file`, if any. */
async function lastSubject(root: string, range: string, file: string): Promise<string | null> {
  const subject = (await gitReadAsync(root, ["log", "--no-merges", "-1", "--format=%s", range, "--", `:(literal)${file}`]))?.trim();
  return subject === undefined || subject === "" ? null : subject;
}

/** The lines a side's hunks cover in its own version of the file (`@@ -a,b +c,d @@`), or null for pure deletions. */
function sideLines(hunks: readonly RefHunk[]): { start: number; end: number } | null {
  const spans = hunks.flatMap(hunk => {
    const match = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/u.exec(hunk.text);
    const start = Number(match?.[1] ?? 0);
    const count = match?.[2] === undefined ? 1 : Number(match[2]);
    return match === null || count === 0 ? [] : [{ start, end: start + count - 1 }];
  });
  if (spans.length === 0) return null;
  return { start: Math.min(...spans.map(span => span.start)), end: Math.max(...spans.map(span => span.end)) };
}

/** A commit `git blame --porcelain` names: when it was made, its subject, and whether it is the range's boundary. */
interface BlamedCommit {
  time: number;
  summary: string;
  boundary: boolean;
}

/** One header line of a blamed commit into `commit`. */
function blameHeader(commit: BlamedCommit, line: string): void {
  if (line.startsWith("committer-time ")) commit.time = Number(line.slice(15));
  else if (line.startsWith("summary ")) commit.summary = line.slice(8);
  else if (line === "boundary") commit.boundary = true;
}

/** The newest commit of a side's own in `git blame --porcelain` output: not the boundary, not uncommitted lines. */
function newestBlamed(porcelain: string): string | null {
  const commits = new Map<string, BlamedCommit>();
  let current: BlamedCommit | undefined;
  for (const line of porcelain.split("\n")) {
    const sha = /^([0-9a-f]{40,64}) \d+ \d+/u.exec(line)?.[1];
    if (sha === undefined) {
      if (current !== undefined) blameHeader(current, line);
      continue;
    }
    // Uncommitted lines blame to the all-zero commit: they have no subject.
    current = /^0+$/u.test(sha) ? undefined : commits.get(sha) ?? { time: 0, summary: "", boundary: false };
    if (current !== undefined) commits.set(sha, current);
  }
  const newest = [...commits.values()].filter(commit => !commit.boundary && commit.summary !== "").sort((left, right) => right.time - left.time)[0];
  return newest?.summary ?? null;
}

/**
 * The subject of the commit that changed a side's contested lines: `git blame` over just those lines, from `from` (the
 * merge base) to `to` (`--contents` of the working file for this branch), the newest blamed commit of the side's own
 * (not the boundary, not uncommitted lines). Null when only uncommitted lines are there; the file's last commit in the
 * range when blame cannot say. No textconv program a repository's config names runs.
 */
async function contestedSubject(root: string, file: string, hunks: readonly RefHunk[], from: string, to: string, working: boolean): Promise<string | null> {
  const lines = sideLines(hunks);
  const blame = lines === null ? undefined : await gitReadAsync(root, [
    "blame", "--porcelain", "--no-textconv", "-L", `${lines.start},${lines.end}`, ...(working ? ["--contents", file] : []), `${from}..${to}`, "--", file,
  ]);
  return blame === undefined ? await lastSubject(root, `${from}..${to}`, file) : newestBlamed(blame);
}

const lineOf = (hunks: readonly { lo: number }[]): number => Math.max(1, Math.floor(hunks[0]?.lo ?? 1));
const hunkText = (hunks: readonly RefHunk[]): string => clipLines(hunks.flatMap(hunk => hunk.text.split("\n")), HUNK_LINES);

/** One detection pass: what it reads Git through, each read once, and this branch's sides by contested lines. */
interface Pass {
  readonly context: DetectContext;
  readonly root: string;
  readonly view: MineView;
  readonly reader: OverlapReader;
  readonly mine: Map<string, Promise<ConflictSide>>;
}

/** Their side of an overlap in `file`. */
async function theirSide(pass: Pass, ref: LiveRef, tip: { author: string | null; tip: ConflictSide["tip"] }, own: string, overlap: GitConflict): Promise<ConflictSide> {
  // Their change to these lines: the commit of theirs that changed them, not their latest.
  const subject = await contestedSubject(pass.root, overlap.file, overlap.theirs, own, ref.sha, false);
  return {
    label: ref.label, ref: ref.ref, sha: ref.sha, author: ref.author ?? tip.author, tip: tip.tip, url: ref.url,
    summary: subject ?? ref.title ?? `edits ${overlap.file}`,
    location: `${overlap.file}:${lineOf(overlap.theirs)}`, hunk: hunkText(overlap.theirs),
  };
}

/**
 * This branch's side of an overlap in `file`: the commit that changed its contested lines, else (uncommitted) its edit.
 * Read once per pass for the same lines, however many refs they overlap.
 */
function mySide(pass: Pass, file: string, ours: readonly RefHunk[]): Promise<ConflictSide> {
  const key = JSON.stringify([file, ours.map(hunk => [hunk.lo, hunk.hi])]);
  let side = pass.mine.get(key);
  if (side === undefined) {
    const { view, root } = pass;
    side = contestedSubject(root, file, ours, view.start, view.head, true).then(subject => ({
      label: "This branch", ref: view.branch ?? view.head, sha: view.head, author: view.person, tip: null, url: null,
      summary: subject ?? `edits ${file}`, location: `${file}:${lineOf(ours)}`, hunk: hunkText(ours),
    }));
    pass.mine.set(key, side);
  }
  return side;
}

/** One overlap with one ref as a stored conflict: the status's view of it, its file, and the anchor its question is built on. */
async function overlapConflict(
  pass: Pass,
  ref: LiveRef,
  tip: { author: string | null; tip: ConflictSide["tip"] },
  own: string,
  overlap: GitConflict & { ours: readonly RefHunk[] },
): Promise<StoredConflict> {
  const { file } = overlap;
  const anchor = await pass.reader.anchor(file, overlap.theirs);
  const question = overlapQuestion(file, anchor);
  const mine = await mySide(pass, file, overlap.ours);
  const theirs = await theirSide(pass, ref, tip, own, overlap);
  const symbol = enclosingSymbol(overlap.theirs);
  const id = conflictId(question, ref.key);
  const conflict: Conflict = {
    id, kind: "overlap", status: "open", shortId: shortConflictId(id), quiet: quietFiles([file]), question,
    title: symbol === null ? basename(file) : `${basename(file)} · ${symbol}`, mine, theirs,
    evidence: [
      `Both sides change ${file} within ${ADJACENT_LINES} lines of each other: yours near line ${lineOf(overlap.ours)}, theirs near line ${lineOf(overlap.theirs)} of the base.`,
      `Only ${ref.label}'s own work since it left ${pass.view.base.name}, and not yet in it, counts; Rerail merged nothing to find this.`,
    ],
    detectedAt: isoSeconds(pass.context.now()), dismissal: null, summary: `${sideName(theirs)} is changing ${symbol ?? file}`, hook: null,
  };
  return { conflict, files: [file], anchor };
}

/** The overlaps of this branch's changes with one live ref; its tip is read once, and only when there is one. */
async function refConflicts(pass: Pass, ref: LiveRef): Promise<StoredConflict[]> {
  const own = await pass.reader.ownBase(ref.sha);
  if (own === null) return [];
  const overlaps = identify(pass.view.ranges, [{ ref: ref.ref, files: await pass.reader.refHunks(own, ref.sha) }]);
  if (overlaps.length === 0) return [];
  const tip = await refTip(pass.root, ref);
  const conflicts: StoredConflict[] = [];
  for (const overlap of overlaps) conflicts.push(await overlapConflict(pass, ref, tip, own, overlap));
  return conflicts;
}

/** What a pass found with the live refs, and the refs it could not compare (with why). */
interface Overlaps {
  readonly conflicts: StoredConflict[];
  readonly failed: ReadonlyArray<{ readonly ref: LiveRef; readonly reason: string }>;
}

/**
 * The overlaps of this branch's changes with each live ref, a few refs at a time. A ref Git cannot compare (a diff too
 * large or too slow) is set aside, so the others still are.
 */
async function overlapConflicts(context: DetectContext, view: MineView, refs: readonly LiveRef[]): Promise<Overlaps> {
  if (view.ranges.size === 0) return { conflicts: [], failed: [] };
  const root = context.checkout.checkoutPath;
  const reader = new OverlapReader(root, { start: view.start, baseTip: view.baseTip }, new Set(view.ranges.keys()));
  const pass: Pass = { context, root, view, reader, mine: new Map() };
  const results = await mapLimited(refs, ref => refConflicts(pass, ref).then(
    conflicts => ({ ref, conflicts, reason: undefined }),
    (error: unknown) => ({ ref, conflicts: [], reason: error instanceof Error ? error.message : String(error) }),
  ));
  return {
    conflicts: results.flatMap(result => result.conflicts),
    failed: results.flatMap(({ ref, reason }) => (reason === undefined ? [] : [{ ref, reason }])),
  };
}

/**
 * What a detection pass left in the store, and what it held just before (the conflict hook's events come from both);
 * how many live branches and open pull requests it compared this checkout's work with, and what it could not read.
 */
export interface DetectedConflicts {
  readonly before: StoredConflict[];
  readonly after: StoredConflict[];
  readonly peers: { readonly branches: number; readonly pullRequests: number };
  readonly problems: string[];
  /** The branch checked out and the base branch's short name, as the pass read them; null where there is none. */
  readonly checkout: { readonly branch: string | null; readonly base: string | null };
  /** The conflict events to hand the hook (`events`); none for a pass that stored nothing. */
  readonly fires: ConflictFire[];
}

/**
 * Find the checkout's conflicts and store them with the dismissed ones. A detection that finishes after Merge Assist
 * was turned off, or once the owner of its `reads` stops, stores nothing and answers null; one that stored answers what
 * it stored, though its `reads` stop after.
 */
export async function detectConflicts(context: DetectContext): Promise<DetectedConflicts | null> {
  return await readsStoppedBy(context.reads, () => detectPass(context));
}

/** Whether the owner of the pass's Git reads has stopped them. */
function readsStopped(context: DetectContext): boolean {
  return context.reads?.stopped === true;
}

/**
 * What a pass that could compare nothing answers, for `problem`: the open conflicts stay as they were, unexamined, as
 * a ref's do when it cannot be read.
 */
function uncompared(known: StoredConflict[], problem: string): DetectedConflicts {
  const stay = known.some(entry => entry.conflict.status === "open") ? [UNCOMPARED_KEPT_PROBLEM] : [];
  return { before: known, after: known, peers: { branches: 0, pullRequests: 0 }, problems: [problem, ...stay], checkout: { branch: null, base: null }, fires: [] };
}

/** {@link detectConflicts}, its Git reads run by its `reads`. */
async function detectPass(context: DetectContext): Promise<DetectedConflicts | null> {
  const root = context.checkout.checkoutPath;
  // What the store held when this pass began: an open conflict another pass stores while this one runs was never
  // examined here, so it stays as it is (below).
  const known = readConflicts(context.database, root);
  const view = await mineView(root);
  // Stopped meanwhile (here and before the store below): its reads ended early and what they gave is no finding.
  if (typeof view === "string") return readsStopped(context) ? null : uncompared(known, view);
  const refs = await liveRefs(root, view, context);
  const { conflicts: detected, failed } = await overlapConflicts(context, view, [...refs.pulls, ...refs.branches]);
  // A ref this pass could not read, or every branch when they could not be listed, keeps the open conflicts it had.
  const unread = new Set(failed.map(({ ref }) => ref.ref));
  const kept = (entry: StoredConflict): boolean => unread.has(entry.conflict.theirs.ref) || (!refs.branchesListed && !entry.conflict.theirs.ref.startsWith("refs/pull/"));
  const problems = [
    ...(refs.branchesListed ? [] : [UNLISTED_BRANCHES_PROBLEM]), ...(refs.remotesCut ? [REMOTE_LIMIT_PROBLEM] : []),
    ...failed.map(({ ref, reason }) => `Could not compare ${ref.label}: ${reason}`),
  ];
  // Stopped meanwhile: its reads ended early and what they gave is no finding, and the database may be closing.
  if (readsStopped(context)) return null;
  const now = context.now();
  const examined = new Set([...known.filter(entry => !kept(entry)), ...detected].map(entry => entry.conflict.id));
  return context.database.transaction(tx => {
    if (!mergeAssistEnabled(tx, context.checkout.repositoryKey)) return null;
    // A dismissal given while this ran stands, and an open conflict this pass did not examine (another pass stored it
    // meanwhile, or its ref could not be read) stays as it is: it was not found gone.
    const before = readConflicts(tx, root);
    const unexamined = before.filter(entry => entry.conflict.status === "open" && !examined.has(entry.conflict.id));
    const after = [...mergeDetected(before, detected), ...unexamined];
    writeConflicts(tx, root, after, now);
    return {
      before, after, peers: { branches: refs.branches.length, pullRequests: refs.pulls.length }, problems,
      checkout: { branch: view.checkedOut, base: view.base.name }, fires: context.events?.(tx, before, after) ?? conflictEvents(before, after),
    };
  }, { behavior: "immediate" });
}
