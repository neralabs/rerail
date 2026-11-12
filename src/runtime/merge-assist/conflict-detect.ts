/**
 * Detecting the checkout's in-flight conflicts (docs/merge-assist-v1-build.md §9.1). Mine is this checkout's working
 * changes, committed and not, against the merge base of HEAD with the base branch. Theirs are the live branches: local
 * branches, remote-tracking branches, and open pull requests whose head this repository holds. An overlap is one
 * conflict per (their ref, file). Every open one is levelled by the dimmer (`conflict-level.ts`). Reads Git only and
 * writes no file; never merges.
 */
import { basename } from "node:path";
import type { CheckoutContext } from "../../engine/catalog/repository-model.js";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { sideName } from "../../engine/code-rule/rerail-steer.js";
import { gitText, runGit } from "../../engine/git.js";
import { baseBranch, commitExists, currentBranch, isAncestor, localBranches, mergeBase, remoteBranches } from "../../engine/merge-assist/branch-git.js";
import { conflictId, overlapQuestion, undimmed } from "../../engine/merge-assist/conflicts.js";
import type { Conflict, ConflictSide } from "../../engine/merge-assist/model.js";
import { ADJACENT_LINES, clipLines, contestedAnchor, type GitConflict, HUNK_LINES, identify, ownBase, refHunks, type RefHunk, workspaceRanges } from "../../engine/merge-assist/overlap.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { levelConflicts, saveBudget } from "./conflict-level.js";
import { mergeDetected, readConflicts, type StoredConflict, writeConflicts } from "./conflict-store.js";
import type { PullRequestSummary } from "./github.js";
import { personName } from "./people.js";
import { isoSeconds } from "./state.js";

/** Why a checkout with no base branch has no conflicts. */
export const NO_BASE_PROBLEM = "No base branch found (origin/HEAD, main or master), so Rerail cannot tell this branch's work from the base's.";

export interface DetectContext {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  readonly now: () => number;
  /** The open pull requests, as last listed; none without `gh`. */
  readonly pulls: readonly PullRequestSummary[];
  /** The session whose boundary asked for this detection; null for the dashboard and the CLI, which grant no stop. */
  readonly sessionId: string | null;
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
  return { head, branch: currentBranch(root), base, start, person: personName(root), ranges: workspaceRanges(root, start) };
}

/** Open pull requests whose head this repository holds and this branch does not already contain. */
function pullRefs(root: string, view: MineView, pulls: readonly PullRequestSummary[]): LiveRef[] {
  return pulls
    .filter(pull => pull.headRefName !== view.branch && commitExists(root, pull.headCommit) && !isAncestor(root, pull.headCommit, view.head))
    .map(pull => ({ ref: `refs/pull/${pull.number}/head`, label: `PR #${pull.number}`, sha: pull.headCommit, author: pull.author, url: pull.url, title: pull.title }));
}

