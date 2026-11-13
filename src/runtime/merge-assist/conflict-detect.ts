/**
 * Detecting the checkout's in-flight conflicts (docs/merge-assist-v1-build.md). Mine is this checkout's working
 * changes, committed and not, against the merge base of HEAD with the base branch. Theirs are the live branches: local
 * branches, remote-tracking branches, and open pull requests whose head this repository holds, less this branch's own
 * (its local and remote-tracking copies, its upstream, its pull request; the branch being rebased while HEAD is
 * detached) and work the base branch already holds. A branch is one peer however it arrives (a pull request, a local or
 * a remote-tracking branch of the same name), so its conflicts keep their ids. An overlap is one conflict per (their
 * branch, file); one whose every file is a lockfile, a changelog, generated output or under `docs/` is quiet. Reads Git
 * only, without blocking, a few processes at a time; writes no file; never merges.
 */
import { basename } from "node:path";
import type { CheckoutContext } from "../../engine/catalog/repository-model.js";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { sideName } from "../../engine/code-rule/rerail-steer.js";
import {
  baseBranch,
  branchBeingRebased,
  currentBranch,
  gitReadAsync,
  isAncestorAsync,
  mergeBaseAsync,
  resolveCommit,
  upstreamOf,
} from "../../engine/merge-assist/branch-git.js";
import { conflictId, overlapQuestion, quietFiles, shortConflictId } from "../../engine/merge-assist/conflicts.js";
import type { Conflict, ConflictSide } from "../../engine/merge-assist/model.js";
import { ADJACENT_LINES, clipLines, type GitConflict, HUNK_LINES, identify, OverlapReader, type RefHunk, workspaceRanges } from "../../engine/merge-assist/overlap.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeDetected, readConflicts, type StoredConflict, writeConflicts } from "./conflict-store.js";
import type { PullRequestSummary } from "./github.js";
import { personName } from "./people.js";
import { isoSeconds } from "./state.js";

/** Why a checkout with no base branch has no conflicts. */
export const NO_BASE_PROBLEM = "No base branch found (origin/HEAD, main or master), so Rerail cannot tell this branch's work from the base's.";

/** Why a checkout whose base branch shares no history with it (an orphan branch, a shallow clone) has no conflicts. */
export function noSharedHistoryProblem(base: string): string {
  return `${base} shares no history with this branch, so Rerail cannot tell this branch's work from the base's.`;
}

/** How many Git processes a detection pass runs at once. */
const GIT_CONCURRENCY = 6;
const LIST_OPTIONS = { maxBuffer: 16 * 1024 * 1024 };

export interface DetectContext {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  readonly now: () => number;
  /** The open pull requests, as last listed; none without `gh`. */
  readonly pulls: readonly PullRequestSummary[];
}

