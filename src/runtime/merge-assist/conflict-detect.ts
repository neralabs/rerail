/**
 * Finding the checkout's in-flight conflicts (CONTRACT: Detection). Mine is this checkout's working changes, committed
 * and not, against the merge base of HEAD with the base branch. Theirs are the live branches: local branches,
 * remote-tracking branches, and open pull requests whose head this repository holds. An overlap is one conflict per
 * (their ref, file); a Talk finding (a teammate's decision contradicting one of this branch's) is one conflict too, and
 * so are two provisional ledger entries on one question, this branch's and a live ref's, with different winners. Each
 * pass reads the ledger view with the merge check (`ledger-store.ts`): new conflicts a verdict in it answers are settled
 * by precedent, and a settled one whose winner's position moved is open again. Every open one is levelled by the
 * dimmer (`conflict-level.ts`). Reads Git only and writes the working tree only when precedent decides; never merges.
 */
import { createHash } from "node:crypto";
import { basename } from "node:path";
import { and, eq, ne } from "drizzle-orm";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { gitText, runGit } from "../../engine/git.js";
import { baseBranch, commitExists, currentBranch, isAncestor, localBranches, mergeBase, remoteBranches } from "../../engine/merge-assist/branch-git.js";
import { readWorktreeCandidates } from "../../engine/merge-assist/candidate-files.js";
import { CANDIDATE_DIRECTORY, candidateFileName, candidateHash, type Candidate } from "../../engine/merge-assist/candidate.js";
import { conflictId, conflictRecord, undimmed } from "../../engine/merge-assist/conflicts.js";
import { decisionFileName, DECISIONS_DIRECTORY, type Position, winnerOf } from "../../engine/merge-assist/decision-entry.js";
import { hunkFingerprint } from "../../engine/merge-assist/decision-files.js";
import { hasVerdict, type LedgerItem, type LedgerView, verdictDisagreements } from "../../engine/merge-assist/ledger.js";
import type { Conflict, ConflictSide } from "../../engine/merge-assist/model.js";
import { ADJACENT_LINES, clipLines, type GitConflict, HUNK_LINES, hunkLinesAt, identify, ownBase, refHunks, type RefHunk, workspaceRanges } from "../../engine/merge-assist/overlap.js";
import { questionOf } from "../../engine/merge-assist/conflict-record.js";
import { mergeAssistItems, type MergeAssistItemRow } from "../../engine/storage/schema.js";
import { levelConflicts, saveBudget } from "./conflict-level.js";
import { type SettleContext, settleByPrecedent } from "./conflict-resolve.js";
import { mergeDetected, newConflicts, readConflicts, type StoredConflict, writeConflicts } from "./conflict-store.js";
import type { PullRequestSummary } from "./github.js";
import { refreshFollowed, refreshLedger } from "./ledger-store.js";
import { personName } from "./people.js";
import { isoSeconds } from "./state.js";