/** Local and remote-tracking branches with work of their own: not this branch, its upstream or the base, not merged. */
function branchRefs(root: string, view: MineView, taken: Set<string>): LiveRef[] {
  const upstream = gitText(root, ["rev-parse", "--symbolic-full-name", "@{upstream}"]);
  const skip = new Set([view.base.ref, upstream, view.branch === null ? undefined : `refs/heads/${view.branch}`]);
  const tips = [
    ...localBranches(root).map(branch => ({ name: `refs/heads/${branch.name}`, commit: branch.commit })),
    ...remoteBranches(root).map(remote => ({ name: remote.ref, commit: remote.commit })),
  ];
  const refs: LiveRef[] = [];
  for (const { name, commit } of tips) {
    if (skip.has(name) || taken.has(commit) || isAncestor(root, commit, view.base.ref) || isAncestor(root, commit, view.head)) continue;
    taken.add(commit);
    refs.push({ ref: name.replace(/^refs\/(?:heads|remotes)\//u, ""), label: name.replace(/^refs\/(?:heads|remotes)\//u, ""), sha: commit, author: null, url: null, title: null });
  }
  return refs;
}

/**
 * A side's tip as the card shows it: when its commit was made, and whether it is pushed: a pull request's head, or a
 * commit some remote-tracking branch holds. A local branch never pushed is only committed.
 */
function tipState(root: string, sha: string, ref: string, at: string | null): ConflictSide["tip"] {
  if (at === null) return null;
  const pushed = ref.startsWith("refs/pull/") || (gitText(root, ["for-each-ref", "--count=1", `--contains=${sha}`, "--format=%(refname)", "refs/remotes"]) ?? "") !== "";
  return { at, pushed };
}

/** A ref's tip author and commit time, as `git log` gives them. */
function tipOf(root: string, sha: string): { author: string | null; at: string | null } {
  const [author, at] = (gitText(root, ["log", "-1", "--format=%an%x00%cI", sha, "--"]) ?? "").split("\0");
  return { author: author === undefined || author === "" ? null : author, at: at === undefined || at === "" ? null : at };
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
function lastSubject(root: string, range: string, file: string): string | null {
  const subject = gitText(root, ["log", "--no-merges", "-1", "--format=%s", range, "--", `:(literal)${file}`]);
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

/**
 * The subject of the commit that changed a side's contested lines: `git blame` over just those lines, from `from` (the
 * merge base) to `to` (`--contents` of the working file for this branch), the newest blamed commit of the side's own
 * (not the boundary, not uncommitted lines). Null when only uncommitted lines are there; the file's last commit in the
 * range when blame cannot say.
 */
function contestedSubject(root: string, file: string, hunks: readonly RefHunk[], from: string, to: string, working: boolean): string | null {
  const lines = sideLines(hunks);
  const blame = lines === null ? null : runGit(root, [
    "blame", "--porcelain", "-L", `${lines.start},${lines.end}`, ...(working ? ["--contents", file] : []), `${from}..${to}`, "--", file,
  ]);
  if (blame === null || blame.status !== 0) return lastSubject(root, `${from}..${to}`, file);
  const commits = new Map<string, { time: number; summary: string; boundary: boolean }>();
  let current: { time: number; summary: string; boundary: boolean } | undefined;
  for (const line of blame.stdout.split("\n")) {
    const header = /^([0-9a-f]{40,64}) \d+ \d+/u.exec(line);
    if (header !== null) {
      const sha = header[1]!;
      current = commits.get(sha) ?? (/^0+$/u.test(sha) ? undefined : { time: 0, summary: "", boundary: false });
      if (current !== undefined) commits.set(sha, current);
    } else if (current !== undefined && line.startsWith("committer-time ")) current.time = Number(line.slice(15));
    else if (current !== undefined && line.startsWith("summary ")) current.summary = line.slice(8);
    else if (current !== undefined && line === "boundary") current.boundary = true;
  }
  const newest = [...commits.values()].filter(commit => !commit.boundary && commit.summary !== "").sort((left, right) => right.time - left.time)[0];
  return newest?.summary ?? null;
}

const lineOf = (hunks: readonly { lo: number }[]): number => Math.max(1, Math.floor(hunks[0]?.lo ?? 1));
const hunkText = (hunks: readonly RefHunk[]): string => clipLines(hunks.flatMap(hunk => hunk.text.split("\n")), HUNK_LINES);

/** Their side of an overlap in `file`. */
function theirSide(root: string, ref: LiveRef, own: string, overlap: GitConflict): ConflictSide {
  const tip = tipOf(root, ref.sha);
  // Their change to these lines: the commit of theirs that changed them, not their latest.
  const subject = contestedSubject(root, overlap.file, overlap.theirs, own, ref.sha, false);
  return {
    label: ref.label, ref: ref.ref, sha: ref.sha, author: ref.author ?? tip.author, tip: tipState(root, ref.sha, ref.ref, tip.at), url: ref.url,
    summary: subject ?? ref.title ?? `edits ${overlap.file}`,
    location: `${overlap.file}:${lineOf(overlap.theirs)}`, hunk: hunkText(overlap.theirs),
  };
}

/** This branch's side of an overlap in `file`: the commit that changed its contested lines, else (uncommitted) its edit. */
function mySide(root: string, view: MineView, file: string, ours: readonly RefHunk[]): ConflictSide {
  return {
    label: "This branch", ref: view.branch ?? view.head, sha: view.head, author: view.person, tip: null, url: null,
    summary: contestedSubject(root, file, ours, view.start, view.head, true) ?? `edits ${file}`, location: `${file}:${lineOf(ours)}`, hunk: hunkText(ours),
  };
}

/** One overlap with one ref as a stored conflict: the status's view of it, its file, its base lines and their anchor. */
function overlapConflict(context: DetectContext, view: MineView, ref: LiveRef, own: string, overlap: GitConflict & { ours: readonly RefHunk[] }): StoredConflict {
  const { file } = overlap;
  const lines = overlap.theirs.map(hunk => ({ lo: hunk.lo, hi: hunk.hi }));
  const anchor = contestedAnchor(context.checkout.checkoutPath, view.start, file, lines);
  const question = overlapQuestion(file, anchor);
  const mine = mySide(context.checkout.checkoutPath, view, file, overlap.ours);
  const theirs = theirSide(context.checkout.checkoutPath, ref, own, overlap);
  const symbol = enclosingSymbol(overlap.theirs);
  const id = conflictId(question, ref.ref);
  const conflict: Conflict = {
    id, kind: "overlap", status: "open", ...undimmed(id), question,
    title: symbol === null ? basename(file) : `${basename(file)} · ${symbol}`, mine, theirs,
    evidence: [
      `Both sides change ${file} within ${ADJACENT_LINES} lines of each other: yours near line ${lineOf(overlap.ours)}, theirs near line ${lineOf(overlap.theirs)} of the base.`,
      `Only ${ref.label}'s own work since it left ${view.base.name} counts; Rerail merged nothing to find this.`,
    ],
    detectedAt: isoSeconds(context.now()), dismissal: null, summary: `${sideName(theirs)} is changing ${symbol ?? file}`, hook: null,
  };
  return { conflict, files: [file], lines, anchor };
}

/** The live refs this checkout's work is compared with: open pull requests, then branches. */
function liveRefs(root: string, view: MineView, pulls: readonly PullRequestSummary[]): { pulls: LiveRef[]; branches: LiveRef[] } {
  const open = pullRefs(root, view, pulls);
  return { pulls: open, branches: branchRefs(root, view, new Set(open.map(pull => pull.sha))) };
}

/** The overlaps of this branch's changes with each live ref; `touches` gets every changed file's refs that change it too. */
function overlapConflicts(context: DetectContext, view: MineView, refs: readonly LiveRef[], touches: Map<string, Set<string>>): StoredConflict[] {
  if (view.ranges.size === 0) return [];
  const root = context.checkout.checkoutPath;
  return refs.flatMap(ref => {
    const own = ownBase(root, ref.sha, view.base.ref, view.start);
    if (own === null) return [];
    const files = refHunks(root, view.start, own, ref.sha, [...view.ranges.keys()]);
    for (const file of Object.keys(files)) touches.set(file, (touches.get(file) ?? new Set()).add(ref.ref));
    return identify(view.ranges, [{ ref: ref.ref, files }]).map(overlap => overlapConflict(context, view, ref, own, overlap));
  });
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
}

/**
 * Find the checkout's conflicts and store them with the dismissed ones, and give every open one its level from the
 * dimmer. A detection that finishes after Merge Assist was turned off stores nothing and answers null.
 */
export async function detectConflicts(context: DetectContext): Promise<DetectedConflicts | null> {
  const root = context.checkout.checkoutPath;
  const view = mineView(root);
  if (view === null) return { before: [], after: [], peers: { branches: 0, pullRequests: 0 }, problems: [NO_BASE_PROBLEM] };
  const touches = new Map<string, Set<string>>();
  const refs = liveRefs(root, view, context.pulls);
  const detected = overlapConflicts(context, view, [...refs.pulls, ...refs.branches], touches);
  const known = readConflicts(context.database, root);
  const result = mergeDetected(known, detected);
  const now = context.now();
  const levelled = levelConflicts({
    db: context.database, root, base: view.base, start: view.start, head: view.head, person: view.person, sessionId: context.sessionId,
    pulls: context.pulls, touches, now,
  }, result);
  const examined = new Set([...known, ...levelled.entries].map(entry => entry.conflict.id));
  return context.database.transaction(tx => {
    if (!mergeAssistEnabled(tx, context.checkout.repositoryKey)) return null;
    // A dismissal given while this ran stands, and an open conflict another pass stored meanwhile, which this one
    // never examined, stays as it is: it was not found gone.
    const before = readConflicts(tx, root);
    const unexamined = before.filter(entry => entry.conflict.status === "open" && !examined.has(entry.conflict.id));
    const after = [...mergeDetected(before, levelled.entries), ...unexamined];
    writeConflicts(tx, root, after, now);
    saveBudget(tx, view.person, levelled.grants, now);
    return { before, after, peers: { branches: refs.branches.length, pullRequests: refs.pulls.length }, problems: [] };
  }, { behavior: "immediate" });
}