/** This checkout's side: where its work starts, its head and branch, the person, and its changed hunks per file. */
interface MineView {
  readonly head: string;
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

/** The checkout's branch (the one being rebased while HEAD is detached) and the keys of its own work. */
async function ownBranches(root: string): Promise<{ branch: string | null; own: Set<string> }> {
  const branch = currentBranch(root) ?? await branchBeingRebased(root);
  if (branch === null) return { branch, own: new Set() };
  const upstream = await upstreamOf(root, branch);
  return { branch, own: new Set(upstream === null ? [branch] : [branch, branchKey(upstream)]) };
}

/** This checkout's side, or why it has none: no commit or base branch, or a base that shares no history with it. */
async function mineView(root: string): Promise<MineView | string> {
  const base = baseBranch(root);
  if (base === null) return NO_BASE_PROBLEM;
  const [head, baseTip] = await Promise.all([resolveCommit(root, "HEAD"), resolveCommit(root, base.ref)]);
  if (head === null || baseTip === null) return NO_BASE_PROBLEM;
  const start = await mergeBaseAsync(root, baseTip, head);
  if (start === null) return noSharedHistoryProblem(base.name);
  const { branch, own } = await ownBranches(root);
  return { head, branch, base, baseTip, start, person: personName(root), ranges: await workspaceRanges(root, start), own };
}

/** Whether a commit is work of its own: this repository holds it, and neither this branch nor the base branch does. */
async function liveCommit(root: string, view: MineView, sha: string): Promise<boolean> {
  if ((await resolveCommit(root, sha)) === null) return false;
  return !(await isAncestorAsync(root, sha, view.head)) && !(await isAncestorAsync(root, sha, view.baseTip));
}

/** Open pull requests whose head this repository holds and neither this branch nor the base branch already contains. */
async function pullRefs(root: string, view: MineView, pulls: readonly PullRequestSummary[]): Promise<LiveRef[]> {
  const live = await mapLimited(pulls, pull => liveCommit(root, view, pull.headCommit));
  return pulls.filter((_, index) => live[index]).map(pull => ({
    ref: `refs/pull/${pull.number}/head`, key: pullKey(pull), label: `PR #${pull.number}`, sha: pull.headCommit, author: pull.author, url: pull.url, title: pull.title,
  }));
}

/** Local and remote-tracking branches with work of their own: neither this branch nor the base branch holds their tip. */
async function branchRefs(root: string, view: MineView): Promise<LiveRef[]> {
  const listed = await gitReadAsync(root, [
    "for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)", `--no-merged=${view.baseTip}`, `--no-merged=${view.head}`, "refs/heads", "refs/remotes",
  ], LIST_OPTIONS);
  return (listed ?? "").split("\n").flatMap(line => {
    const [name = "", sha = "", symref = ""] = line.split("\0");
    if (name === "" || sha === "" || symref !== "") return [];
    const label = name.replace(/^refs\/(?:heads|remotes)\//u, "");
    return [{ ref: label, key: branchKey(name), label, sha, author: null, url: null, title: null }];
  });
}

/**
 * The live refs this checkout's work is compared with, open pull requests first, then local, then remote-tracking
 * branches: none of this branch's own, and each branch and each commit once, the first way it arrived.
 */
async function liveRefs(root: string, view: MineView, pulls: readonly PullRequestSummary[]): Promise<{ pulls: LiveRef[]; branches: LiveRef[] }> {
  const [open, branches] = await Promise.all([pullRefs(root, view, pulls), branchRefs(root, view)]);
  const keys = new Set(view.own);
  const shas = new Set<string>();
  const kept = [...open, ...branches].filter(ref => {
    if (keys.has(ref.key) || shas.has(ref.sha)) return false;
    keys.add(ref.key);
    shas.add(ref.sha);
    return true;
  });
  return { pulls: kept.filter(ref => ref.ref.startsWith("refs/pull/")), branches: kept.filter(ref => !ref.ref.startsWith("refs/pull/")) };
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

/** The overlaps of this branch's changes with each live ref, a few refs at a time. */
async function overlapConflicts(context: DetectContext, view: MineView, refs: readonly LiveRef[]): Promise<StoredConflict[]> {
  if (view.ranges.size === 0) return [];
  const root = context.checkout.checkoutPath;
  const reader = new OverlapReader(root, { start: view.start, baseTip: view.baseTip }, new Set(view.ranges.keys()));
  const pass: Pass = { context, root, view, reader, mine: new Map() };
  return (await mapLimited(refs, ref => refConflicts(pass, ref))).flat();
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
 * Find the checkout's conflicts and store them with the dismissed ones. A detection that finishes after Merge Assist
 * was turned off stores nothing and answers null.
 */
export async function detectConflicts(context: DetectContext): Promise<DetectedConflicts | null> {
  const root = context.checkout.checkoutPath;
  const view = await mineView(root);
  if (typeof view === "string") return { before: [], after: [], peers: { branches: 0, pullRequests: 0 }, problems: [view] };
  const refs = await liveRefs(root, view, context.pulls);
  const detected = await overlapConflicts(context, view, [...refs.pulls, ...refs.branches]);
  const known = readConflicts(context.database, root);
  const now = context.now();
  const examined = new Set([...known, ...detected].map(entry => entry.conflict.id));
  return context.database.transaction(tx => {
    if (!mergeAssistEnabled(tx, context.checkout.repositoryKey)) return null;
    // A dismissal given while this ran stands, and an open conflict another pass stored meanwhile, which this one
    // never examined, stays as it is: it was not found gone.
    const before = readConflicts(tx, root);
    const unexamined = before.filter(entry => entry.conflict.status === "open" && !examined.has(entry.conflict.id));
    const after = [...mergeDetected(before, detected), ...unexamined];
    writeConflicts(tx, root, after, now);
    return { before, after, peers: { branches: refs.branches.length, pullRequests: refs.pulls.length }, problems: [] };
  }, { behavior: "immediate" });
}