export interface DetectContext extends SettleContext {
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
  // Rerail's own candidate and ledger files are decisions, not work that overlaps.
  const ranges = new Map([...workspaceRanges(root, start)].filter(([path]) => !path.startsWith(`${CANDIDATE_DIRECTORY}/`) && !path.startsWith(`${DECISIONS_DIRECTORY}/`)));
  return { head, branch: currentBranch(root), base, start, person: personName(root), ranges };
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
  const candidates = [
    ...localBranches(root).map(branch => ({ name: `refs/heads/${branch.name}`, commit: branch.commit })),
    ...remoteBranches(root).map(remote => ({ name: remote.ref, commit: remote.commit })),
  ];
  const refs: LiveRef[] = [];
  for (const { name, commit } of candidates) {
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

/**
 * A side's position in an overlap: its hunks against `start` (to `to`, or the working files) at the base lines their
 * change covers, with its fingerprint; the hunks it was found with when Git cannot diff them again.
 */
function hunkPosition(root: string, start: string, to: string | null, at: Extract<Position, { kind: "hunks" }>, found: readonly RefHunk[]): { fingerprint: string; at: Position } {
  let lines: string[];
  try {
    lines = hunkLinesAt(root, start, to, at.path, at.lines);
  } catch {
    lines = found.flatMap(hunk => hunk.text.split("\n").slice(1));
  }
  return { fingerprint: hunkFingerprint(lines), at };
}

/** Their side of an overlap in `file`. */
function theirSide(root: string, ref: LiveRef, own: string, overlap: GitConflict): ConflictSide {
  const tip = tipOf(root, ref.sha);
  // Their change to these lines: the commit of theirs that changed them, not their latest.
  const subject = contestedSubject(root, overlap.file, overlap.theirs, own, ref.sha, false);
  return {
    label: ref.label, ref: ref.ref, sha: ref.sha, author: ref.author ?? tip.author, tip: tipState(root, ref.sha, ref.ref, tip.at), url: ref.url,
    summary: subject ?? ref.title ?? `edits ${overlap.file}`,
    location: `${overlap.file}:${lineOf(overlap.theirs)}`, hunk: hunkText(overlap.theirs), decisionId: null,
  };
}

/** This branch's side of an overlap in `file`: the commit that changed its contested lines, else (uncommitted) its edit. */
function mySide(root: string, view: MineView, file: string, ours: readonly RefHunk[]): ConflictSide {
  return {
    label: "This branch", ref: view.branch ?? view.head, sha: view.head, author: view.person, tip: null, url: null,
    summary: contestedSubject(root, file, ours, view.start, view.head, true) ?? `edits ${file}`, location: `${file}:${lineOf(ours)}`, hunk: hunkText(ours), decisionId: null,
  };
}

/** One overlap with one ref as a stored conflict: the status's view of it, its file, and its log record. */
function overlapConflict(context: DetectContext, view: MineView, ref: LiveRef, own: string, overlap: GitConflict & { ours: readonly RefHunk[] }): StoredConflict {
  const { file } = overlap;
  // The question is the file and the base lines their overlapping change covers: another overlap in the same file, or
  // another change of theirs, is another question, so an earlier verdict on one never settles the other.
  const lines = overlap.theirs.map(hunk => `${file}:${Math.floor(hunk.lo)}-${Math.floor(hunk.hi)}`);
  const question = questionOf({ files: [file], symbols: lines }, { files: [file], symbols: lines });
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
    detectedAt: isoSeconds(context.now()), resolution: null,
  };
  const record = conflictRecord({ repository: context.checkout.repositoryKey, base: view.start, question }, { a: { side: mine, files: [file] }, b: { side: theirs, files: [file] } });
  const at = { kind: "hunks" as const, path: file, lines: overlap.theirs.map(hunk => ({ lo: hunk.lo, hi: hunk.hi })) };
  const root = context.checkout.checkoutPath;
  const positions = { a: hunkPosition(root, view.start, null, at, overlap.ours), b: hunkPosition(root, view.start, ref.sha, at, overlap.theirs) };
  return { conflict, files: [file], record, positions };
}

/** The overlaps of this branch's changes with each live ref; `touches` gets every changed file's refs that change it too. */
function overlapConflicts(context: DetectContext, view: MineView, touches: Map<string, Set<string>>): StoredConflict[] {
  if (view.ranges.size === 0) return [];
  const root = context.checkout.checkoutPath;
  const pulls = pullRefs(root, view, context.pulls);
  const refs = [...pulls, ...branchRefs(root, view, new Set(pulls.map(pull => pull.sha)))];
  return refs.flatMap(ref => {
    const own = ownBase(root, ref.sha, view.base.ref, view.start);
    if (own === null) return [];
    const files = refHunks(root, view.start, own, ref.sha, [...view.ranges.keys()]);
    for (const file of Object.keys(files)) touches.set(file, (touches.get(file) ?? new Set()).add(ref.ref));
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
  return { ...side, tip: null, summary: decision.decision, location: `${CANDIDATE_DIRECTORY}/${candidateFileName(decision.id)}`, hunk: null, decisionId: decision.id };
}

/** The decision an amendment chain starts from: `supersedes` followed back while `supersedesOf` knows the next. */
function chainRoot(id: string, supersedesOf: (id: string) => string | null | undefined): string {
  const seen = new Set([id]);
  let root = id;
  for (let next = supersedesOf(root); next !== null && next !== undefined && !seen.has(next); next = supersedesOf(root)) {
    seen.add(next);
    root = next;
  }
  return root;
}

/**
 * One Talk finding as a stored conflict, or null when it lacks this branch's decision or their commit. `theirs` holds
 * the other branches' candidates this checkout knows, by `<peer ref>\0<candidate id>`.
 */
function talkConflict(context: DetectContext, view: MineView, item: MergeAssistItemRow, own: ReadonlyMap<string, Candidate>, known: ReadonlyMap<string, Candidate>): StoredConflict | null {
  const evidence = item.evidence as TalkEvidence;
  const mineId = evidence.mine?.id;
  const mineDecision = evidence.mine?.decision;
  if (typeof mineId !== "string" || typeof mineDecision !== "string" || item.peerCommit === null) return null;
  const peer = talkRef(item);
  const mineCandidate = own.get(mineId);
  const mineFiles = [...(mineCandidate?.scope ?? [])];
  // The question is the pair of decisions where each started, sorted, with their scopes: both branches compute the same
  // one, so their verdicts can meet, and an amendment (Keep mine amends this branch's) leaves it settled.
  const mineRoot = chainRoot(mineId, id => own.get(id)?.supersedes);
  const theirRoot = chainRoot(item.candidateId, id => (id === item.candidateId ? item.candidate : known.get(`${item.peerRef}\0${id}`))?.supersedes);
  const symbols = [mineRoot, theirRoot].sort();
  const theirRootScope = known.get(`${item.peerRef}\0${theirRoot}`)?.scope ?? item.candidate.scope;
  const question = questionOf({ files: [...(own.get(mineRoot)?.scope ?? mineFiles)], symbols }, { files: [...theirRootScope], symbols });
  const author = item.peerKind === "pull-request" && item.peerAuthor !== null ? item.peerAuthor : item.candidate.author;
  const mine = decisionSide({ label: "This branch", ref: view.branch ?? view.head, sha: view.head, author: view.person, url: null }, { id: mineId, decision: mineDecision });
  const theirs = decisionSide({ label: peer.label, ref: peer.ref, sha: item.peerCommit, author, url: item.peerUrl }, item.candidate);
  const explanation = typeof evidence.judgment?.explanation === "string" ? evidence.judgment.explanation : "Your agent's compare call found they contradict.";
  const id = conflictId(question, peer.ref);
  const conflict: Conflict = {
    id, kind: "decisions", status: "open", ...undimmed(id), question,
    title: item.candidate.decision.length > 80 ? `${item.candidate.decision.slice(0, 79).trimEnd()}…` : item.candidate.decision,
    mine, theirs,
    evidence: [explanation, "Both decisions are in force on their branches, and no decision in force answers which one holds."],
    detectedAt: isoSeconds(context.now()), resolution: null,
  };
  const files = [...new Set([...mineFiles, ...item.candidate.scope])];
  const record = conflictRecord({ repository: context.checkout.repositoryKey, base: view.start, question }, { a: { side: mine, files: mineFiles }, b: { side: theirs, files: item.candidate.scope } });
  // A decision's position is its candidate's content; one this branch no longer holds, the text it was judged by.
  const positions = {
    a: { fingerprint: mineCandidate === undefined ? createHash("sha256").update(mineDecision).digest("hex") : candidateHash(mineCandidate), at: { kind: "candidate" as const, candidateId: mineId } },
    b: { fingerprint: item.candidateHash, at: { kind: "candidate" as const, candidateId: item.candidateId } },
  };
  return { conflict, files, record, positions };
}

/** Every Talk finding the person has not dismissed, as a conflict of kind `decisions`. */
async function decisionConflicts(context: DetectContext, view: MineView): Promise<StoredConflict[]> {
  const root = context.checkout.checkoutPath;
  const items = context.database.select().from(mergeAssistItems).where(and(
    eq(mergeAssistItems.checkoutPath, root), eq(mergeAssistItems.origin, "peer"), eq(mergeAssistItems.tier, "talk"), ne(mergeAssistItems.status, "dismissed"),
  )).all();
  if (items.length === 0) return [];
  const own = new Map((await readWorktreeCandidates(root)).candidates.map(candidate => [candidate.id, candidate]));
  const peers = context.database.select().from(mergeAssistItems).where(and(eq(mergeAssistItems.checkoutPath, root), eq(mergeAssistItems.origin, "peer"))).all();
  const known = new Map(peers.map(peer => [`${peer.peerRef}\0${peer.candidateId}`, peer.candidate]));
  return items.flatMap(item => talkConflict(context, view, item, own, known) ?? []);
}

/**
 * One side of a verdicts conflict: the ledger entry's place, its winner's position, and the decision it follows.
 * `tip` is the ref carrying it: null for this branch, whose entry may not be committed.
 */
function verdictSide(item: LedgerItem, side: Pick<ConflictSide, "label" | "ref" | "sha" | "tip">): ConflictSide {
  const { entry } = item;
  const winner = winnerOf(entry);
  const picked = winner === null ? "a new decision" : `${entry.sides[winner].label}'s side`;
  return {
    ...side, author: entry.decidedBy.kind === "person" ? entry.decidedBy.name : null, url: null,
    summary: entry.result.text, location: `${DECISIONS_DIRECTORY}/${decisionFileName(entry.id)}`, hunk: `Chose ${picked}.`, decisionId: entry.result.candidateId,
  };
}

/** A position a verdict picked, as a conflict side holds it: its winner's, or for a new decision its text. */
function verdictPosition(item: LedgerItem): { fingerprint: string; at: Position } {
  const winner = winnerOf(item.entry);
  if (winner !== null) return { fingerprint: item.entry.sides[winner].fingerprint, at: item.entry.sides[winner].at };
  return { fingerprint: createHash("sha256").update(item.entry.result.text).digest("hex"), at: item.entry.sides.a.at };
}

/** Each of this branch's provisional verdicts a live ref's provisional verdict on the same question contradicts. */
function verdictConflicts(context: DetectContext, view: MineView, ledger: LedgerView): StoredConflict[] {
  return verdictDisagreements(ledger).map(({ mine: ours, theirs: other }) => {
    const { question } = ours.entry;
    const mine = verdictSide(ours, { label: "This branch", ref: view.branch ?? view.head, sha: view.head, tip: null });
    const theirSha = other.source.sha ?? view.head;
    const theirs = verdictSide(other, { label: other.source.label, ref: other.source.ref, sha: theirSha, tip: tipState(context.checkout.checkoutPath, theirSha, other.source.ref, tipOf(context.checkout.checkoutPath, theirSha).at) });
    // Its own id space: an overlap on the same question with the same ref is another conflict.
    const id = conflictId(`verdicts:${question}`, other.source.ref);
    const files = [...new Set([ours.entry.sides.a.at, other.entry.sides.a.at].flatMap(at => (at.kind === "hunks" ? [at.path] : [])))];
    const conflict: Conflict = {
      id, kind: "verdicts", status: "open", ...undimmed(id), question, title: ours.entry.title, mine, theirs,
      evidence: [
        `${theirs.author ?? other.source.label} decided this question on ${other.source.label}: ${other.entry.result.text}`,
        `This branch decided it differently: ${ours.entry.result.text}`,
        "Neither verdict has merged; the first to merge is in force, and the other meets it there.",
      ],
      detectedAt: isoSeconds(context.now()), resolution: null,
    };
    const record = conflictRecord({ repository: context.checkout.repositoryKey, base: view.start, question }, { a: { side: mine, files }, b: { side: theirs, files } });
    return { conflict, files, record, positions: { a: verdictPosition(ours), b: verdictPosition(other) } };
  });
}

/**
 * The stored conflicts with each one settled by a verdict the ledger now reopens (its winner's position moved) open
 * again when this pass finds it again, quietly: at `ambient`, the dimmer's state reset. The reopening is stored at once, so the pass's own write keeps
 * it as any open conflict, and a person's answer given meanwhile still stands.
 */
function reopenMoved(context: DetectContext, stored: readonly StoredConflict[], detected: readonly StoredConflict[], ledger: LedgerView): StoredConflict[] {
  const reopened = new Set(ledger.items.filter(item => item.status === "reopened").map(item => item.entry.id));
  const found = new Set(detected.map(entry => entry.conflict.id));
  const moved = (entry: StoredConflict): boolean => entry.conflict.status === "resolved" && found.has(entry.conflict.id) && reopened.has(entry.conflict.resolution?.logId ?? "");
  if (!stored.some(moved)) return [...stored];
  const root = context.checkout.checkoutPath;
  const reopen = (entries: readonly StoredConflict[]): StoredConflict[] => entries.map(entry => (moved(entry) ? { ...entry, conflict: { ...entry.conflict, ...undimmed(entry.conflict.id), level: "ambient", status: "open", resolution: null } } : entry));
  context.database.transaction(tx => {
    if (mergeAssistEnabled(tx, context.checkout.repositoryKey)) writeConflicts(tx, root, reopen(readConflicts(tx, root)), context.now());
  }, { behavior: "immediate" });
  return reopen(stored);
}

/**
 * Find the checkout's conflicts and store them with the ones already settled; new ones a verdict on the same dispute
 * answers are settled by precedent (local state only), and every open one gets its level from the dimmer. A detection
 * that finishes after Merge Assist was turned off stores nothing.
 */
export async function detectConflicts(context: DetectContext): Promise<void> {
  const root = context.checkout.checkoutPath;
  const view = mineView(root);
  if (view === null) return;
  const ledger = await refreshLedger(context.database, root, context.pulls, context.now());
  await refreshFollowed(context.database, root, context.now());
  const touches = new Map<string, Set<string>>();
  const detected = [...overlapConflicts(context, view, touches), ...await decisionConflicts(context, view), ...verdictConflicts(context, view, ledger)];
  const stored = reopenMoved(context, readConflicts(context.database, root), detected, ledger);
  const merged = mergeDetected(stored, detected);
  const fresh = newConflicts(stored, merged);
  const settled = new Map(settleByPrecedent(context, fresh, ledger).map(entry => [entry.conflict.id, entry]));
  const result = merged.map(entry => settled.get(entry.conflict.id) ?? entry);
  const levelled = levelConflicts({
    db: context.database, root, base: view.base, start: view.start, head: view.head, person: view.person, sessionId: context.sessionId,
    pulls: context.pulls, verdict: question => hasVerdict(ledger, question), touches, now: context.now(),
  }, result);
  context.database.transaction(tx => {
    if (!mergeAssistEnabled(tx, context.checkout.repositoryKey)) return;
    // A person's answer given while this ran stands.
    writeConflicts(tx, root, mergeDetected(readConflicts(tx, root), levelled.entries), context.now());
    saveBudget(tx, view.person, levelled.grants, context.now());
  }, { behavior: "immediate" });
}
