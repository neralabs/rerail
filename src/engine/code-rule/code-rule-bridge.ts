import { createHash, randomBytes } from "node:crypto";
import { constants, type Dirent, type Stats } from "node:fs";
import { access, type FileHandle, lstat, mkdtemp, open, opendir, realpath, unlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import type {
  AuditorCloseOptions,
  CodeRuleEdit,
  CodeRuleEditBatch,
  CodeRuleAuditor,
  CodeRuleResponse,
  CodeRuleCheckpoint,
  AuditSettings,
  CodeRuleSourceLanguage,
  IgnoredEditMatch,
  WalkView,
} from "./code-rule.js";
import type { AuditMode } from "../audit/audit.js";
import { fileStamp, stampTime } from "../file-stamp.js";
import {
  gitBranchTips,
  gitCommitsBetween,
  gitHead,
  gitModifiedOrUntrackedListing,
  gitModifiedOrUntrackedPaths,
  GitReads,
  gitUnmergedReader,
  headMovedOnlyByCommits,
  readGitBlobIds,
  readGitBlobs,
  type ChangedPathsListing,
  type CrlfRule,
} from "../git.js";
import { readGitCrlfPaths } from "../git-crlf-reader.js";
import { readGitHead } from "../git-head-reader.js";
import type { RewoundPaths } from "./rerail-steer.js";
import { compareDeterministicStrings } from "../stable-json.js";
import { removeScratch, renameWithRetry } from "../file-replace.js";
import { blobIds, heldAs, textAs } from "./line-endings.js";
import { openNoFollow } from "../no-follow.js";
import { sourceLanguageOfPath, UNLISTED_TEXT_LANGUAGE } from "./code-rule-languages.js";
import { IGNORED_DIRECTORIES } from "./ignored-directories.js";
import { binaryPrefix } from "./binary-prefix.js";
import { sensitiveByName } from "../path-boundary.js";
import { containsSecret, scrubSecrets } from "../secret-scrub.js";
import { IgnoreRuleFiles, type IgnoreRulesChange, shownIgnoreFile } from "./ignore-rule-files.js";
import { type EndedCalls, ToolCallWindows } from "./tool-call-windows.js";
import { capturedOtherThanMade, placesBeforeWrite, type ReportedPreimage, reportedText, textBeforeWrite, unprovenCaptureBefore } from "./tool-result/reported-preimage.js";
import { type LentBytes, WalkReadBuffers } from "./walk-read-buffers.js";
import {
  type WatcherWalk, type WorktreeFindings, type WorktreeScan, type WorktreeScanned, type WorktreeSnapshot, WorktreeView, type WorktreeWalk, WorktreeWalks,
  type WorktreeWatch,
} from "./worktree-walks.js";

export { WORKTREE_WALK_GAP_MS } from "./worktree-walks.js";


/** The name of the ignore files a walk observes whatever Git says of them. */
const GITIGNORE = ".gitignore";
const MAX_SOURCE_BYTES = 1_000_000;
/** The read error of a file past `MAX_SOURCE_BYTES`, never read whole. */
const OVERSIZE_ERROR = `Source exceeds ${MAX_SOURCE_BYTES} byte policy limit`;
/** The most source text the session keeps as later preimages (`#texts`); the least recently changed goes first. */
const MAX_SESSION_TEXT_BYTES = 64 * 1024 * 1024;
/**
 * The most of that the text of sources first seen with changes Git does not hold may take (`first-sight`): every
 * session holds it from its start, so it is kept smaller; a source past it falls back to Git.
 */
const FIRST_SIGHT_TEXT_BYTES = 8 * 1024 * 1024;
/** Paths observed per wave; on network or virtualised mounts each lstat/read is a round trip, so latency, not bandwidth, bounds the walk. */
const OBSERVATION_CONCURRENCY = 16;
/**
 * How many listings asked again (`#startListingAgain`) hold a slot to ask Git at once, and how long one holds its slot
 * at most before the next listing asked takes it: until Git answers, or `LISTING_AGAIN_WAIT_MS`. The slots are each
 * session's own (one bridge per session), so every count here holds in each session. Git lists a checkout in
 * milliseconds, so no more than `LISTINGS_AGAIN_AT_ONCE` run at once in each session while each answers in time, and a
 * slow one keeps the rest waiting no more than a second. Four slots list 72 late checkouts within the second or two
 * before an agent's next tool call on a busy Windows machine, where one or two do not. When every listing is slow, four
 * more start each second, so up to about 44 run at once in each session until Git's 10 s limit ends the first.
 */
const LISTINGS_AGAIN_AT_ONCE = 4;
const LISTING_AGAIN_WAIT_MS = 1_000;

/**
 * A source as a tool call found it: its text, why it could not be read, or neither when it did not exist. A file named
 * like one holding secrets (`sensitiveByName`) keeps only its SHA-256 and size (`withheld`): preimages go into the
 * session ledger in the checkout, so its text is never captured, and `rewind` refuses to put it back. A preimage whose
 * text holds a secret is written to the ledger the same way ({@link ledgerPreimage}), though the session keeps its text,
 * and so is one of a file named so only where it really is (`secretTarget`).
 */
export interface Preimage {
  readonly text?: string;
  readonly readError?: string;
  readonly withheld?: { readonly sha256: string; readonly bytes: number };
  /**
   * The text is of a file named like one holding secrets where it really is (`Source.secretTarget`), not as the call
   * spells it: the session keeps it, so `rewind` can put the file back, and the ledger keeps only its digest and size.
   */
  readonly secretTarget?: true;
}

/**
 * `preimage` as the session ledger records it: text holding a secret (`containsSecret`), or of a file named like one
 * holding secrets where it really is (`secretTarget`), is withheld, its SHA-256 and size kept, so a session resumed from
 * the ledger still finds the file unchanged by its digest but cannot put it back; a read error is scrubbed. The live
 * session keeps the text it captured.
 */
export function ledgerPreimage(preimage: Preimage): Preimage {
  if (preimage.text !== undefined && (preimage.secretTarget === true || containsSecret(preimage.text))) {
    return { withheld: { sha256: createHash("sha256").update(preimage.text).digest("hex"), bytes: Buffer.byteLength(preimage.text) } };
  }
  return preimage.readError === undefined ? preimage : { ...preimage, readError: scrubSecrets(preimage.readError) };
}

export interface Source {
  readonly path: string;
  readonly language: CodeRuleSourceLanguage;
  readonly text?: string;
  readonly deleted?: boolean;
  readonly readError?: string;
  readonly stamp?: string;
  readonly digest?: string;
  readonly modifiedAt?: number;
  /** Filesystem-clock reading taken before the walk that produced this observation. */
  readonly observedAt?: number;
  /**
   * The file is named like one holding secrets where it really is, from the checkout that owns it, however `path`
   * spells it: a link to a folder on the way (`conf/config.py`, `conf` a link to `.aws`) or a Windows short name
   * (`AWS~1/config.py`) may lead there.
   */
  readonly secretTarget?: true;
}
export type ObservedSource = Omit<Source, "text" | "deleted">;
/** Baseline observations keyed by workspace directory, shared so later sessions reuse digests of unchanged sources. */
export type SourceObservationCache = Map<string, ReadonlyMap<string, ObservedSource>>;
/**
 * Where a session's source walks stood when its runtime suspended it (`CodeRuleBridge.walkResume`), for the walks of the
 * session's next open to go on from: what was written since `since` is judged as the open session's next walk would have
 * judged it, so another writer's change made while the session was suspended is recorded, not taken as the baseline.
 */
export interface SessionWalkResume {
  /** Filesystem-clock reading: the earlier start of the last finished walks of the session folder and of the other worktrees. */
  readonly since: number;
  /** Each source whose text the evaluator held as the session's (`HELD_BY_EVALUATOR`), as last observed; undefined when gone. */
  readonly held: ReadonlyMap<string, ObservedSource | undefined>;
  /**
   * What the walks last saw of Git (`SourceDiscovery.heads`, `unmerged` and `passedCheckouts`): one `HEAD` and the
   * unmerged entries per checkout, so the first walk's Git verdicts compare with what was there before the suspension,
   * as the open session's next walk would.
   */
  readonly gitSeen: WalkedGit;
}
/** A walk's snapshot of Git, by checkout root (`SessionWalkResume.gitSeen`). */
export interface WalkedGit {
  readonly heads: ReadonlyMap<string, string>;
  readonly unmerged: ReadonlyMap<string, ReadonlyMap<string, string>>;
  readonly passedCheckouts: ReadonlySet<string>;
}
/** What a session opened fresh has seen of Git: nothing yet. */
const NO_GIT_SEEN: WalkedGit = { heads: new Map(), unmerged: new Map(), passedCheckouts: new Set() };
/**
 * A file the auditor may read: `unlisted` when no language lists it, so it is read as `UNLISTED_TEXT_LANGUAGE` and,
 * being binary (a NUL byte in its first bytes, `binaryPrefix`, tested before it is read whole) or no regular file, is
 * no source at all and skipped without a read error.
 */
interface SourceLocation {
  readonly absolute: string;
  readonly path: string;
  readonly language: CodeRuleSourceLanguage;
  readonly unlisted: boolean;
}
/** Where `path` is read from and with which label: its listed one, else `UNLISTED_TEXT_LANGUAGE`. */
function sourceLocation(absolute: string, path: string): SourceLocation {
  const language = sourceLanguageOfPath(path);
  return { absolute, path, language: language ?? UNLISTED_TEXT_LANGUAGE, unlisted: language === undefined };
}
/**
 * `source` as a baseline keeps it: without its text or deletion mark. A source that carries neither is kept as it is, so
 * every session whose walk found it unchanged holds the one object the runtime's shared observations hold, not a copy
 * of every file of the checkout each.
 */
function observation(source: Source): ObservedSource {
  if (!("text" in source) && !("deleted" in source)) return source;
  const { text: _text, deleted: _deleted, ...observed } = source;
  return observed;
}
interface PendingEdit {
  readonly paths: readonly string[];
  readonly preimages: ReadonlyMap<string, Preimage>;
}

/** A tool call the session's ledger holds open as the session opens, with what it captured as it began. */
export interface HeldCall {
  readonly toolCallId: string;
  readonly paths: readonly string[];
  readonly preimages: Readonly<Record<string, Preimage>>;
}
/**
 * A finished edit `rewind` may still roll back: the revision its finish audited, and each path's digest after it (none
 * when absent). `fromCapture` when finish read every file against the call's own capture or its very text
 * (`readAsCaptured`): an edit read otherwise, against what the tool's result says the file held or over the whole
 * file, has a capture that may hold the tool's write, as when the call reached Rerail only after the tool ran. Written
 * back, it would undo only what changed the file since and say the edit was rolled back, so such an edit is never
 * rolled back.
 */
interface RewindableEdit {
  readonly revision: number;
  readonly preimages: ReadonlyMap<string, Preimage>;
  readonly after: ReadonlyMap<string, string | undefined>;
  readonly fromCapture: boolean;
}
/** What `finish` takes besides the call: whether `rewind` may roll its edit back, and what its result says the file held. */
interface FinishOptions {
  readonly rewindable?: boolean;
  readonly reported?: ReportedPreimage | undefined;
}
/** A source as a preimage (`textPreimage` for its text). Undefined when it is none. */
function preimageOf(source: Source): Preimage | undefined {
  if (source.text !== undefined) return textPreimage(source, source.text);
  if (source.readError !== undefined) return { readError: source.readError };
  return source.deleted === true ? {} : undefined;
}
/**
 * `source`'s `text` as a preimage: a file named like one holding secrets as spelled keeps only its digest and size; one
 * named so only where it really is (`secretTarget`) keeps its text for the session, marked so the ledger keeps only its
 * digest and size too (`ledgerPreimage`).
 */
function textPreimage(source: Source, text: string): Preimage {
  if (sensitiveByName(source.path)) return { withheld: { sha256: source.digest!, bytes: Buffer.byteLength(text) } };
  return source.secretTarget === true ? { text, secretTarget: true } : { text };
}
function holdsSecrets(source: Source): boolean {
  return source.secretTarget === true || sensitiveByName(source.path);
}
/**
 * `source`, read at its spelled path, marked `secretTarget` when `real`, where it really is from the checkout that owns
 * it, is named like a file holding secrets.
 */
function judgedWhereItLeads(source: Source | undefined, real: string): Source | undefined {
  return source === undefined || !sensitiveByName(real) ? source : { ...source, secretTarget: true };
}
/**
 * What a tool's result says `source` held, as its edit is read against it: a file named like one holding secrets, as
 * spelled or where it really is (`holdsSecrets`), keeps only its digest and size, so its edit is read over the whole
 * file, as a direct edit of it is. An edit read against what a result says is never rolled back (`RewindableEdit`), so
 * the session needs no copy of that text.
 */
function keptPreimage(source: Source, preimage: Preimage): Preimage {
  if (preimage.text === undefined || !holdsSecrets(source)) return preimage;
  return { withheld: { sha256: sha256(preimage.text), bytes: Buffer.byteLength(preimage.text) } };
}
/**
 * The digest a tool edit's source had before it when its text was not captured: the one observed before a call whose start
 * was not seen, or a secret-like file's, which finds it again where this session or Git holds it.
 */
function uncapturedPrior(edit: CodeRuleEdit, preimage: Preimage | undefined, before: string | undefined): string | undefined {
  if (edit.generatedText === undefined) return undefined;
  return preimage === undefined ? before : preimage.withheld?.sha256;
}
/**
 * What Git decided about a change: it was Git's (`keptAs` when the text it left is held by neither the index nor `HEAD`:
 * a stopped merge wrote it, or it was restored from a branch), or the text Git's move proves it was changed from.
 */
interface GitVerdict {
  readonly previousText?: string;
  readonly keptAs?: "git";
}
/** What Git holds for a walk's changes in one checkout, as one reading of its verdicts finds it (`#gitVerdicts`). */
interface GitBlobs {
  /** Each change's index blob, its conflict stages, its `HEAD` blob and, when `HEAD` moved, its blob at `before`. */
  readonly blobs: ReadonlyMap<string, Buffer>;
  /** `HEAD` now. */
  readonly head: string;
  /** `HEAD` moved since the last walk... */
  readonly moved: boolean;
  /** ...only by commits, which write no file. */
  readonly committed: boolean;
  /** `HEAD` as the last walk saw it, when it moved since. */
  readonly before: string | undefined;
  /** The changes whose blobs a checkout writes with CRLF line breaks, each with the rule it writes them by (`gitCrlfPaths`). */
  readonly crlf: ReadonlyMap<string, CrlfRule>;
  /** The index's unmerged entries now (`gitUnmergedReader`); undefined when Git could not list them. */
  readonly unmerged: ReadonlyMap<string, string> | undefined;
  /** Those entries as the last walk saw them; undefined when no walk has read them yet. */
  readonly unmergedBefore: ReadonlyMap<string, string> | undefined;
}
interface GitView extends GitBlobs {
  /** A merge, rebase or cherry-pick stopped on a conflict, having not moved `HEAD` for its work so far. */
  readonly stopped: boolean;
}
async function readGitView(discovery: SourceDiscovery, checkout: string, paths: readonly string[]): Promise<GitBlobs | undefined> {
  const { git } = discovery;
  const head = await git.run(signal => readGitHead(checkout, { signal }));
  if (head === undefined) return undefined;
  const from = discovery.heads.get(checkout);
  const unmerged = await readUnmergedView(discovery, checkout);
  const moved = from !== head;
  const committed = moved && await git.run(signal => headMovedOnlyByCommits(checkout, from, { signal }));
  const before = from === head ? undefined : from;
  const [blobs, crlf] = await Promise.all([
    // A blob larger than a source could never match its digest: it is skipped, never read into memory.
    git.run(signal => readGitBlobs(checkout, paths.flatMap(path => gitRevisions(path, before)), { maxBlobBytes: MAX_SOURCE_BYTES, signal })),
    git.run(signal => readGitCrlfPaths(checkout, paths, { signal })),
  ]);
  return { blobs, head, moved, committed, before, crlf, ...unmerged };
}
/**
 * A checkout's unmerged entries now (`SourceDiscovery.readUnmerged`), read through the session's Git reads, which `close`
 * stops and waits out as it does every other Git a walk starts.
 */
function unmergedNow(discovery: SourceDiscovery, checkout: string): Promise<ReadonlyMap<string, string> | undefined> {
  return discovery.git.run(signal => discovery.readUnmerged(checkout, { signal }));
}
/** Puts `root`'s unmerged entries now in `unmerged`; a checkout whose entries Git could not list is left out. */
async function readEntriesInto(discovery: SourceDiscovery, root: string, unmerged: Map<string, ReadonlyMap<string, string>>): Promise<void> {
  const entries = await unmergedNow(discovery, root);
  if (entries !== undefined) unmerged.set(root, entries);
}
/** `HEAD` of each of `roots` that names a commit, with its unmerged entries put in `unmerged`. */
async function readHeads(discovery: SourceDiscovery, roots: Iterable<string>, unmerged: Map<string, ReadonlyMap<string, string>>): Promise<Map<string, string>> {
  const heads = new Map<string, string>();
  for (const root of roots) {
    const head = await discovery.git.run(signal => readGitHead(root, { signal }));
    if (head === undefined) continue;
    heads.set(root, head);
    await readEntriesInto(discovery, root, unmerged);
  }
  return heads;
}
/** A checkout's unmerged entries now and as the last walk saw them (`keepFirstSnapshot` takes the first ones). */
async function readUnmergedView(discovery: SourceDiscovery, checkout: string): Promise<Pick<GitBlobs, "unmerged" | "unmergedBefore">> {
  const unmerged = await unmergedNow(discovery, checkout);
  return { unmerged, unmergedBefore: discovery.unmerged.get(checkout) };
}
/**
 * Where no walk has taken a checkout's snapshot yet, makes it the `HEAD` and unmerged entries the standing reading of
 * its Git verdicts read. Only that reading leaves one, so each reading of a walk is judged from what the last walk saw,
 * never from an earlier reading's.
 */
function keepFirstSnapshot(discovery: SourceDiscovery, checkout: string, view: GitBlobs): void {
  if (!discovery.heads.has(checkout)) discovery.heads.set(checkout, view.head);
  if (!discovery.unmerged.has(checkout) && view.unmerged !== undefined) discovery.unmerged.set(checkout, view.unmerged);
}
/**
 * Whether a merge, rebase or cherry-pick stopped on a conflict since the last walk: the unmerged entries changed. A
 * modify/delete conflict can leave its file as it was, so the walk's changes need not name what stopped Git. With no
 * entries to compare (Git could not list them, or no walk has read this checkout's yet, so a conflict older than the
 * session would read as new), the changes that a conflict wrote (`conflictedByGit`) are all there is to go by.
 */
function stoppedSince(view: GitBlobs, named: ReadonlyArray<{ path: string; prior: string | undefined }>): boolean {
  const { unmerged, unmergedBefore } = view;
  if (unmerged === undefined || unmergedBefore === undefined) return named.some(({ path, prior }) => conflictedByGit(view, path, prior));
  return [...unmerged].some(([path, entries]) => unmergedBefore.get(path) !== entries);
}
/** The commits that moved `HEAD` since the last walk: none when something else moved it, undefined when Git cannot list them. */
async function commitsMade(git: GitReads, checkout: string, view: GitBlobs): Promise<string[] | undefined> {
  if (!view.committed) return [];
  const { before, head } = view;
  const commits = before === undefined ? [head] : await git.run(signal => gitCommitsBetween(checkout, before, head, { signal }));
  return commits.length === 0 ? undefined : commits;
}
/** A walk's change in one checkout: its path there and, when its text was not captured, the digest it had before. */
interface CheckoutEdit {
  readonly edit: CodeRuleEdit;
  readonly path: string;
  readonly prior: string | undefined;
}
/** One reading of a checkout's Git verdicts (`#gitVerdicts`), and the view of Git it was made from. */
interface VerdictReading {
  readonly verdicts: Map<string, GitVerdict>;
  readonly view: GitBlobs;
}
/**
 * How many readings of a checkout's Git verdicts a walk makes at most, while each finds a change Git's and `HEAD` moved
 * as it was read: a command that commits as the first reading is made is read through by the second. A `HEAD` that
 * moves through all of them belongs to a command still committing, or a rebase at work, which no reading holds for.
 */
const GIT_VERDICT_READS = 3;
/** Whether a reading takes a change for Git's: one with no text Git's move proves it was changed from. */
function findsGitWrite(verdicts: ReadonlyMap<string, GitVerdict>): boolean {
  return [...verdicts.values()].some(verdict => verdict.previousText === undefined);
}
/**
 * Whether `checkout`'s `HEAD` still names `head`, asked of Git itself through the session's Git reads, not of its files
 * (`readGitHead`): only a reading that takes a change for Git's asks, so a commit made meanwhile is Git's to name.
 */
async function headStill(discovery: SourceDiscovery, checkout: string, head: string): Promise<boolean> {
  return await discovery.git.run(signal => gitHead(checkout, { signal })) === head;
}
function gitRevisions(path: string, before: string | undefined): string[] {
  const revisions = [`:./${path}`, `:2:./${path}`, `:3:./${path}`, `HEAD:./${path}`];
  return before === undefined ? revisions : [...revisions, `${before}:./${path}`];
}
/** Whether a file at `path` whose bytes have `digest` holds the blob `revision` names, in either form (`heldAs`). */
function holdsBlob(view: GitBlobs, path: string, revision: string, digest: string): boolean {
  const blob = view.blobs.get(revision);
  return blob !== undefined && heldAs(blob, digest, view.crlf.get(path)) !== undefined;
}
function heldBefore(view: GitBlobs, path: string, digest: string): boolean {
  return view.before !== undefined && holdsBlob(view, path, `${view.before}:./${path}`, digest);
}
/**
 * A file stays unmerged after the agent edits it: Git wrote it only if it changed from what Git started the merge on,
 * and only in the walk its unmerged entries changed in. A later change to it is the session's, even to the blob of its side.
 */
function conflictedByGit(view: GitBlobs, path: string, prior: string | undefined): boolean {
  if (!view.blobs.has(`:2:./${path}`) && !view.blobs.has(`:3:./${path}`)) return false;
  if (conflictSeenBefore(view, path)) return false;
  return prior === undefined || holdsBlob(view, path, `:2:./${path}`, prior) || heldBefore(view, path, prior);
}
/** `path` has the unmerged entries the last walk saw: the conflict was there before this walk's changes. */
function conflictSeenBefore(view: GitBlobs, path: string): boolean {
  const entries = view.unmerged?.get(path);
  return entries !== undefined && view.unmergedBefore?.get(path) === entries;
}
/** A file a move left absent from `HEAD` was removed by it; while a merge is stopped, so was one the index no longer has. */
function removedByGit(view: GitView, path: string): boolean {
  return (view.moved && !view.committed && !view.blobs.has(`HEAD:./${path}`)) || (view.stopped && !view.blobs.has(`:./${path}`));
}
/** A file holding `HEAD`'s blob, unless only commits moved `HEAD`; while a merge is stopped, one holding its index blob. */
function holdsGitBlob(view: GitView, path: string, after: string): boolean {
  return (!view.committed && holdsBlob(view, path, `HEAD:./${path}`, after)) || (view.stopped && holdsBlob(view, path, `:./${path}`, after));
}
/**
 * The text the change was made from, when it held the file's blob at `before`: `HEAD`'s, unless only commits moved it,
 * else that one, in the line breaks the file held it in.
 */
function movedFromText(view: GitBlobs, path: string, prior: string | undefined): GitVerdict | undefined {
  if (view.before === undefined) return undefined;
  const form = heldAs(view.blobs.get(`${view.before}:./${path}`), prior, view.crlf.get(path));
  if (form === undefined) return undefined;
  const earlier = view.committed ? undefined : view.blobs.get(`HEAD:./${path}`);
  const previous = earlier ?? view.blobs.get(`${view.before}:./${path}`);
  return previous === undefined ? undefined : { previousText: textAs(previous, form) };
}
/** The read error of a file too large to be a source or holding binary data, found once it was read. */
const NOT_TEXT_ERROR = "Source exceeds its byte budget or contains binary data";
/** What a walk observes of every file it looks at, before any read. */
type StampedSource = Pick<Source, "path" | "language"> & { readonly stamp: string; readonly observedAt: number; readonly modifiedAt: number };
function knownDigest(previous: ObservedSource | undefined): Pick<Source, "digest"> {
  return previous?.digest === undefined ? {} : { digest: previous.digest };
}
/** A file observed without a read: an unlisted binary one, with no read error, or one too large to be a source. Undefined to read it. */
async function unreadFile(location: SourceLocation, info: Stats): Promise<Pick<Source, "readError"> | undefined> {
  if (location.unlisted && await binaryPrefix(location.absolute)) return {};
  return info.size > MAX_SOURCE_BYTES ? { readError: OVERSIZE_ERROR } : undefined;
}
/** A read file that is no source text: nothing, for an unlisted binary one, else a read error. Undefined for text. */
function nonText(location: SourceLocation, source: Buffer): Pick<Source, "readError"> | undefined {
  if (location.unlisted && source.includes(0)) return {};
  return source.length > MAX_SOURCE_BYTES || source.includes(0) ? { readError: NOT_TEXT_ERROR } : undefined;
}
/** The latest stamp of a file a walk read, which a shell may have written while it read. */
async function modifiedSince(absolute: string, modifiedAt: number): Promise<number> {
  const after = await lstat(absolute);
  return Math.max(modifiedAt, after.mtimeMs, after.ctimeMs);
}
/**
 * The stamp a walk gives what it read: what the file was stamped when the walk looked at it, or, where a shell may have
 * written it while the read was pending, its stamp after the read. A baseline walk takes the later one, so it does not
 * take bytes written meanwhile as the baseline. So does a walk that finds a command of the session it holds sources back
 * for as its read ends (`#toolChanges`): the command may have begun after the walk looked, and be part way through what
 * the walk read. With none, none the walk holds sources back for began before the bytes were read. The original `stamp`
 * is kept either way, so the next walk reads the file again. A file such a walk cannot stamp after its read (gone, as
 * when the command emptied and then removed it) is stamped now, so the command holds it back too. So is one whose times
 * after the read do not date the bytes it read (`undated`, `undatedRead`). A baseline walk gives such a read no stamp
 * and keeps what was seen of the file before (`baselineStamp`).
 */
async function stampOfRead(walk: SourceWalk, absolute: string, modifiedAt: number, undated: (at: number) => boolean): Promise<number | undefined> {
  if (!walk.captureChanges) return baselineStamp(walk, await modifiedSince(absolute, modifiedAt), undated);
  if (walk.commandRuns?.() !== true) return modifiedAt;
  const after = await modifiedSince(absolute, modifiedAt).catch(() => undefined);
  return after === undefined || undated(after) ? Math.max(modifiedAt, await filesystemClock()) : after;
}
/**
 * A baseline walk's stamp of what it read: the file's times after the read, `after`; none where a command of the session
 * runs and those times do not date the bytes (`undated`). The walk then keeps what was seen of the file before
 * (`#readObservedFile`): what the session saw before a quiet spell, or what the folder's shared observations hold. It
 * cannot hold the file back as a later walk does (`#toolChanges`) where the session has seen nothing of its own.
 */
function baselineStamp(walk: SourceWalk, after: number, undated: (at: number) => boolean): number | undefined {
  return walk.commandRuns?.() === true && undated(after) ? undefined : after;
}
/**
 * Whether a walk read other bytes than `previous`, the observation of the file it compares with (the session's last, or
 * in a baseline walk the folder's shared one), though the file's times, `at`, have not moved past that observation's.
 * Those times do not say when the bytes were written: a file system can show a change before it stamps it, as Linux
 * empties a file it truncates before it gives the file its new times.
 */
function undatedRead(previous: ObservedSource | undefined, bytes: Buffer, at: number): boolean {
  return previous?.stamp !== undefined && at <= stampTime(previous.stamp) && previous.digest !== sha256(bytes);
}
/**
 * A text file as a walk observes it, with its text only when it may be an edit (`#discoveredEdit`): its digest is not
 * the one observed before, and it was stamped since the session's start (`changedSinceStart`), or the walk captures
 * changes and this is no first sight of a baseline (`firstSightIsBaseline`). A walk reads every file it holds no digest
 * for, such as each file of a worktree it walks first, and text of those would be a string per file that nothing reads.
 */
function observedText(walk: SourceWalk, stamped: StampedSource, previous: ObservedSource | undefined, read: LentBytes): Source {
  const digest = sha256(read.bytes);
  const changed = previous?.digest !== digest
    && (changedSinceStart(walk.discovery, stamped) || (walk.captureChanges && !firstSightIsBaseline(walk, stamped.path, previous)));
  return { ...stamped, digest, ...(changed ? { text: read.text() } : {}) };
}
/**
 * The walk sees `path` for the first time under one of its `baselineRoots`, so `#discoveredEdit` takes it as a baseline
 * unless it was stamped since the session's start (`changedSinceStart`).
 */
function firstSightIsBaseline(walk: SourceWalk, path: string, previous: ObservedSource | undefined): boolean {
  return previous === undefined && underBaselineRoot(walk.baselineRoots, path);
}
/**
 * An unlisted file too large to read is unread, reported once per stamp (`newlyUnread`); any other read error is the walk's.
 * A file seen before in a folder the walk could not list keeps the folder's error (`failFolder`), and the folder's gap
 * stands for it: the walk's gaps are the same whether or not a walk saw the folder's files before.
 */
function recordReadError(scan: SourceScan, source: Source): void {
  if (source.readError === undefined || underBaselineRoot(scan.failedRoots, source.path)) return;
  if (unreadOversize(source)) scan.unread.set(source.path, { stamp: source.stamp, readError: source.readError });
  else scan.errors.push({ path: source.path, readError: source.readError });
}
/**
 * Notes each file the session folder's walk could not read, with the calls that may have written it unseen
 * (`SourceDiscovery.missedFiles`). Whether the walk lets the other calls go: it gave up on no path it met
 * (`SourceScan.gaveUp`), so it read every file but those, and the calls it keeps for them claim only their writes
 * (`ToolCallWindows.ranDuring`).
 */
function holdMissedFiles(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): boolean {
  discovery.missedFiles = missedFiles(discovery, walk, scan);
  if (scan.gaveUp === true) return false;
  discovery.listedEnds = walk.callsEnded;
  return true;
}
/**
 * Each file the walk could not read, with the calls it keeps: from those that ended after the last walk that listed
 * every folder before its first miss began, up to this walk's start. A file under a folder the walk could not list is
 * one: the walk tried no read of it. `failFolder` finds a source there only for a file the session observes, and a file
 * the last walk missed may have none (a tool call pending on it, or a read of it that failed, leaves it unobserved), so
 * each such file the last walk missed stays missed here with its calls.
 */
function missedFiles(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): Map<string, EndedCalls> {
  const missed = new Map<string, EndedCalls>();
  for (const [path, source] of scan.sources) if (missedRead(source)) missed.set(path, missedThrough(discovery, path, walk.callsEnded));
  for (const path of discovery.missedFiles.keys())
    if (!missed.has(path) && underBaselineRoot(scan.failedRoots, path)) missed.set(path, missedThrough(discovery, path, walk.callsEnded));
  return missed;
}
/** A file whose read failed: an error with no stamp. */
function missedRead(source: Source): boolean {
  return source.readError !== undefined && source.stamp === undefined;
}
/** The calls the file at `path` keeps as a walk that began once `through` calls had ended misses it. */
function missedThrough(discovery: SourceDiscovery, path: string, through: number): EndedCalls {
  return { after: discovery.missedFiles.get(path)?.after ?? discovery.listedEnds, through };
}
function unreadOversize(source: Source): source is Source & { readonly stamp: string } {
  return source.readError === OVERSIZE_ERROR && source.stamp !== undefined && sourceLanguageOfPath(source.path) === undefined;
}
/**
 * The walk gave up on a path it met (`SourceScan.gaveUp`), whether its failure is a place or not: the one way a walk
 * says it did not see every file.
 */
function gaveUpOn(scan: SourceScan): void {
  scan.gaveUp = true;
}
/** A folder the walk could not read: its first sight next time is a baseline, and each source seen under it keeps the error. */
function failFolder(walk: SourceWalk, path: string, readError: string): void {
  const { previousSources, scan } = walk;
  scan.errors.push({ path, readError });
  scan.failedRoots.add(path);
  gaveUpOn(scan);
  const failed = new Set([path]);
  for (const [sourcePath, source] of previousSources)
    if (underBaselineRoot(failed, sourcePath)) scan.sources.set(sourcePath, { path: sourcePath, language: source.language, readError });
}
/** Whether the tool call left the source as its preimage: the same text, or a secret-like file's same digest. */
function unchangedSince(source: Source, preimage: Preimage | undefined): boolean {
  return preimage?.withheld === undefined ? preimage?.text === source.text : preimage.withheld.sha256 === source.digest;
}
/** Why a tool edit whose call may have come after the tool wrote is checked over its whole file (`editPreimage`). */
const UNREBUILT_PREIMAGE = "The tool's result no longer fits the file, so what it held before the tool ran is unknown";
/**
 * The preimage a tool edit of `source` is read against: the call's own capture, or, where the result shows that capture
 * holds what the tool wrote (the call reached Rerail only after the tool ran), what the tool's result says the file
 * held. That capture is the file as it is now, or one the file changed from since (`changedSinceWrite`).
 */
function editPreimage(source: Source, captured: Preimage | undefined, reported: ReportedPreimage | undefined): Preimage | undefined {
  if (reported === undefined || captured === undefined) return captured;
  return unchangedSince(source, captured) ? unchangedSinceWrite(source, captured, reported) : changedSinceWrite(source, captured, reported);
}
/**
 * What the file held before a tool whose call captured the file as it is now, as the tool's result says: that capture
 * holds what the tool wrote, unless the file was put back before the result. Where the result gives no text that fits
 * the file (`reportedText`), the capture is kept where it is the file before the tool (`placesBeforeWrite`): the file
 * was put back, and the edit changed nothing. Otherwise what the file held is unknown: the edit is read as one whose
 * earlier text could not be read, so it is checked over its whole file rather than read as no change.
 */
function unchangedSinceWrite(source: Source, captured: Preimage, reported: ReportedPreimage): Preimage {
  const preimage = reportedText(reported, source.text);
  if (preimage !== undefined) return keptPreimage(source, preimage);
  return captured.text !== undefined && placesBeforeWrite(reported, captured.text) ? captured : { readError: UNREBUILT_PREIMAGE };
}
/**
 * The preimage of an edit whose file changed since its call's capture: the text the result's line diff rebuilds from
 * that capture where the diff fits it, which only a capture taken after the tool wrote does (`textBeforeWrite`), with
 * whatever changed the file since (a formatter, the next edit), as an edit whose call came on time is read. Otherwise
 * the capture, where the line diff places it before the tool (`placesBeforeWrite`), as it places every on-time capture,
 * or where the tool made a file the call read with other text than the tool wrote (`capturedOtherThanMade`).
 * Otherwise, the capture not being the file before the tool, what the result says the file held
 * (`unprovenCaptureBefore`): nothing, for a file the tool made; the text the tool replaced; or the text the diff
 * rebuilds from the capture, which then holds the tool's lines where the diff places them. A result that says none of
 * these shows nothing about the capture, which another writer between the call and the tool may have left as it is, so
 * the edit is read against it, as an edit whose result says nothing is.
 */
function changedSinceWrite(source: Source, captured: Preimage, reported: ReportedPreimage): Preimage {
  const text = captured.text;
  if (text === undefined) return captured;
  const before = textBeforeWrite(reported, text, source.text);
  if (before !== undefined) return keptPreimage(source, before);
  if (placesBeforeWrite(reported, text) || capturedOtherThanMade(reported, text)) return captured;
  const said = unprovenCaptureBefore(reported, text, source.text);
  return said === undefined ? captured : keptPreimage(source, said);
}
/**
 * Whether an edit read against `preimage` was read against what its call captured: that capture, or its very text, as
 * what an on-time Edit's result says the file held is, so `rewind` may write the capture back.
 */
function readAsCaptured(preimage: Preimage | undefined, captured: Preimage | undefined): boolean {
  return preimage === captured || (preimage?.text !== undefined && preimage.text === captured?.text);
}
function absentThroughout(source: Source, preimage: Preimage | undefined): boolean {
  return source.deleted === true && preimage !== undefined && preimage.text === undefined && preimage.readError === undefined;
}
/** What a tool edit's source was before it, as far as its preimage says. */
function previousOf(preimage: Preimage | undefined): Pick<CodeRuleEdit, "previousText" | "previousReadError"> {
  if (preimage === undefined) return { previousReadError: "Original source was not captured for this tool call" };
  if (preimage.withheld !== undefined) return { previousReadError: "Rerail keeps no copy of a file holding secrets" };
  return { ...(preimage.text === undefined ? {} : { previousText: preimage.text }), ...(preimage.readError === undefined ? {} : { previousReadError: preimage.readError }) };
}
/** `absolute` opened without following a final symlink; undefined when it is one. */
async function openSource(absolute: string): Promise<FileHandle | undefined> {
  try {
    return await openNoFollow(absolute, constants.O_RDONLY);
  } catch (error) {
    if (hasErrorCode(error, "ELOOP")) return undefined;
    throw error;
  }
}
/**
 * The text of the file at `absolute` while its bytes have `digest`, else undefined: what a session held of a source
 * before its runtime suspended it. Read as `openSource` opens a file, and only a regular file of a source's size; the
 * digest proves the bytes are the ones a walk observed in place.
 */
async function textWithDigest(absolute: string, digest: string): Promise<string | undefined> {
  const handle = await openedOrNone(absolute);
  if (handle === undefined) return undefined;
  try {
    const info = await handle.stat();
    if (!info.isFile() || info.size > MAX_SOURCE_BYTES) return undefined;
    const bytes = await handle.readFile();
    return sha256(bytes) === digest ? bytes.toString("utf8") : undefined;
  } catch {
    return undefined;
  } finally {
    await handle.close();
  }
}
/** `openSource`, undefined for a file it cannot open (gone, or no longer the session's to read). */
async function openedOrNone(absolute: string): Promise<FileHandle | undefined> {
  try {
    return await openSource(absolute);
  } catch {
    return undefined;
  }
}
/** The opened file as a source: none for an unlisted binary one, a read error for one too large or binary. */
async function readOpenedSource(handle: FileHandle, location: SourceLocation, info: Stats): Promise<Source | undefined> {
  if (location.unlisted && await binaryPrefix(handle)) return undefined;
  if (info.size > MAX_SOURCE_BYTES) return { path: location.path, language: location.language, readError: OVERSIZE_ERROR };
  return textSource(location, info, await handle.readFile());
}
function textSource(location: SourceLocation, info: Stats, source: Buffer): Source | undefined {
  const content = nonText(location, source);
  if (content === undefined)
    return { path: location.path, language: location.language, text: source.toString("utf8"), stamp: fileStamp(info), digest: sha256(source) };
  return content.readError === undefined ? undefined : { path: location.path, language: location.language, readError: content.readError };
}
/** A file `rewind` puts back: where it is, what it held before the edit, and what it holds now. */
interface RollbackTarget {
  readonly location: SourceLocation;
  readonly preimage: Preimage;
  readonly current: Source;
}
/** Whether a steer over `scope` may roll the edit back: the scope lies within its captured paths, and each preimage was read. */
function coversScope(preimages: ReadonlyMap<string, Preimage>, scope: readonly string[]): boolean {
  return scope.length > 0 && scope.every(path => preimages.has(path)) && [...preimages.values()].every(preimage => preimage.readError === undefined);
}
/** The file holds what the edit left: it was read, with the digest the edit's finish saw (none when absent). */
function holdsEditText(current: Source | undefined, digestAfter: string | undefined): current is Source {
  return current !== undefined && current.readError === undefined && current.digest === digestAfter;
}
/** The paths `rewind` put back, and the revision that audited them back (in `log` mode). */
export interface RewoundEdit extends RewoundPaths {
  readonly revision: number;
}

/** Which boundary asked for a discovery pass; a stronger request absorbs weaker ones queued behind the same walk. */
type DiscoveryCheckpoint = "edit" | "turn" | "final";
const DISCOVERY_PRIORITY: Readonly<Record<DiscoveryCheckpoint, number>> = { edit: 0, turn: 1, final: 2 };
function strongerCheckpoint(checkpoint: DiscoveryCheckpoint, other: DiscoveryCheckpoint | undefined): DiscoveryCheckpoint {
  return other !== undefined && DISCOVERY_PRIORITY[other] > DISCOVERY_PRIORITY[checkpoint] ? other : checkpoint;
}
function discoveryCheckpoints(checkpoint: DiscoveryCheckpoint): CodeRuleCheckpoint[] {
  return checkpoint === "edit" ? ["edit", "function"] : [checkpoint];
}

/**
 * What the session took last from the walk of the repository's other worktrees (`#scheduleWorktreeWalk`), which the
 * sessions watching them share and which is kept apart from the session folder's.
 */
interface WorktreeDiscovery {
  /** The other worktrees, relative to the session folder. */
  roots: ReadonlySet<string>;
  /** Those whose first walk is still to finish: as with `SourceDiscovery.baselineRoots`, what it finds is a baseline. */
  readonly fresh: Set<string>;
  /** Subtrees the last walk could not read: what it first finds there is a baseline too. */
  baselineRoots: Set<string>;
  /** The session's watch of the shared walk, from its first request on. */
  watch: WorktreeWatch | undefined;
  /** What the last walk could not check, reported at each boundary as the session walk's `gaps` are. */
  gaps: readonly WalkGap[];
  /** The unlisted files the last walk found too large to read, as `SourceDiscovery.unread`. */
  unread: ReadonlyMap<string, { readonly stamp: string; readonly readError: string }>;
  /** Filesystem-clock reading taken as the last finished walk began; undefined until one finishes. */
  walkedFrom: number | undefined;
}

function worktreeDiscovery(): WorktreeDiscovery {
  return {
    roots: new Set(), fresh: new Set(), baselineRoots: new Set(), watch: undefined, gaps: [], unread: new Map(), walkedFrom: undefined,
  };
}

/** What a session holds, as a walk of the other worktrees begins, to take what it finds. */
interface WorktreeStart {
  /** `WorktreeDiscovery.roots` as the walk began. */
  readonly worktrees: ReadonlySet<string>;
  readonly versions: ReadonlyMap<string, number>;
  /** `ToolCallWindows.ends` as the walk began. */
  readonly callsEnded: number;
}

/** A walk of the other worktrees as this session takes it: what changed since its snapshots, read off the lane. */
interface WorktreeTake {
  readonly walk: WalkStart;
  readonly scan: SourceScan;
  /** The snapshot of each worktree the walk read, by its path relative to the session folder. */
  readonly snapshots: ReadonlyMap<string, WorktreeSnapshot>;
  /**
   * Sources the take leaves to the next walk, each with what the session saw of it before: one the session could not
   * read now, and one it never saw and found gone.
   */
  readonly views: ReadonlyMap<string, ObservedSource | undefined>;
}

/** The paths of `next` whose observation is not `base`'s: new, changed or gone. */
function changedPaths(base: WorktreeSnapshot | undefined, next: WorktreeSnapshot): string[] {
  if (base === next) return [];
  const changed: string[] = [];
  for (const [path, source] of next.sources) if (base?.sources.get(path) !== source) changed.push(path);
  if (base !== undefined) for (const path of base.sources.keys()) if (!next.sources.has(path)) changed.push(path);
  return changed;
}

/** `path` in the worktree at `worktree` (both session-relative paths but `path`, relative to the worktree; `""` for it). */
function inWorktree(worktree: string, path: string): string {
  return path === "" ? worktree : `${worktree}/${path}`;
}

/** What a walk found of `path` (session-relative, in `worktree`), named as the session names it, with any text the walk kept. */
function takenSource(worktree: string, findings: WorktreeFindings, path: string): Source | undefined {
  const key = path.slice(worktree.length + 1);
  const found = findings.snapshot.sources.get(key);
  if (found === undefined) return undefined;
  const text = findings.texts.get(key);
  return text === undefined ? { ...found, path } : { ...found, path, text };
}

/** Puts into `scan` what a walk of `worktree` could not check where the session takes it, named as the session names it. */
function takeGaps(worktree: string, findings: WorktreeFindings, scan: SourceScan, takes: (path: string) => boolean): void {
  for (const failed of findings.failedRoots) if (takes(failed)) scan.failedRoots.add(inWorktree(worktree, failed));
  scan.errors.push(...takenErrors(worktree, findings.errors, takes));
  for (const [path, unread] of findings.unread) if (takes(path)) scan.unread.set(inWorktree(worktree, path), unread);
}
/**
 * The errors of a walk of `worktree` the session takes, named as the session names them, each with all it holds: an EPERM
 * the shared walk had no room to look at again stays unrecorded (`ScanError.unrecorded`) until a later read looks at it, as
 * in the session folder.
 */
function takenErrors(worktree: string, errors: readonly ScanError[], takes: (path: string) => boolean): ScanError[] {
  return errors.filter(error => takes(error.path)).map(error => ({ ...error, path: inWorktree(worktree, error.path) }));
}

/**
 * Puts into `taken` the heads and unmerged entries a walk read of the worktree at `root` and of the checkouts inside it,
 * and the unmerged entries of the checkouts it passed (`SourceScan.unmerged`), by physical root.
 */
function takeCheckouts(
  root: string,
  findings: WorktreeFindings,
  taken: { readonly heads: Map<string, string>; readonly unmerged: Map<string, ReadonlyMap<string, string>>; readonly passed: SourceScan["unmerged"] },
): void {
  if (findings.head !== undefined) taken.heads.set(root, findings.head);
  if (findings.unmerged !== undefined) taken.unmerged.set(root, findings.unmerged);
  for (const [checkout, head] of findings.inner.heads) taken.heads.set(checkout, head);
  for (const [checkout, entries] of findings.inner.unmerged) taken.unmerged.set(checkout, entries);
  for (const [checkout, entries] of findings.inner.passed) taken.passed.set(checkout, entries);
}

/**
 * Puts what the session reads now of a changed source the walk kept no text of (`#reread`) into `scan`, and into `kept`
 * a source it leaves to the next walk: one it cannot read now, with what it saw before, and one it never saw and finds gone.
 */
function takeReread(path: string, read: Source | "gone" | "unread", walk: WalkStart, scan: SourceScan, kept: Map<string, ObservedSource | undefined>): void {
  if (typeof read !== "string") scan.sources.set(path, read);
  else scan.sources.delete(path);
  if (read === "unread") kept.set(path, walk.previous.get(path));
  else if (read === "gone" && !walk.previous.has(path)) kept.set(path, undefined);
}

/** `read`, a file changed again since a walk found it as `found`, stamped as the walk found it: what was written since was written later still. */
function stampedAsFound(read: Source, found: Source): Source {
  const { modifiedAt, observedAt } = found;
  return { ...read, ...(modifiedAt === undefined ? {} : { modifiedAt }), ...(observedAt === undefined ? {} : { observedAt }) };
}

/**
 * Which paths of the worktree at `root` (relative to it) a session in `folder` takes from the shared walk: every one, but
 * in a worktree around the folder, none in the folder on the way to it. The session names those files from its folder
 * (`../x.py`), a path under no worktree's, and no walk records a change there; the snapshot names them from the worktree
 * (`../../../a/x.py`), which would be a second name for a file the session's tool calls name the first way.
 */
function takenFrom(root: string, folder: string): (path: string) => boolean {
  const inside = relative(root, folder);
  if (inside === "" || inside === ".." || inside.startsWith(`..${sep}`) || isAbsolute(inside)) return () => true;
  const aside = inside.split(sep)[0]!;
  return path => path !== aside && !path.startsWith(`${aside}/`);
}

/** The innermost of `roots` (session-relative) that holds `path`; each folder above it is looked up once. */
function rootHolding(roots: ReadonlySet<string>, path: string): string | undefined {
  for (let slash = path.lastIndexOf("/"); slash > 0; slash = path.lastIndexOf("/", slash - 1)) {
    const root = path.slice(0, slash);
    if (roots.has(root)) return root;
  }
  return undefined;
}

interface SourceDiscovery {
  /**
   * Filesystem-clock reading taken before the walk; files stamped at or after it may hold this session's shell writes.
   * A session opened again after its runtime suspended it starts where its walks stood then (`SessionWalkResume.since`).
   */
  readonly startedAt: number;
  /** Filesystem-clock reading taken as the session folder's last finished walk began; `startedAt` until one finishes. */
  walkedFrom: number;
  readonly root: string;
  cancelled: boolean;
  /** Resolves once every requested pass has run; undefined while no walk is in flight. */
  idle: Promise<void> | undefined;
  baselineComplete: boolean;
  requested: DiscoveryCheckpoint | undefined;
  readonly versions: Map<string, number>;
  /** Subtrees whose first successful observation must still be a baseline. */
  baselineRoots: Set<string>;
  /** The refusals the session folder's walks withdrew (`SourceWalk.withdrawn`): the session's own, gone with it. */
  readonly withdrawn: Set<string>;
  /**
   * The texts the next boundary puts once in `checkpoint`'s answer (`newlyLate`), which the daemon does not read, of
   * each checkout whose Git did not list its changed files in time: each text once, so a later late walk of a checkout
   * does not hide that its listing asked again failed or ran late.
   */
  readonly late: Set<string>;
  /**
   * The checkouts a listing asked again off the walks' path (`#listLateAgain`) is waiting for, running or queued, each
   * with the sources it may keep the first-sight text of, by path.
   */
  readonly askingAgain: Map<string, Map<string, Source>>;
  /** Each listing asked again that has not finished, with its reads and keeps of first-sight texts (`settleListingsAgain`). */
  readonly listingsAgain: Set<Promise<void>>;
  /** The slots listings asked again take to ask Git (`LISTINGS_AGAIN_AT_ONCE`), and those waiting for one. */
  readonly listingSlots: Slots;
  readonly worktrees: WorktreeDiscovery;
  /** What the session folder's last walk could not check, or why that walk failed whole. */
  gaps: readonly WalkGap[];
  /** A boundary answered while the current walk was running; closing before it finishes says so again. */
  pendingReported: boolean;
  /**
   * The unlisted files the last walk found too large to read, by path, with their stamps. Each is reported as not
   * checked at one boundary only while its stamp holds (`reported`), not at every turn: it is no listed source.
   */
  unread: ReadonlyMap<string, { readonly stamp: string; readonly readError: string }>;
  /** The stamp each unlisted file in `unread` was last reported with this session. */
  readonly reported: Map<string, string>;
  /** Each checkout's `HEAD` when the last walk that finished began, by its root: what the next walk's changes followed. */
  readonly heads: Map<string, string>;
  /**
   * Every Git read the session's walks and ignore rules start, which `close` stops and waits out: a walk is never waited
   * for, and on Windows a Git it left running would hold the checkout after the session ended.
   */
  readonly git: GitReads;
  /** Each checkout's unmerged entries (`readUnmerged`) when that walk began: a conflict they hold is not the next walk's. */
  readonly unmerged: Map<string, ReadonlyMap<string, string>>;
  /** The checkouts whose `.git` the session folder's last walk passed (`SourceScan.checkouts`): those inside it, and its own. */
  passedCheckouts: ReadonlySet<string>;
  /**
   * Reads a checkout's unmerged entries now. Every pass asks, so a Git process runs only if its index file changed; each
   * read goes through `git` (`unmergedNow`).
   */
  readonly readUnmerged: ReturnType<typeof gitUnmergedReader>;
  /** When the session's tool calls ran: a change the session folder's walk finds is its own only when one ran then. */
  readonly toolCalls: ToolCallWindows;
  /**
   * Each file the session folder's last walk could not read, by path, with the calls that may have written it unseen
   * (`holdMissedFiles`): any call from the last walk that listed every folder before the walks began to miss it, up to
   * the last walk's start. Another program may let the file go between two walks that miss it, so each call in that
   * time may have written it.
   */
  missedFiles: ReadonlyMap<string, EndedCalls>;
  /** `ToolCallWindows.ends` as the last session folder walk that listed every folder began. */
  listedEnds: number;
}

/**
 * Samples the clock file timestamps are stamped with. `Date.now()` is not that clock: on Linux file stamps
 * advance per scheduler tick and lag it, so a write made right after session start carries an earlier mtime,
 * while on macOS stamps carry sub-millisecond precision that `Date.now()` truncates away.
 */
export async function filesystemClock(): Promise<number> {
  try {
    const probe = await mkdtemp(join(tmpdir(), "rerail-clock-"));
    try {
      const info = await lstat(probe);
      return Math.min(info.mtimeMs, info.ctimeMs);
    } finally {
      // Retried while Windows holds the folder, then left in the temp directory: a failed removal must not throw the
      // sample away for `Date.now()`, the clock this function exists to avoid.
      await removeScratch(probe);
    }
  } catch {
    return Date.now();
  }
}

/**
 * One entry a walk could not check, so shell writes there went unchecked: its path as the session folder names it (`.` for
 * the folder itself), absent when the walk failed whole, and why.
 */
interface WalkGap {
  readonly path?: string;
  readonly reason: string;
  /**
   * Said by the checkpoint but not yet a place for the record (`#tellGaps`): an EPERM the walk had no room to look at
   * again (past the first `MAX_LOOKED_AGAIN` in path order, those an earlier walk withdrew last: `judgeRefusals`), which a
   * later walk looks at, or a whole walk a path being removed made fail.
   */
  readonly unrecorded?: true;
}
/**
 * A path that refused the walk with EPERM, looked at once more by `lookedAgain` before its failure is a gap: Windows refuses
 * a path while its delete is pending, and always when the user is denied it.
 */
interface Refusal {
  /** The failure's path as the walk's errors name it (`ScanError.path`). */
  readonly path: string;
  /** The path the refusal named: the one examined, or a child whose real path its folder's listing asked for. */
  readonly refused: string;
  /** `refused` is the path examined, so a look again that finds it gone finds the failure's path gone. */
  readonly examined: boolean;
  /** What the walk's lstat found at `refused` before the refusal, when it found it there. */
  readonly before?: Stats;
}
/** What a look again at a refused path finds: still there (a denial), gone, going (its delete pending), or another entry than the walk found. */
type LookedAgain = "there" | "gone" | "going" | "changed";
/** Something a walk could not read, at its path (`""` for the folder the walk started at). */
export interface ScanError {
  readonly path: string;
  readonly readError: string;
  /** Its EPERM was past the first `MAX_LOOKED_AGAIN` the walk looked at again (`settleRefusals`): `WalkGap.unrecorded`. */
  readonly unrecorded?: true;
}
export interface SourceScan {
  readonly sources: Map<string, Source>;
  /** The folders the walk could not list: a first sight under one next time is a baseline (`firstSightIsBaseline`). */
  readonly failedRoots: Set<string>;
  /**
   * The walk gave up on a path it met (`gaveUpOn`): a folder it could not list, or a path gone when it examined it or
   * looked at it again, which may have moved where the walk did not look. Whether a place is recorded for it or not, the
   * walk did not see every file: it lets no ended call go (`holdMissedFiles`), and another worktree's read is tried again.
   */
  gaveUp?: true;
  readonly errors: ScanError[];
  /** Unlisted files too large to read, by path (`SourceDiscovery.unread`); left out of `errors`. */
  readonly unread: Map<string, { readonly stamp: string; readonly readError: string }>;
  /**
   * The text of each source the walk saw first that its checkout's Git may not hold, read again after the scan, by path
   * (`#readFirstSeenTexts`): the preimage of the session's next shell write to it, which no blob could give.
   */
  readonly firstSeen: Map<string, { readonly text: string; readonly digest: string }>;
  /** The checkout that owns each folder the walk passed a `.git` entry in: a checkout inside the session folder, or inside another worktree, is found here. */
  readonly checkouts: Set<string>;
  /** The unmerged entries of each of `checkouts` that no walk had read, read after the scan (`#readPassedCheckouts`). */
  readonly unmerged: Map<string, ReadonlyMap<string, string>>;
  /** For a read of another worktree (`#scanWorktree`): what it walked into and what it left out, which a notice is weighed against. */
  readonly seen?: { readonly folders: Set<string>; readonly leftOut: Set<string> };
  /**
   * The checkouts whose Git did not list their changed files in time (`#firstSeenModifiedOrUntracked`,
   * `#firstSeenInWorktrees`), each with the sources the walk saw first there, by path.
   */
  readonly lateListings: Map<string, ReadonlyMap<string, Source>>;
}
function emptyScan(): SourceScan {
  return {
    sources: new Map(), failedRoots: new Set(), errors: [], unread: new Map(), firstSeen: new Map(), checkouts: new Set(), unmerged: new Map(),
    lateListings: new Map(),
  };
}
/** What first-sight texts are taken against (`takeFirstSeen`): the sources as a walk saw them, and the texts taken so far. */
interface FirstSightTake {
  readonly sources: ReadonlyMap<string, Source>;
  readonly firstSeen: SourceScan["firstSeen"];
}
/**
 * Where a text `CodeRuleBridge` keeps came from: an edit the session made, text Git wrote that neither the index nor
 * `HEAD` holds, another writer's change in the session folder, or a walk's first sight of a source Git may not hold,
 * which is no change of the session's.
 */
type TextOrigin = "edit" | "git" | "other-writer" | "first-sight";
/** Whether the evaluator holds a text of that origin as the session's: what it last audited of the source. */
const HELD_BY_EVALUATOR: Readonly<Record<TextOrigin, boolean>> = { edit: true, git: true, "other-writer": false, "first-sight": false };
/**
 * What a traversal reads against: the folder its paths are relative to, when the session opened (a file stamped since
 * has its text kept), and whether it was called off.
 */
type WalkScope = Pick<SourceDiscovery, "root" | "startedAt" | "cancelled">;
/** One traversal of the workspace: what it compares against, when it started, and what it has found so far. */
interface SourceWalk {
  readonly discovery: WalkScope;
  /** The checkout that owns the `.git` entry at a path the walk names (`SourceScan.checkouts`); undefined for none. */
  readonly checkoutOf: (path: string) => string | undefined;
  readonly observedAt: number;
  readonly previousSources: ReadonlyMap<string, ObservedSource>;
  readonly captureChanges: boolean;
  /**
   * Whether a shell command of the session runs, or ended since the walk began (`ToolCallWindows.commandSince`); the
   * other worktrees' walks ask nothing.
   */
  readonly commandRuns?: () => boolean;
  /**
   * Where a source the walk has no observation of is a baseline, as the walk began: the subtrees the last walk could not
   * read (`firstSightIsBaseline`). What a walk keeps no text of under them is what `#discoveredEdit` then takes as one.
   */
  readonly baselineRoots: ReadonlySet<string>;
  /** The buffers the walk reads files into: one per read it runs at once, not one per file. */
  readonly buffers: WalkReadBuffers;
  readonly scan: SourceScan;
  readonly stack: string[];
  /** Where the walk starts: a folder reached again from another is not walked twice. */
  readonly roots: ReadonlySet<string>;
  /** What the walk asks of each file and folder it finds: the auditor's `walkView` for this walk, or the auditor. */
  readonly view: WalkView;
  /**
   * Whether the record holds the EPERM gap of a path (`#holdsRefusal`), or for another worktree's walk the record of every
   * session it serves (`WorktreeScan.held`): a refusal there is a gap, and is not looked at again.
   */
  readonly held: (path: string) => boolean;
  /**
   * The refusals the walk met (`noteRefusal`): the first `MAX_LOOKED_AGAIN` in path order, those an earlier walk withdrew
   * last (`judgeRefusals`), are looked at once its other paths are done.
   */
  refusals?: Refusal[];
  /**
   * The refusals earlier walks of this folder looked at again and found no denial, that each walk since met and had no
   * room to look at: the session folder's walks' (`SourceDiscovery.withdrawn`), or for another worktree the reads of it
   * through any session (`WorktreeScan.withdrawn`). The walk looks at them after the others (`judgeRefusals`) and keeps
   * this up to date.
   */
  readonly withdrawn: Set<string>;
}
/** What a walk of `SourceDiscovery` compares its scan with, all read before it started. */
interface WalkStart {
  /** The session folder's walk, else the walk of the other worktrees. */
  readonly inSession: boolean;
  /** The session folder's first walk: what it finds is a baseline. */
  readonly baseline: boolean;
  readonly checkpoint: DiscoveryCheckpoint;
  /** `WorktreeDiscovery.roots` as the walk started. */
  readonly worktrees: ReadonlySet<string>;
  readonly previous: ReadonlyMap<string, ObservedSource>;
  readonly versions: ReadonlyMap<string, number>;
  readonly heads: ReadonlyMap<string, string>;
  readonly unmerged: ReadonlyMap<string, ReadonlyMap<string, string>>;
  /**
   * `ToolCallWindows.ends` as the walk started: a command that ended since may have been writing what the walk read, and
   * only the calls that ended before are those the walk answers for (`ToolCallWindows.answered`).
   */
  readonly callsEnded: number;
  /** Filesystem-clock reading taken as the walk started. */
  readonly observedAt: number;
}
/** A worktree whose walk a session takes: its root (physical), its path as the session names it, and what the walk found. */
interface TakenWorktree {
  readonly root: string;
  readonly worktree: string;
  readonly findings: WorktreeFindings;
  readonly scan: SourceScan;
}
/**
 * A walk's changes as Git's verdicts sort them (`#separateOwnWrites`): those Git did not write, for the session to
 * claim; those another writer or Git made that are still audited; and the digest each uncaptured change had before.
 */
interface SortedWrites {
  readonly unwritten: CodeRuleEdit[];
  readonly notOwn: CodeRuleEdit[];
  readonly uncaptured: Map<string, string>;
}
/**
 * What the Git of `checkout` lists as modified or untracked: as the walk listed it, else listed now; undefined when it
 * ran past its time limit.
 */
async function listedChanges(discovery: SourceDiscovery, findings: WorktreeFindings, checkout: string): Promise<readonly string[] | undefined> {
  return findings.listed.has(checkout)
    ? findings.listed.get(checkout)
    : await discovery.git.run(signal => gitModifiedOrUntrackedPaths(checkout, { signal }));
}
/**
 * Each walk answers for what it covers: a path a tool wrote elsewhere is never observed missing here. A worktree that
 * came or went while a walk ran is not the session walk's, and one gone is no longer the worktree walk's either.
 */
function walkAnswersFor(discovery: SourceDiscovery, walk: WalkStart, path: string): boolean {
  const walked = underBaselineRoot(walk.worktrees, path);
  const current = underBaselineRoot(discovery.worktrees.roots, path);
  if (!walk.inSession) return walked && current;
  return !walked && !current && path !== ".." && !path.startsWith("../");
}
/**
 * The digest a changed source was observed with before, when the walk found it changed and could not read what it was;
 * none for a source the walk takes as a baseline (`baselinePath`), which has no earlier observation to trust.
 */
function uncapturedDigest(edit: CodeRuleEdit, before: ObservedSource | undefined, baselinePath: boolean): string | undefined {
  if (baselinePath || edit.previousReadError === undefined || edit.generatedText === undefined) return undefined;
  return before?.digest;
}
/** A first observation made after the session started may hold its shell writes, and is not a trustworthy preimage. */
function changedSinceStart(discovery: Pick<SourceDiscovery, "startedAt">, after: Source | undefined): boolean {
  return after?.modifiedAt !== undefined && after.modifiedAt >= discovery.startedAt;
}
/** A linked worktree's `.git` is a file, and no more a source than the folder would be. */
function neverWalked(entry: Dirent): boolean {
  return entry.isSymbolicLink() || entry.name === ".git" || (entry.isDirectory() && IGNORED_DIRECTORIES.has(entry.name));
}
/**
 * A stamp only proves a file unchanged when it is older than the observation it is compared with:
 * a same-size rewrite within the same filesystem-clock tick leaves the stamp identical (git's racy-index rule).
 */
function staleStamp(previous: ObservedSource | undefined, stamp: string, modifiedAt: number): boolean {
  return previous?.stamp === stamp && previous.observedAt !== undefined && modifiedAt < previous.observedAt;
}

function strings(value: unknown, output: string[]): void {
  if (typeof value === "string") output.push(value);
  else for (const item of nestedValues(value)) strings(item, output);
}
const PATH_LIKE_KEY = /^(?:path|paths|file|filename|file_path|filePath|patch|input|command)$/u;
/** What `strings` looks in: an array's items, or an object's values under a path-like key. */
function nestedValues(value: unknown): unknown[] {
  if (Array.isArray(value)) return value;
  if (value === null || typeof value !== "object") return [];
  return Object.entries(value).filter(([key]) => PATH_LIKE_KEY.test(key)).map(([, item]) => item);
}
function patchPaths(text: string): string[] {
  return [
    ...[...text.matchAll(/^\[([^#\r\n]+)#[0-9A-F]{4}\]$/gmu)].map(match => match[1]!),
    ...[...text.matchAll(/^\*\*\* (?:Add File|Update File|Delete File|Move to): ([^\r\n]+)$/gmu)].map(match => match[1]!),
  ];
}
function sameSource(left: ObservedSource | undefined, right: Source | undefined): boolean {
  return left?.digest === right?.digest && left?.readError === right?.readError;
}
function emptyResponse(revision: number): CodeRuleResponse {
  return { revision, disposition: "continue", violations: [], failures: [] };
}
/** The session folder's walk is running after a boundary stopped waiting for it. */
function sessionWalkAbandoned(discovery: SourceDiscovery): boolean {
  return discovery.idle !== undefined && discovery.pendingReported;
}
/** A worktree walk a boundary asked for has yet to be taken, and there are worktrees for it to check. */
function worktreeWalkAbandoned(discovery: SourceDiscovery): boolean {
  return discovery.worktrees.watch?.pending() === true && discovery.worktrees.roots.size > 0;
}
/** Each of `unread` this session has not yet reported at its stamp, now marked reported. */
function newlyUnread(discovery: SourceDiscovery, unread: SourceDiscovery["unread"]): WalkGap[] {
  const gaps: WalkGap[] = [];
  for (const [path, { stamp, readError }] of unread) {
    if (discovery.reported.get(path) === stamp) continue;
    discovery.reported.set(path, stamp);
    gaps.push({ path, reason: readError });
  }
  return gaps;
}
/** Each of `unread` as a gap. */
function unreadGaps(unread: SourceDiscovery["unread"]): WalkGap[] {
  return [...unread].map(([path, { readError }]) => ({ path, reason: readError }));
}
/** The texts of `SourceDiscovery.late` for the boundary's `checkpoint` answer, each put there once. */
function newlyLate(discovery: SourceDiscovery): string[] {
  const errors = [...discovery.late];
  discovery.late.clear();
  return errors;
}
/**
 * What a boundary's `checkpoint` answer holds of a checkout whose Git did not list its changed files in time, first and
 * after the listing asked again; and after a listing asked again that failed. The daemon does not read that answer.
 */
const LATE_LISTING = "Git did not list the checkout's changed files in time";
const KEPT_NO_FIRST_SIGHT = "a shell edit to a file changed there before the session is checked against Git's text of it, or whole";
const LATE_AGAIN = `${LATE_LISTING} again: ${KEPT_NO_FIRST_SIGHT}`;
const FAILED_AGAIN = `Git could not list the checkout's changed files when asked again: ${KEPT_NO_FIRST_SIGHT}`;
function lateListing(discovery: SourceDiscovery, checkout: string, message: string): string {
  return `${sessionPath(discovery.root, checkout) || "."}: ${message}`;
}
/**
 * A timer that calls `resolve` after `ms`, or none when `ms` is not finite: such a wait (`discoveryWaitMs: Infinity`) ends
 * only with what it waits for, where Node would fire a timer past 2^31 - 1 ms after 1 ms.
 */
function waitTimer(ms: number, resolve: () => void): NodeJS.Timeout | undefined {
  return Number.isFinite(ms) ? setTimeout(resolve, ms) : undefined;
}
/** Resolves once `answered` has settled or `ms` have passed, whichever comes first. */
function settledOrAfter(answered: Promise<unknown>, ms: number): Promise<void> {
  return new Promise(resolve => {
    const timer = setTimeout(resolve, ms);
    const settle = (): void => {
      clearTimeout(timer);
      resolve();
    };
    answered.then(settle, settle);
  });
}
/** Slots for tasks that run at once: how many are free, and the tasks waiting for one, in the order they asked. */
interface Slots {
  free: number;
  readonly waiting: Array<() => void>;
}
/** Resolves once a slot of `slots` is the caller's: at once while one is free, else when one is handed on to it. */
function slotTaken(slots: Slots): Promise<void> {
  if (slots.free === 0) return new Promise(resolve => { slots.waiting.push(resolve); });
  slots.free -= 1;
  return Promise.resolve();
}
/** Hands the caller's slot in `slots` on to the task that has waited longest, or frees it. */
function slotHandedOn(slots: Slots): void {
  const next = slots.waiting.shift();
  if (next === undefined) slots.free += 1;
  else next();
}
/** The sources `scan` found at `paths`, by path. */
function sourcesAt(scan: SourceScan, paths: readonly string[]): Map<string, Source> {
  const sources = new Map<string, Source>();
  for (const path of paths) {
    const source = scan.sources.get(path);
    if (source !== undefined) sources.set(path, source);
  }
  return sources;
}
/**
 * `gaps` in path order: walks list their gaps in the order their reads finish, which differs from walk to walk. Gaps of one
 * path keep the order they were found in.
 */
function byPath(gaps: WalkGap[]): WalkGap[] {
  return gaps.sort((left, right) => compareDeterministicStrings(left.path ?? "", right.path ?? ""));
}
/**
 * Every place the last walks of the session folder and of the other worktrees could not check, each unlisted file too
 * large to read among them, in path order: what the record takes (`#tellGaps`), at every checkpoint until it holds it.
 */
function walkGaps(discovery: SourceDiscovery): WalkGap[] {
  return byPath([...discovery.gaps, ...discovery.worktrees.gaps, ...unreadGaps(discovery.unread), ...unreadGaps(discovery.worktrees.unread)]);
}
/**
 * What a checkpoint says the walks could not check (`discoveryGap`): `walkGaps`, but each unlisted file too large to read
 * only at the first checkpoint that meets it at its stamp (`newlyUnread`).
 */
function saidGaps(discovery: SourceDiscovery): WalkGap[] {
  const gaps = [...discovery.gaps, ...discovery.worktrees.gaps];
  for (const unread of [discovery.unread, discovery.worktrees.unread]) gaps.push(...newlyUnread(discovery, unread));
  return byPath(gaps);
}
/**
 * A checkpoint's `source-incomplete` text: `gaps` (`saidGaps`) joined, then each text of `SourceDiscovery.late`
 * (`newlyLate`), which is said and not recorded; else whether the session folder's walk is still to finish.
 */
function discoveryGap(discovery: SourceDiscovery, gaps: readonly WalkGap[]): string | undefined {
  const said = [...gaps.map(gap => (gap.path === undefined ? gap.reason : `${gap.path}: ${gap.reason}`)), ...newlyLate(discovery)];
  if (said.length > 0) return said.join("; ");
  if (!discovery.baselineComplete) return "Initial source discovery is still pending";
  return discovery.idle === undefined ? undefined : "Source reconciliation is still pending";
}
/**
 * The most gaps one bridge tells `onDiscoveryGaps` of. Past it, one more gap says the walks could not check more than that
 * (`UNTOLD_GAPS`): a tree of unreadable files records a bounded number of rows.
 */
export const MAX_TOLD_GAPS = 1_000;
/**
 * The most gaps one checkpoint tells, in path order; later checkpoints tell the next ones. Writing 1,000 rows held a
 * turn end 90 to 160 ms past main's on Windows; 100 hold it a few.
 */
export const MAX_TOLD_AT_ONCE = 100;
const UNTOLD_GAPS = `The walks could not check more than ${MAX_TOLD_GAPS} places; the rest are not recorded`;
/**
 * What tells one gap from another (`DiscoveryGap.key`): its path (`""` for a walk that failed whole) and its reason's
 * kind, the error code that opens a filesystem error's text (`EACCES: permission denied, opendir '…'`), else the reason
 * whole, as for a file too large to read.
 */
function gapKey(path: string | undefined, kind: string): string {
  return JSON.stringify([path ?? "", kind]);
}
function reasonKind(reason: string): string {
  return /^([A-Z][A-Z0-9_]*): /u.exec(reason)?.[1] ?? reason;
}
/** The key of the gap past `MAX_TOLD_GAPS` (`UNTOLD_GAPS`): once the record holds it, it holds all it will. */
const UNTOLD_KEY = gapKey(undefined, UNTOLD_GAPS);
/** `gap` as the bridge tells it (`gapKey`). */
function toldGap(boundary: DiscoveryGap["boundary"], { path, reason }: WalkGap): DiscoveryGap {
  return { boundary, ...(path === undefined ? {} : { path }), reason, key: gapKey(path, reasonKind(reason)) };
}
function untoldGaps(boundary: DiscoveryGap["boundary"]): DiscoveryGap {
  return { boundary, reason: UNTOLD_GAPS, key: UNTOLD_KEY, more: true };
}
function hasErrorCode(error: unknown, code: string): boolean {
  return error instanceof Error && "code" in error && error.code === code;
}
/**
 * A path removed while the walk examined it: ENOENT from its lstat, opendir, open or read, ENOTDIR from those where a
 * folder on its path, or the folder itself, was replaced by a file, or the real path of one being removed, which on
 * Windows can fail with EBADF.
 */
function vanished(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  const { code, syscall } = error as NodeJS.ErrnoException;
  return code === "ENOENT" || code === "ENOTDIR" || (code === "EBADF" && syscall === "realpath");
}
/**
 * A path a folder's listing named that was gone when the walk examined it: nothing is left there to check, so it is no
 * gap, as a read finds a file gone (`#failedSourceRead`). It may have moved where the walk did not look, so the walk gave
 * up on it (`gaveUpOn`). A path the walk starts at stays a gap when it cannot be read.
 */
function goneWhenExamined(walk: SourceWalk, absolute: string, error: unknown): boolean {
  return vanished(error) && !walk.roots.has(absolute);
}
/**
 * The most refusals one walk looks at again, as many as a checkpoint records (`MAX_TOLD_AT_ONCE`), the first in path order,
 * those an earlier walk withdrew last (`judgeRefusals`): the rest are said and left to later walks to look at, so a tree of
 * 10,000 denied files costs a walk at most that many looks more, and the record keeps the same places on every system.
 */
const MAX_LOOKED_AGAIN = MAX_TOLD_AT_ONCE;
/** The path an error names, when it names one. */
function refusedPath(error: unknown): string | undefined {
  const path: unknown = error instanceof Error && "path" in error ? error.path : undefined;
  return typeof path === "string" ? path : undefined;
}
/**
 * Keeps the walk's failure of `absolute` (`#recordObservationFailure`) to look at again once its other paths are done
 * (`judgeRefusals`) when it refused with EPERM, as Windows refuses a path whose delete is pending and one the user is
 * denied. Not a path the walk starts at that refused itself, which stays a gap, nor one held a gap (`SourceWalk.held`).
 */
function noteRefusal(walk: SourceWalk, absolute: string, error: unknown, before: Stats | undefined): void {
  if (!hasErrorCode(error, "EPERM")) return;
  const refused = refusedPath(error) ?? absolute;
  const path = sessionPath(walk.discovery.root, absolute);
  if (startRefused(walk, absolute, refused) || walk.held(path || ".")) return;
  (walk.refusals ??= []).push(refusalOf(path, absolute, refused, before));
}
/** A path the walk starts at refused it itself, not a child its listing asked about. */
function startRefused(walk: SourceWalk, absolute: string, refused: string): boolean {
  return refused === absolute && walk.roots.has(absolute);
}
/** What the walk's lstat found (`before`) names `absolute`: it tells whether `refused` is still that entry only when it is `absolute`. */
function refusalOf(path: string, absolute: string, refused: string, before: Stats | undefined): Refusal {
  const examined = refused === absolute;
  return { path, refused, examined, ...(examined && before !== undefined ? { before } : {}) };
}
/**
 * What a look at `refusal.refused` finds, reading nothing of it: gone, or going (its delete pending); there as the walk
 * found it, a denial; or another entry (a path removed and made again). Its attributes (`access`) tell a denial from a
 * delete: on Windows 11 they are refused with EPERM while a delete of the path is pending, one that is not POSIX or whose
 * deleting handle is still open, and given for a path the user is denied to anyone who may list its folder, as the walk
 * did to find it. Its lstat answers in both cases, and the error's code is EPERM in both.
 */
async function lookedAgain({ refused, before }: Refusal): Promise<LookedAgain> {
  const attributes = await access(refused).then(() => undefined, (error: unknown) => ({ error }));
  if (attributes !== undefined) return withoutAttributes(attributes.error);
  return before === undefined ? "there" : await sameEntryNow(refused, before);
}
/** What a look whose attributes were refused finds: gone, going, else there (as Linux refuses a path beneath a folder it may not search). */
function withoutAttributes(error: unknown): LookedAgain {
  if (vanished(error)) return "gone";
  return deletePending(error) ? "going" : "there";
}
/**
 * Windows refuses the attributes of a path whose delete is pending with EPERM (its STATUS_DELETE_PENDING reaches Node as
 * EPERM), where it gives those of a path the user is denied. Linux and macOS never answer an existence check (`access`
 * with F_OK) with EPERM. Windows also refuses with EPERM a path beneath a folder the user is denied, which a walk meets
 * only when the folder is denied between its listing and the look: that folder is the gap at the next walk.
 */
function deletePending(error: unknown): boolean {
  return hasErrorCode(error, "EPERM");
}
/** What an lstat of `path` finds against what the walk's lstat found (`before`). */
async function sameEntryNow(path: string, before: Stats): Promise<LookedAgain> {
  const now = await lstat(path).then(info => ({ info }), (error: unknown) => ({ error }));
  if ("error" in now) return vanished(now.error) ? "gone" : "there";
  return sameEntry(before, now.info) ? "there" : "changed";
}
function sameEntry(before: Stats, now: Stats): boolean {
  return before.dev === now.dev && before.ino === now.ino;
}
/**
 * Looks again at the first `MAX_LOOKED_AGAIN` refusals of `walk` (`noteRefusal`) in path order, those an earlier walk
 * withdrew last (`SourceWalk.withdrawn`), at once: no listing is made and no path walked again. One there is a denial,
 * and its failure a gap; any other is no gap (`withdrawn`), and a path examined that is gone is absent from the walk
 * (`absentAfterAll`). One whose delete is pending keeps what the walk held of it until a walk finds it gone, as a read
 * that fails does. The rest are said but not recorded at this walk (`Judged.unconfirmed`). So files whose deletes stay
 * pending fill the looks of one walk, not of every walk: a denial no earlier walk withdrew, past N of them, is recorded
 * by the walk numbered (N + 1) / `MAX_LOOKED_AGAIN` rounded up, the second for 101 of them. A walk that met no refusal
 * keeps none of those an earlier walk withdrew (`keepWithdrawn`).
 */
async function judgeRefusals(walk: SourceWalk): Promise<Judged> {
  const judged: Judged = { withdrawn: new Set(), unconfirmed: new Set() };
  const { refusals } = walk;
  if (walk.discovery.cancelled) return judged;
  if (refusals === undefined) {
    walk.withdrawn.clear();
    return judged;
  }
  const withdrawnBefore = ({ path }: Refusal): number => Number(walk.withdrawn.has(path));
  refusals.sort((left, right) => withdrawnBefore(left) - withdrawnBefore(right) || compareDeterministicStrings(left.path, right.path));
  for (const { path } of refusals.splice(MAX_LOOKED_AGAIN)) judged.unconfirmed.add(path);
  const looks = await Promise.all(refusals.map(lookedAgain));
  refusals.forEach((refusal, index) => notDenied(walk, judged, refusal, looks[index]));
  keepWithdrawn(walk.withdrawn, judged);
  return judged;
}
/** Keeps of `withdrawn` the refusals this walk met and had no room to look at, and adds those it withdrew. */
function keepWithdrawn(withdrawn: Set<string>, { withdrawn: now, unconfirmed }: Judged): void {
  for (const path of withdrawn) if (!unconfirmed.has(path)) withdrawn.delete(path);
  for (const path of now) withdrawn.add(path);
}
/** What `judgeRefusals` found of a walk's refusals, by the failure's path (`ScanError.path`). */
interface Judged {
  /** The failures a look again found no denial: no gap. */
  readonly withdrawn: Set<string>;
  /** The failures whose EPERM the walk had no room to look at again: said, not recorded (`ScanError.unrecorded`). */
  readonly unconfirmed: Set<string>;
}
function notDenied(walk: SourceWalk, judged: Judged, { path, examined }: Refusal, looked: LookedAgain | undefined): void {
  if (looked === "there") return;
  judged.withdrawn.add(path);
  if (looked === "gone" && examined) absentAfterAll(walk, path);
}
/**
 * Takes back the walk's failure of `path`, which was gone when looked at again: a file's read error, or a folder's
 * (`failFolder`) and what it set for each source seen in it before, which then reads as deleted, as when the walk's
 * lstat finds it gone. It costs what the failure did. As for a path gone when examined, the walk gave up on it.
 */
function absentAfterAll(walk: SourceWalk, path: string): void {
  const { scan } = walk;
  gaveUpOn(scan);
  scan.sources.delete(path);
  if (!scan.failedRoots.delete(path)) return;
  const gone = new Set([path]);
  for (const sourcePath of walk.previousSources.keys()) if (underBaselineRoot(gone, sourcePath)) scan.sources.delete(sourcePath);
}
/**
 * Settles the walk's errors by what its refusals' judgement found: none for a failure a look again found no denial
 * (`withdrawn`), and those whose EPERM it had no room to look at again marked unrecorded (`unconfirmed`).
 */
function settleRefusals(scan: SourceScan, { withdrawn, unconfirmed }: Judged): void {
  const kept = scan.errors.filter(error => !withdrawn.has(error.path));
  scan.errors.splice(0, scan.errors.length, ...kept.map((error): ScanError => (unconfirmed.has(error.path) ? { ...error, unrecorded: true } : error)));
}
/** A walk's errors as its places, in path order, each named as the session folder names it (`.` for the folder itself). */
function scanGaps(errors: readonly ScanError[]): WalkGap[] {
  return byPath(errors.map(({ path, readError, unrecorded }): WalkGap => ({ path: path || ".", reason: readError, ...(unrecorded ? { unrecorded } : {}) })));
}
/**
 * Whether Git ignores the folder `child` a listing named. One removed as the walk asks (its owner's lookup `vanished`) is
 * left to its own examination, which finds it gone (`goneWhenExamined`): it never fails the folder that listed it.
 */
function ignoredFolder(walk: SourceWalk, child: string): boolean {
  try {
    return walk.view.ignoresPath?.(sessionPath(walk.discovery.root, child), true) === true;
  } catch (error) {
    if (vanished(error)) return false;
    throw error;
  }
}
/** Whether `checkout` is still a Git checkout: its `.git` entry is there. A look that fails otherwise says nothing of it. */
async function stillACheckout(checkout: string): Promise<boolean> {
  try {
    await lstat(join(checkout, ".git"));
    return true;
  } catch (error) {
    return !hasErrorCode(error, "ENOENT") && !hasErrorCode(error, "ENOTDIR");
  }
}
function sha256(data: string | Buffer): string {
  return createHash("sha256").update(data).digest("hex");
}
/** Whether `path`, an absolute path, is `folder` or lies below it. */
function withinFolder(folder: string, path: string): boolean {
  return path === folder || path.startsWith(`${folder}${sep}`);
}
/** `edit` without the reason its preimage was unknown, once one is known. */
function withoutUncaptured({ previousReadError: _uncaptured, ...rest }: CodeRuleEdit): CodeRuleEdit {
  return rest;
}
/** Recording what a boundary found is best-effort: a record that throws is logged, and the boundary goes on. Whether it recorded. */
function reportBestEffort(report: () => void): boolean {
  try {
    report();
    return true;
  } catch (error) {
    process.stderr.write(`rerail: policy execution recording failed: ${errorMessage(error)}\n`);
    return false;
  }
}
/**
 * Whether the session folder's walk first sees `path` already written since the session started, on the filesystem's
 * clock: it may be the session's own write, so it is no baseline. A worktree's first walk never says so, since
 * `git worktree add` writes every file as the worktree appears; nor does a file that cannot be read.
 */
async function writtenThisSession(discovery: SourceDiscovery, walk: WalkStart, path: string): Promise<boolean> {
  if (!walk.inSession) return false;
  try {
    const info = await lstat(resolve(discovery.root, path));
    return Math.max(info.mtimeMs, info.ctimeMs) >= discovery.startedAt;
  } catch {
    return false;
  }
}
/** When the file at `absolute` was last written, on the filesystem's clock, as a walk reads its stamp; undefined when it cannot be read. */
async function writtenAt(absolute: string): Promise<number | undefined> {
  const info = await lstat(absolute).catch(() => undefined);
  return info === undefined ? undefined : Math.max(info.mtimeMs, info.ctimeMs);
}
/** The stamp of the nearest folder under `root` that holds, or held, `path`, on the filesystem's clock; undefined only when `root` itself cannot be read. */
async function folderStamp(root: string, path: string): Promise<number | undefined> {
  for (let folder = dirname(path); ; folder = dirname(folder)) {
    try {
      const info = await lstat(resolve(root, folder));
      return Math.max(info.mtimeMs, info.ctimeMs);
    } catch {
      if (folder === dirname(folder)) return undefined;
    }
  }
}
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
/**
 * Whether the other worktrees' walk finds the files of the checkout at `root`: it is one of them, or lies inside one (a
 * submodule there, or a repository of its own). Told as `walkAnswersFor` tells a file's walk, by its path from the
 * session folder: a worktree around the session folder does not take the session's checkout, nor one inside its folder.
 */
function inWorktrees(discovery: SourceDiscovery, root: string): boolean {
  const path = sessionPath(discovery.root, root);
  return discovery.worktrees.roots.has(path) || underBaselineRoot(discovery.worktrees.roots, path);
}
/**
 * Whether `path` lies below one of `roots` (`""` holds every path). Each folder above `path` is looked up in `roots`, so
 * a walk that asks this of every file costs the depth of the file, not the number of the repository's worktrees.
 */
function underBaselineRoot(roots: ReadonlySet<string>, path: string): boolean {
  if (roots.size === 0) return false;
  if (roots.has("")) return true;
  for (let slash = path.indexOf("/"); slash >= 0; slash = path.indexOf("/", slash + 1)) {
    if (roots.has(path.slice(0, slash))) return true;
  }
  return false;
}
/**
 * Whether the session folder's own walk covers `path`: inside the folder, and in none of the other `worktrees`. A path
 * that leads out of the folder is told apart by its first segment, before any worktree is looked up.
 */
function inSessionFolder(worktrees: ReadonlySet<string>, path: string): boolean {
  return path !== ".." && !path.startsWith("../") && !underBaselineRoot(worktrees, path);
}
/** `absolute` relative to the walk's root (the session folder), `/`-separated, as the auditor names paths. */
function sessionPath(root: string, absolute: string): string {
  return relative(root, absolute).split(sep).join("/");
}
/** `configuredPath` (as a tool names it) relative to the session folder `root`, `/`-separated; undefined for the folder itself or another drive. */
function sessionRelativePath(root: string, configuredPath: string): string | undefined {
  const path = sessionPath(root, isAbsolute(configuredPath) ? configuredPath : resolve(root, configuredPath));
  return path === "" || isAbsolute(path) ? undefined : path;
}
/** Why an edit of a source changed while the session's first walk was still to take it is read over its whole file. */
const CHANGED_IN_INITIAL_DISCOVERY = "Source changed during initial discovery without a captured preimage";
function uncapturedPreimage(baselinePath: boolean, before: ObservedSource | undefined): Pick<CodeRuleEdit, "previousReadError"> {
  if (baselinePath) return { previousReadError: CHANGED_IN_INITIAL_DISCOVERY };
  if (before === undefined) return {};
  return { previousReadError: "Source changed outside a tool with a captured preimage" };
}
function uncapturedEdit(
  path: string,
  after: Source,
  preimage: Pick<CodeRuleEdit, "previousReadError">,
): CodeRuleEdit | undefined {
  if (after.readError !== undefined) return { path, language: after.language, ...preimage, readError: after.readError };
  if (after.text !== undefined) return { path, language: after.language, ...preimage, generatedText: after.text };
  return undefined;
}
/** Puts in `scan.firstSeen` each of `sources` still holding the digest the scan observed; the bytes it took. */
function takeFirstSeen(scan: FirstSightTake, sources: ReadonlyArray<Source | undefined>): number {
  let bytes = 0;
  for (const source of sources) {
    const digest = source === undefined ? undefined : scan.sources.get(source.path)?.digest;
    if (source?.text === undefined || digest === undefined || source.digest !== digest) continue;
    scan.firstSeen.set(source.path, { text: source.text, digest });
    bytes += Buffer.byteLength(source.text);
  }
  return bytes;
}

/**
 * The digests `uncaptured` holds for `edits`. A walk's map also holds those of the changes it took for Git's, or dropped
 * with a worktree being removed (`#separateOwnWrites`): no edit needs their preimages, so no Git is read for them.
 */
function batchDigests(edits: readonly CodeRuleEdit[], uncaptured: ReadonlyMap<string, string>): Map<string, string> {
  const digests = new Map<string, string>();
  for (const { path } of edits) {
    const digest = uncaptured.get(path);
    if (digest !== undefined) digests.set(path, digest);
  }
  return digests;
}

/**
 * Puts in `recovered` each of `paths` (owned by `checkout`) whose Git index or `HEAD` blob has the digest `uncaptured`
 * names for it.
 */
async function recoverFromGit(
  discovery: SourceDiscovery,
  checkout: string,
  paths: readonly string[],
  uncaptured: ReadonlyMap<string, string>,
  recovered: Map<string, string>,
): Promise<void> {
  const named = paths.map(path => ({ path, inCheckout: sessionPath(checkout, resolve(discovery.root, path)) }));
  const revisions = named.flatMap(({ inCheckout }) => [`:./${inCheckout}`, `HEAD:./${inCheckout}`]);
  const [blobs, crlf] = await Promise.all([
    // A blob larger than a source could never match its digest: it is skipped, never read into memory.
    discovery.git.run(signal => readGitBlobs(checkout, revisions, { maxBlobBytes: MAX_SOURCE_BYTES, signal })),
    discovery.git.run(signal => readGitCrlfPaths(checkout, named.map(({ inCheckout }) => inCheckout), { signal })),
  ]);
  for (const { path, inCheckout } of named) {
    const digest = uncaptured.get(path);
    for (const blob of [blobs.get(`:./${inCheckout}`), blobs.get(`HEAD:./${inCheckout}`)]) {
      const form = heldAs(blob, digest, crlf.get(inCheckout));
      if (blob === undefined || form === undefined) continue;
      // In the line breaks the file held it in, so the diff against the working tree lines up line by line.
      recovered.set(path, textAs(blob, form));
      break;
    }
  }
}

/**
 * The paths an edit tool's input names: each single-line string under a path-like key, or the files a patch in one
 * names. Any file may be one; the auditor decides which it reads (`acceptsPath`).
 */
export function generatedEditPaths(toolName: string, input: unknown): string[] {
  if (!/^(?:write|edit|multiedit|ast_edit|apply_patch)$/iu.test(toolName)) return [];
  const candidates: string[] = [];
  strings(input, candidates);
  const paths = candidates.flatMap(candidate => {
    const patched = patchPaths(candidate);
    return patched.length > 0 ? patched : [candidate];
  });
  return [...new Set(paths.filter(path => path.trim() !== "" && !/[\0\r\n]/u.test(path)))];
}

/** Tools that run arbitrary commands can write any source without a preimage, so their results reconcile the workspace. */
export function isCommandTool(toolName: string): boolean {
  return /bash|shell|exec|terminal|command|cmd/iu.test(toolName);
}

/**
 * How long a turn checkpoint or a command result waits for a checkout-wide source walk, and a code-rule boundary
 * waits for the session's evaluator to open, before answering with what it has. The work goes on, and what it finds
 * is audited and delivered at a later boundary; waiting longer would hold the session's event lane past the host's
 * per-event timeout.
 */
export const BOUNDARY_DISCOVERY_WAIT_MS = 10_000;

/**
 * A boundary answered while a source walk was still running: its shell writes are checked later, or never for `close`,
 * which a worktree walk still due also makes pending.
 */
export type DiscoveryPendingBoundary = "turn" | "final" | "command" | "close";

/**
 * One place the walks a checkpoint answered after could not fully check: a folder or file the last walk of the session
 * folder or of the other worktrees could not read, or an unlisted file too large to read, where shell writes went
 * unchecked. The checkpoint's `source-incomplete` failure joins each as `path: reason`.
 */
export interface DiscoveryGap {
  readonly boundary: Extract<CodeRuleCheckpoint, "turn" | "final">;
  /** The place as the session folder names it (`src/locked`, `.` for the folder, `../other/src` in another worktree); absent when the walk failed whole. */
  readonly path?: string;
  /** Why, as the walk was told: `EACCES: permission denied, opendir '…'`, `Source exceeds 1000000 byte policy limit`. */
  readonly reason: string;
  /**
   * What tells this gap from another, whatever walk met it and in whatever order: its path and its reason's kind (the
   * error code, else the reason). The same key is the same gap, though its reason's text may differ (`opendir`, `open`).
   */
  readonly key: string;
  /**
   * The one gap past `MAX_TOLD_GAPS`: no place, it says the walks met more places than the session's record takes, and
   * none of them is told.
   */
  readonly more?: true;
}

/** A place the session's record holds (`recordedDiscoveryGaps`): its path and why, as `onDiscoveryGaps` was told them. */
export interface RecordedDiscoveryGap {
  readonly path?: string | undefined;
  readonly reason: string;
}

/**
 * A finished tool edit of a file no rule checked because Git ignores it (`CodeRuleAuditor.ignoredEdit`): the file as
 * the session folder names it, and the pattern that ignores it, with its file (as `shownIgnoreFile` names one) and line.
 */
export type IgnoredEdit = Omit<IgnoredEditMatch, "source"> & { path: string; source?: string };

/** Captures only changed source snapshots; the startup snapshot is discovery state, never evaluator input. */
export class CodeRuleBridge {
  readonly #cwd: string;
  readonly #auditor: CodeRuleAuditor;
  readonly #onRevision: ((sessionId: string, revision: number, paths: readonly string[]) => void) | undefined;
  readonly #observations: SourceObservationCache | undefined;
  readonly #pending = new Map<string, PendingEdit>();
  /** Finished edits kept rewindable by their tool call, from `finish` until `release`. */
  readonly #rewindable = new Map<string, RewindableEdit>();
  /** Command tool calls whose results must reconcile shell writes. */
  readonly #commands = new Set<string>();
  /**
   * What the session last observed of each source of its folder, and of each source elsewhere that a tool call of it
   * read; of the other worktrees, only where that differs from the snapshot `#worktreeView` holds (`#observe`).
   */
  readonly #observed = new Map<string, ObservedSource>();
  /** The other worktrees as the shared walk last showed them to this session. */
  readonly #worktreeView = new WorktreeView();
  /** The walk of the other worktrees this session shares with others (`worktreeWalks`), else its own, made on first use. */
  #worktreeWalks: WorktreeWalks | (() => WorktreeWalks) | undefined;
  /**
   * The text each source this session changed was last seen with: the preimage of its next change made without one
   * captured, once its digest proves it (`#recoverPreimages`). The baseline is not kept, but for a source a walk first
   * sees with text Git may not hold (`#keepFirstSeenTexts`), which no other preimage could give; that text is no change
   * of the session's (`first-sight`), and takes at most `FIRST_SIGHT_TEXT_BYTES`. At most `MAX_SESSION_TEXT_BYTES` of
   * them are kept, the least recently changed evicted first: an evicted source falls back to Git.
   */
  readonly #texts = new Map<string, { text: string; bytes: number; origin: TextOrigin }>();
  #textBytes = 0;
  /**
   * The sources the evaluator held as the session's before its runtime suspended it (`SessionWalkResume.held`), whose
   * text the session's new evaluator never saw: the first walk compares each with what was observed of it then, though
   * that walk is a baseline, and a change another writer makes to one is recorded as it was before the suspension, until
   * the source's text is next kept or dropped. Their text, where it is unchanged, is kept too (`#restoreHeldTexts`).
   */
  readonly #resumedHeld = new Set<string>();
  /** Where the first `openWorkspace` goes on from, for a session its runtime suspended; taken once. */
  #resume: SessionWalkResume | undefined;
  /** The calls the session's ledger held open as it opened, which the first `openWorkspace` claims (`#claimHeldCalls`). */
  #heldCalls: readonly HeldCall[];
  /** Settles once the first `openWorkspace` has read back the held sources' text (`#restoreHeldTexts`), or found none to. */
  readonly #heldTexts = Promise.withResolvers<void>();
  /** The calls the runtime answered while the evaluator opened (`unwatchedCall`), until the workspace opens and takes them. */
  #unwatched = new ToolCallWindows();
  #lastResponse = emptyResponse(0);
  #sessionId: string | undefined;
  #revision = 0;
  #queue: Promise<void> = Promise.resolve();
  #discovery: SourceDiscovery | undefined;
  readonly #discoveryWaitMs: number;
  readonly #onDiscoveryPending: ((boundary: DiscoveryPendingBoundary) => void) | undefined;
  readonly #onDiscoveryGaps: ((gaps: readonly DiscoveryGap[]) => void) | undefined;
  readonly #recordedDiscoveryGaps: ((sessionId: string, limit: number) => Iterable<RecordedDiscoveryGap>) | undefined;
  /**
   * The key of each gap the session's record holds: those it held as the workspace opened (`recordedDiscoveryGaps`), and
   * each told to `onDiscoveryGaps` since, once the call returned (`#tellGaps`).
   */
  readonly #toldGaps = new Set<string>();
  readonly #onIgnoredEdit: ((edit: IgnoredEdit) => void) | undefined;
  readonly #onIgnoreRulesChanged: ((change: IgnoreRulesChange) => void) | undefined;
  /**
   * The session's ignore rule files, read as it opens while `onIgnoreRulesChanged` is told of their changes. The open
   * never waits for them; the session's first tool call does, so what they hold before it runs is the baseline.
   */
  #ignoreRules: Promise<IgnoreRuleFiles | undefined> = Promise.resolve(undefined);
  /** The `.gitignore` files walks found new, changed or gone since the last look at the ignore rules, session-relative. */
  readonly #walkedIgnoreChanges = new Set<string>();
  readonly #worktreeWalkGapMs: number | undefined;
  /** The most of `#texts` first-sight text may take (`FIRST_SIGHT_TEXT_BYTES`), and how much it takes now. */
  readonly #firstSightTextBytes: number;
  #firstSightBytes = 0;
  /** The most `#texts` may take (`MAX_SESSION_TEXT_BYTES`). */
  readonly #sessionTextBytes: number;

  constructor(
    cwd: string,
    auditor: CodeRuleAuditor,
    options: {
      onRevision?(sessionId: string, revision: number, paths: readonly string[]): void;
      observations?: SourceObservationCache;
      /**
       * How long a boundary waits for a source walk; `BOUNDARY_DISCOVERY_WAIT_MS` by default. `Infinity` waits for the
       * walk however long it takes.
       */
      discoveryWaitMs?: number;
      /** Told when a boundary answers, or the bridge closes, while a source walk is still running. */
      onDiscoveryPending?(boundary: DiscoveryPendingBoundary): void;
      /**
       * Told of the gaps of the walks (`DiscoveryGap`) a checkpoint answers with that the session's record does not hold,
       * each once per key, at most `MAX_TOLD_GAPS` of them in the session and `MAX_TOLD_AT_ONCE` a checkpoint, in one
       * call, so they can be written together; a call that throws is made again at the next checkpoint. A walk still to
       * finish is `onDiscoveryPending`'s.
       */
      onDiscoveryGaps?(gaps: readonly DiscoveryGap[]): void;
      /**
       * The first `limit` gaps the record of `sessionId` holds, as `onDiscoveryGaps` was told them, read as the workspace
       * opens: the session's bound and its once per place hold across a reopened session and a daemon restart. Without
       * it, they hold while the bridge serves the session.
       */
      recordedDiscoveryGaps?(sessionId: string, limit: number): Iterable<RecordedDiscoveryGap>;
      /** Told of each file a finished tool edit wrote that no rule checked because Git ignores it. */
      onIgnoredEdit?(edit: IgnoredEdit): void;
      /** Told when the session changes Git's ignore rules; without it they are not watched. */
      onIgnoreRulesChanged?(change: IgnoreRulesChange): void;
      /** `WORKTREE_WALK_GAP_MS` by default, for the bridge's own walk of the other worktrees. */
      worktreeWalkGapMs?: number;
      /**
       * The walk of the other worktrees the bridge shares with the other sessions of its repository, in its runtime or
       * engine service; one of its own otherwise. Shared, each worktree is walked and held once however many sessions
       * watch it.
       */
      worktreeWalks?: WorktreeWalks | (() => WorktreeWalks);
      /** `FIRST_SIGHT_TEXT_BYTES` by default. */
      firstSightTextBytes?: number;
      /** `MAX_SESSION_TEXT_BYTES` by default. */
      sessionTextBytes?: number;
      /** Where the session's walks stood when its runtime suspended it: its first walk goes on from there, not from a baseline. */
      resume?: SessionWalkResume;
      /**
       * The calls the session's ledger holds open as it opens: the first walk leaves their files to each call's own check,
       * as it leaves a begun call's, so an edit one made while the session was closed is not read as another writer's.
       */
      heldCalls?: readonly HeldCall[];
    } = {},
  ) {
    this.#cwd = resolve(cwd);
    this.#auditor = auditor;
    this.#onRevision = options.onRevision;
    this.#observations = options.observations;
    this.#resume = options.resume;
    this.#heldCalls = options.heldCalls ?? [];
    // A session opened fresh has no text to read back before its first event is answered.
    if (options.resume === undefined) this.#heldTexts.resolve();
    this.#discoveryWaitMs = options.discoveryWaitMs ?? BOUNDARY_DISCOVERY_WAIT_MS;
    this.#onDiscoveryPending = options.onDiscoveryPending;
    this.#onDiscoveryGaps = options.onDiscoveryGaps;
    this.#recordedDiscoveryGaps = options.recordedDiscoveryGaps;
    this.#onIgnoredEdit = options.onIgnoredEdit;
    this.#onIgnoreRulesChanged = options.onIgnoreRulesChanged;
    this.#worktreeWalkGapMs = options.worktreeWalkGapMs;
    this.#worktreeWalks = options.worktreeWalks;
    this.#firstSightTextBytes = options.firstSightTextBytes ?? FIRST_SIGHT_TEXT_BYTES;
    this.#sessionTextBytes = options.sessionTextBytes ?? MAX_SESSION_TEXT_BYTES;
  }

  /** Resolves once the evaluator is open; the baseline walk runs behind it and delays only the next checkpoint or command result. */
  async openWorkspace(sessionId: string): Promise<void> {
    await this.#enqueue(async () => {
      const startedAt = await filesystemClock();
      const previous = this.#discovery;
      if (previous !== undefined) previous.cancelled = true;
      const unwatched = previous?.worktrees.watch?.close();
      if (this.#sessionId !== undefined) {
        this.#sessionId = undefined;
        await this.#auditor.close();
      }
      this.#pending.clear();
      this.#rewindable.clear();
      this.#commands.clear();
      this.#observed.clear();
      this.#worktreeView.clear();
      this.#texts.clear();
      this.#resumedHeld.clear();
      this.#textBytes = 0;
      this.#firstSightBytes = 0;
      this.#discovery = undefined;
      await previous?.git.stop();
      await unwatched;
      const root = await realpath(this.#cwd);
      // Before the evaluator opens, which takes seconds: a shell write the agent makes meanwhile finds the text it left.
      const from = await this.#takeResume(root, startedAt);
      await this.#auditor.openWorkspace(sessionId);
      const git = new GitReads();
      // After the auditor opens, so it can name the checkouts whose ignore rules apply.
      this.#ignoreRules = this.#openIgnoreRules(root, git);
      this.#sessionId = sessionId;
      this.#revision = 0;
      this.#lastResponse = emptyResponse(0);
      this.#readToldGaps(sessionId);
      const discovery: SourceDiscovery = {
        startedAt: from.since, walkedFrom: from.since, root, cancelled: false, idle: undefined, baselineComplete: false, requested: undefined,
        versions: new Map(), baselineRoots: new Set(), withdrawn: new Set(), worktrees: worktreeDiscovery(), gaps: [], unread: new Map(), reported: new Map(), pendingReported: false,
        heads: new Map(from.gitSeen.heads), git, unmerged: new Map(from.gitSeen.unmerged), passedCheckouts: new Set(from.gitSeen.passedCheckouts),
        readUnmerged: gitUnmergedReader(), toolCalls: this.#unwatched, missedFiles: new Map(), listedEnds: 0,
        late: new Set(), askingAgain: new Map(), listingsAgain: new Set(), listingSlots: { free: LISTINGS_AGAIN_AT_ONCE, waiting: [] },
      };
      this.#unwatched = new ToolCallWindows();
      // Before the first walk can start, which would otherwise read a held call's edit as another writer's change.
      this.#claimHeldCalls();
      this.#discovery = discovery;
      // The lowest priority: whichever boundary first waits on the baseline labels what it finds.
      void this.#requestDiscovery(discovery, "edit");
    });
  }

  /**
   * Where the session's walks start: `now` with no Git seen, or for a session its runtime suspended, where they stood
   * then and what they saw of Git, with each source the evaluator held kept as last observed, so the first walk compares
   * it as the open session's next walk would have, and finds it changed or gone. The text of each such source still as
   * observed is read back (`#restoreHeldTexts`), and `heldTextsRead` settles once it is.
   */
  async #takeResume(root: string, now: number): Promise<Pick<SessionWalkResume, "since" | "gitSeen">> {
    const resume = this.#resume;
    this.#resume = undefined;
    try {
      if (resume === undefined) return { since: now, gitSeen: NO_GIT_SEEN };
      this.#holdResumed(resume.held);
      await this.#restoreHeldTexts(root, resume.held);
      return { since: Math.min(now, resume.since), gitSeen: resume.gitSeen };
    } finally {
      this.#heldTexts.resolve();
    }
  }

  /**
   * Holds the files each call the ledger held open captured as pending (`heldCalls`), as `begin` does, so walks leave
   * them to the call's check: `begin` again with these preimages, then `finish`, which reads the edit once, as the
   * call's. Taken once, by the first `openWorkspace`.
   */
  #claimHeldCalls(): void {
    const calls = this.#heldCalls;
    this.#heldCalls = [];
    for (const call of calls) {
      this.#pending.set(call.toolCallId, { paths: [...call.paths], preimages: new Map(Object.entries(call.preimages)) });
    }
  }

  /** Counts each source of `held` as the evaluator's, observed as it was last (`#resumedHeld`). */
  #holdResumed(held: SessionWalkResume["held"]): void {
    for (const [path, observed] of held) {
      this.#resumedHeld.add(path);
      if (observed !== undefined) this.#observed.set(path, observed);
    }
  }

  /** Settles once the rules the auditor builds ahead of any call as the session opens are built (`rulesBuiltAhead`). */
  async rulesBuiltAhead(): Promise<void> {
    await this.#auditor.rulesBuiltAhead?.();
  }

  /**
   * Settles once the first `openWorkspace` has read back the text of the sources the session held before its runtime
   * suspended it (`#restoreHeldTexts`), which waits for no rule to compile; at once for a session opened fresh, and at
   * the latest after `#discoveryWaitMs`. The runtime answers a reopened session's first event after it, so the agent's
   * next command, which may rewrite such a source, is measured from the text the agent left there.
   */
  async heldTextsRead(): Promise<void> {
    const bound = Promise.withResolvers<void>();
    const timer = waitTimer(this.#discoveryWaitMs, bound.resolve);
    try {
      await Promise.race([this.#heldTexts.promise, bound.promise]);
    } finally {
      clearTimeout(timer);
    }
  }

  /**
   * Keeps the text of each source the evaluator held before the suspension whose bytes still have the digest last
   * observed: the text the open session kept, from which its next shell write to the source is measured. The suspension
   * keeps no text, so a suspended session holds none in memory; one changed or gone meanwhile is left to the first walk,
   * which records the change.
   */
  async #restoreHeldTexts(root: string, held: SessionWalkResume["held"]): Promise<void> {
    const digests: [path: string, digest: string][] = [];
    for (const [path, observed] of held) if (observed?.digest !== undefined) digests.push([path, observed.digest]);
    while (digests.length > 0) {
      const wave = digests.splice(0, OBSERVATION_CONCURRENCY);
      const texts = await Promise.all(wave.map(([path, digest]) => textWithDigest(resolve(root, path), digest)));
      wave.forEach(([path], index) => this.#keepHeldText(path, texts[index]));
    }
  }

  /**
   * Keeps `text` as the session's own text of `path`. The path stays among `#resumedHeld`, so the first walk compares the
   * source with what was observed of it before the suspension, though that walk is a baseline.
   */
  #keepHeldText(path: string, text: string | undefined): void {
    if (text === undefined) return;
    this.#rememberText(path, text, "edit");
    this.#resumedHeld.add(path);
  }

  /**
   * Where the session's walks stand (`SessionWalkResume`), for its runtime to suspend it; undefined while the workspace is
   * not open. Read in turn with the walks' changes, so none lands in it half applied.
   */
  walkResume(): Promise<SessionWalkResume | undefined> {
    return this.#enqueue(() => Promise.resolve(this.#walkResumeNow()));
  }

  #walkResumeNow(): SessionWalkResume | undefined {
    const discovery = this.#discovery;
    if (discovery === undefined || discovery.cancelled) return undefined;
    const held = new Map<string, ObservedSource | undefined>();
    for (const path of this.#heldPaths()) held.set(path, this.#observedSource(path));
    const { heads, unmerged, passedCheckouts } = discovery;
    return {
      since: Math.min(discovery.walkedFrom, discovery.worktrees.walkedFrom ?? discovery.walkedFrom),
      held,
      gitSeen: { heads: new Map(heads), unmerged: new Map(unmerged), passedCheckouts: new Set(passedCheckouts) },
    };
  }

  /** The sources whose text the evaluator holds as the session's (`#heldByEvaluator`). */
  #heldPaths(): string[] {
    return [...this.#resumedHeld, ...[...this.#texts].filter(([, entry]) => HELD_BY_EVALUATOR[entry.origin]).map(([path]) => path)];
  }

  /** Whether the evaluator holds `path`'s text as the session's: one it audited, or one it held before a suspension. */
  #heldByEvaluator(path: string): boolean {
    const held = this.#texts.get(path);
    return (held !== undefined && HELD_BY_EVALUATOR[held.origin]) || this.#resumedHeld.has(path);
  }

  async begin(
    toolCallId: string,
    toolName: string,
    input: unknown,
    restoredPreimages?: Readonly<Record<string, Preimage>>,
  ): Promise<Readonly<Record<string, Preimage>>> {
    const discovery = this.#requireDiscovery();
    discovery.toolCalls.begin(toolCallId, await filesystemClock(), isCommandTool(toolName));
    this.refreshCodeRules();
    const paths = generatedEditPaths(toolName, input);
    this.#auditor.refreshOwners?.(paths);
    // The rules of a checkout no lane opened yet compile off the daemon's thread, before the capture asks for them.
    if (paths.length > 0) await this.#auditor.prepareRules?.(paths);
    // What the ignore files a tool is about to write hold now is the baseline its edit is compared with.
    await this.#rememberIgnoreFiles(paths);
    if (paths.length === 0) {
      if (isCommandTool(toolName)) this.#commands.add(toolCallId);
      return {};
    }
    return await this.#enqueue(async () => {
      this.#assertDiscovery(discovery);
      const preimages = restoredPreimages === undefined
        ? await this.#capturePreimages(discovery, paths)
        : new Map(Object.entries(restoredPreimages));
      for (const path of preimages.keys()) this.#touchSource(discovery, path);
      this.#pending.set(toolCallId, { paths, preimages });
      return Object.fromEntries(preimages);
    });
  }

  /**
   * A call the runtime answered without code rules while the evaluator was still opening: never begun, so nothing of it
   * is captured, but what it writes is the session's own. Its window is taken by the workspace as it opens, so the walk
   * credits what was stamped while it ran; its result reconciles nothing, and what it wrote is the next walk's. A host
   * whose edit tool no hook shows opens one for each turn the same way, as its writes are seen by no call. `toolName`
   * names the call's tool, none for a turn's window; `input` is its tool input, whose files the window names, so a later
   * call's capture of one of them knows it holds an edit no walk has checked yet (`#unobservedOpeningEdit`).
   */
  async unwatchedCall(toolCallId: string, toolName?: string, input?: unknown): Promise<void> {
    const at = await filesystemClock();
    const named = toolName === undefined ? [] : await this.#sessionPaths(generatedEditPaths(toolName, input));
    (this.#discovery?.toolCalls ?? this.#unwatched).begin(toolCallId, at, toolName !== undefined && isCommandTool(toolName), named);
  }

  /**
   * `configuredPaths` as the session names its sources, whatever the auditor reads, which it cannot say before it opens;
   * none where the session's folder cannot be resolved.
   */
  async #sessionPaths(configuredPaths: readonly string[]): Promise<string[]> {
    const root = await realpath(this.#cwd).catch(() => undefined);
    return root === undefined ? [] : configuredPaths.flatMap(path => sessionRelativePath(root, path) ?? []);
  }

  /** Ends the window `unwatchedCall` opened for the call, at its result. */
  async unwatchedResult(toolCallId: string): Promise<void> {
    (this.#discovery?.toolCalls ?? this.#unwatched).finish(toolCallId, await filesystemClock());
  }

  /**
   * A failed command may still have written files, so command results reconcile regardless of `failed`. With
   * `rewindable` the call's own captures are kept for `rewind` until `release`. `reported` is what the tool's result
   * says the file it wrote held before (`ReportedPreimage`): the edit is read against that where the call's own capture
   * already holds the write, as it does when the call reached Rerail only after the tool ran. Such an edit is never
   * rolled back (`RewindableEdit`), so a rewind of it leaves the file as it is.
   */
  async finish(toolCallId: string, failed = false, options: FinishOptions = {}): Promise<readonly CodeRuleResponse[]> {
    const discovery = this.#requireDiscovery();
    discovery.toolCalls.finish(toolCallId, await filesystemClock());
    if (this.#commands.delete(toolCallId)) {
      const responses = await this.#reconcile(discovery);
      await this.#noteIgnoreRules([]);
      return responses;
    }
    return await this.#enqueue(async () => await this.#finishEdit(discovery, toolCallId, failed, options));
  }

  /** The queued part of `finish` for a call that edits files: its edits read against their preimages and audited. */
  async #finishEdit(discovery: SourceDiscovery, toolCallId: string, failed: boolean, options: FinishOptions): Promise<readonly CodeRuleResponse[]> {
    this.#assertDiscovery(discovery);
    const pending = this.#pending.get(toolCallId);
    this.#pending.delete(toolCallId);
    if (pending === undefined || failed) return [];
    this.refreshCodeRules();
    this.#auditor.refreshOwners?.(pending.paths);
    this.#reportIgnoredEdits(discovery.root, pending.paths);
    await this.#noteIgnoreRules(pending.paths);
    const { edits, uncaptured, notFromCapture } = await this.#readEdits(discovery, pending, options.reported);
    if (edits.length === 0) return [];
    // Kept before the audit starts, so a steer its evaluation publishes finds it.
    if (options.rewindable === true) {
      const after = new Map([...pending.preimages.keys()].map(path => [path, this.#observedSource(path)?.digest]));
      this.#rewindable.set(toolCallId, { revision: this.#revision + 1, preimages: pending.preimages, after, fromCapture: notFromCapture.size === 0 });
    }
    return [await this.#auditEdits(discovery, edits, ["edit", "function"], uncaptured)];
  }

  /** Ends the window in which `rewind` may roll the call's edit back. */
  release(toolCallId: string): void {
    this.#rewindable.delete(toolCallId);
  }

  /**
   * Finishes calls begun whose result will not come, as that result would: the person refused the call and the host
   * sent no result, or the result was lost while the daemon was down. Each edit is checked against what its call
   * captured, so a refused call is checked for nothing and what a call did write is checked as its result would have
   * it. A pending call's paths are kept out of every source walk; from here on walks check them again. A command's
   * writes are left to the next walk.
   */
  async finishUnanswered(toolCallIds: readonly string[]): Promise<void> {
    for (const toolCallId of toolCallIds) this.#commands.delete(toolCallId);
    for (const toolCallId of toolCallIds) await this.finish(toolCallId);
  }

  /** Whether a steer at `revision` over `scope` is the check of the call's finished edit, from `finish` until `release`. */
  checks(toolCallId: string, revision: number, scope: readonly string[]): boolean {
    const rewindable = this.#rewindable.get(toolCallId);
    return rewindable !== undefined && rewindable.revision === revision
      && scope.length > 0 && scope.every(path => rewindable.preimages.has(path));
  }

  /**
   * The SHA-256 of `path` (relative to the session's folder, `/`-separated) as this session last observed it, by a walk,
   * a tool call's finish or the snapshot of the worktree holding it (`#observedSource`); undefined when it has observed
   * no file there.
   */
  observedDigest(path: string): string | undefined {
    return this.#observedSource(path)?.digest;
  }

  /**
   * Rolls a finished, still rewindable edit back to its captured preimages, every file or none: a file that did not
   * exist is deleted. Only for a steer at the revision its finish audited whose `scope` lies within the captured
   * paths, while every preimage was read, every file still holds what the edit left, and no other call begun and
   * unfinished, or still rewindable, covers any of them (rolling back over its write would undo it). Each file is
   * replaced whole, so a failed write leaves it as it was; when putting already rolled-back files back fails too, those
   * files stay rolled back and are reported. The rollback is observed as Rerail's own write, so no walk reports it as an
   * uncaptured edit, and audited in `log` mode, so the evaluator holds the restored source without a steer for it; that
   * audit failing is logged, never a reason to lose what was rolled back. Undefined when nothing was rolled back.
   */
  async rewind(toolCallId: string, revision: number, scope: readonly string[]): Promise<RewoundEdit | undefined> {
    const rewindable = this.#takeRewindable(toolCallId, revision, scope);
    if (rewindable === undefined) return undefined;
    const discovery = this.#discovery;
    if (discovery === undefined || discovery.cancelled) return undefined;
    return await this.#enqueue(async () => await this.#rollBack(discovery, toolCallId, rewindable));
  }

  /**
   * The call's edit, out of the window `rewind` may roll it back in, when a steer at `revision` over `scope` may: the edit
   * was read against its captures (`RewindableEdit`), the scope lies within its captured paths and each preimage was
   * read. One holding secrets has no copy to put back, and throws.
   */
  #takeRewindable(toolCallId: string, revision: number, scope: readonly string[]): RewindableEdit | undefined {
    const rewindable = this.#rewindable.get(toolCallId);
    if (rewindable === undefined || rewindable.revision !== revision) return undefined;
    this.#rewindable.delete(toolCallId);
    if (!rewindable.fromCapture || !coversScope(rewindable.preimages, scope)) return undefined;
    const secret = [...rewindable.preimages].find(([, preimage]) => preimage.withheld !== undefined);
    if (secret !== undefined)
      throw new Error(`${secret[0]} holds secrets, by its name or its text, so Rerail kept no copy of it to put back; the edit stays as it is`);
    return rewindable;
  }

  async #rollBack(discovery: SourceDiscovery, toolCallId: string, { preimages, after }: RewindableEdit): Promise<RewoundEdit | undefined> {
    if (this.#abandoned(discovery) || this.#shared(toolCallId, preimages.keys())) return undefined;
    const root = await realpath(this.#cwd);
    const targets = await this.#rollbackTargets(root, preimages, after);
    const written = targets === undefined ? undefined : await this.#writeRollback(toolCallId, targets);
    if (written === undefined) return undefined;
    await this.#auditRollback(discovery, toolCallId, root, written);
    return {
      revision: this.#revision,
      restored: written.filter(target => target.preimage.text !== undefined).map(target => target.location.path),
      deleted: written.filter(target => target.preimage.text === undefined).map(target => target.location.path),
    };
  }

  /** Each file to put back, or undefined when one may not be: unread, no longer what the edit left, or in a folder outside its checkout. */
  async #rollbackTargets(
    root: string,
    preimages: ReadonlyMap<string, Preimage>,
    after: ReadonlyMap<string, string | undefined>,
  ): Promise<RollbackTarget[] | undefined> {
    const targets: RollbackTarget[] = [];
    for (const [path, preimage] of preimages) {
      const target = await this.#rollbackTarget(root, path, preimage, after.get(path));
      if (target === undefined) return undefined;
      targets.push(target);
    }
    return targets;
  }

  async #rollbackTarget(root: string, path: string, preimage: Preimage, digestAfter: string | undefined): Promise<RollbackTarget | undefined> {
    const location = this.#sourceLocation(root, path);
    const current = await this.#readSource(root, path);
    if (location === undefined || !holdsEditText(current, digestAfter)) return undefined;
    return await this.#mayRecreate(root, location, preimage, current) ? { location, preimage, current } : undefined;
  }

  /** A file the edit deleted is created again only in a folder of the checkout that owns it. */
  async #mayRecreate(root: string, location: SourceLocation, preimage: Preimage, current: Source): Promise<boolean> {
    return current.deleted !== true || preimage.text === undefined || await this.#ownsFolder(root, location);
  }

  /**
   * Puts each target back, every file or none: when a write fails, what was already put back takes the edit's text again.
   * The files that stay rolled back, as putting them back failed too, or undefined when none did.
   */
  async #writeRollback(toolCallId: string, targets: readonly RollbackTarget[]): Promise<RollbackTarget[] | undefined> {
    const written: RollbackTarget[] = [];
    try {
      for (const target of targets.filter(target => target.current.text !== target.preimage.text)) {
        await this.#writeSource(target.location, target.preimage.text);
        written.push(target);
      }
      return written;
    } catch (error) {
      const stuck = await this.#undoRollback(toolCallId, written, error);
      return stuck.length === 0 ? undefined : stuck;
    }
  }

  async #undoRollback(toolCallId: string, written: readonly RollbackTarget[], error: unknown): Promise<RollbackTarget[]> {
    const stuck: RollbackTarget[] = [];
    let undoError: unknown;
    for (const target of written) {
      await this.#writeSource(target.location, target.current.text).catch((failure: unknown) => {
        stuck.push(target);
        undoError ??= failure;
      });
    }
    const kept = stuck.length === 0 ? "" : `; ${stuck.map(target => target.location.path).join(", ")} stay rolled back: ${errorMessage(undoError)}`;
    process.stderr.write(`rerail: could not roll back tool call ${toolCallId}: ${errorMessage(error)}${kept}\n`);
    return stuck;
  }

  /** The rollback is audited in `log` mode; that failing is logged, never a reason to lose what was rolled back. */
  async #auditRollback(discovery: SourceDiscovery, toolCallId: string, root: string, written: readonly RollbackTarget[]): Promise<void> {
    try {
      const edits = await this.#rolledBackEdits(discovery, root, written);
      // Every rolled-back file has its preimage (the edit's text), so nothing is left to recover.
      if (edits.length > 0) await this.#auditEdits(discovery, edits, ["edit", "function"], new Map(), "log");
    } catch (error) {
      process.stderr.write(`rerail: could not audit the rollback of tool call ${toolCallId}: ${errorMessage(error)}\n`);
    }
  }

  /** Each rolled-back file as Rerail's own write, from the edit's text: no walk reports it as an uncaptured edit. */
  async #rolledBackEdits(discovery: SourceDiscovery, root: string, written: readonly RollbackTarget[]): Promise<CodeRuleEdit[]> {
    const edits: CodeRuleEdit[] = [];
    for (const { location, current } of written) {
      const source = await this.#readSource(root, location.path);
      this.#touchSource(discovery, location.path);
      if (source === undefined) continue;
      this.#updateObservedSource(source);
      const edit = this.#editFromToolSource(source, current.text === undefined ? {} : { text: current.text });
      if (edit !== undefined) edits.push(edit);
    }
    return edits;
  }

  /** Whether a call other than `toolCallId`, begun and unfinished or still rewindable, covers any of `paths`. */
  #shared(toolCallId: string, paths: Iterable<string>): boolean {
    const others = [...this.#pending, ...this.#rewindable].flatMap(([id, edit]) => (id === toolCallId ? [] : [edit.preimages]));
    for (const path of paths) if (others.some(preimages => preimages.has(path))) return true;
    return false;
  }

  /** Whether the folder a file is to be created in lies in the checkout that owns the file. */
  async #ownsFolder(root: string, location: SourceLocation): Promise<boolean> {
    const owner = this.#owningCheckout(root, location.path);
    if (owner === undefined) return false;
    const folder = await realpath(dirname(location.absolute)).catch(() => undefined);
    return folder !== undefined && withinFolder(owner, folder);
  }

  /**
   * Replaces the file whole with `text`, written to a temporary file beside it and renamed over it, so a failed write
   * leaves the file as it was; a symlink or other non-regular file there is refused, never followed or replaced. An
   * existing file keeps its permissions. Without text the file is removed.
   */
  async #writeSource(location: SourceLocation, text: string | undefined): Promise<void> {
    if (text === undefined) {
      await unlink(location.absolute).catch((error: unknown) => {
        if (!hasErrorCode(error, "ENOENT")) throw error;
      });
      return;
    }
    const existing = await lstat(location.absolute).catch((error: unknown) => {
      if (error instanceof Error && "code" in error && error.code === "ENOENT") return undefined;
      throw error;
    });
    if (existing !== undefined && !existing.isFile()) throw new Error(`${location.path} is not a regular file`);
    const temporary = join(dirname(location.absolute), `.${basename(location.absolute)}.rerail-${randomBytes(6).toString("hex")}.tmp`);
    // A new file under a random name: O_CREAT with O_EXCL refuses a name that exists, which POSIX says includes a
    // symbolic link, so O_NOFOLLOW (which Windows lacks) would add nothing.
    const handle = await open(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL, 0o666);
    try {
      try {
        if (existing !== undefined) await handle.chmod(existing.mode & 0o7777);
        await handle.writeFile(text, "utf8");
      } finally {
        await handle.close();
      }
      // Retried on Windows while an editor, language server or scanner holds the file; a read-only file is replaced too.
      await renameWithRetry(temporary, location.absolute);
    } catch (error) {
      await unlink(temporary).catch(() => undefined);
      throw error;
    }
  }

  /**
   * Resolves after every requested walk has run, so the checkpoint's violations are delivered with this boundary; a walk
   * that outlasts the boundary's wait is reported pending, and what it finds reaches a later boundary.
   */
  async checkpoint(checkpoint: Extract<CodeRuleCheckpoint, "turn" | "final">): Promise<CodeRuleResponse> {
    const discovery = this.#requireDiscovery();
    discovery.toolCalls.finishAll(await filesystemClock());
    await this.#awaitDiscovery(discovery, checkpoint, checkpoint);
    await this.#noteIgnoreRules([]);
    this.#assertDiscovery(discovery);
    return this.#discoveryResponse(discovery, checkpoint);
  }

  async #reconcile(discovery: SourceDiscovery): Promise<readonly CodeRuleResponse[]> {
    const revision = this.#revision;
    this.refreshCodeRules();
    await this.#awaitDiscovery(discovery, "edit", "command");
    this.#assertDiscovery(discovery);
    return this.#revision === revision ? [] : [this.#lastResponse];
  }
  /** The auditor's `settleGeneratedEdits` once the bridge's queued work is done; with `steering`, the edits that may steer. */
  async settleGeneratedEdits(only?: "steering"): Promise<void> {
    this.#requireSession();
    await this.#queue;
    if (this.#auditor.settleGeneratedEdits !== undefined) {
      await this.#auditor.settleGeneratedEdits(only);
    } else {
      await this.#auditor.settle?.();
    }
  }

  /** Whether its wait for the edits that may steer waits for a walk's `log` edits too (the auditor's `steeringWaitsForLog`). */
  steeringWaitsForLog(): boolean {
    return this.#auditor.steeringWaitsForLog?.() === true;
  }

  async settle(): Promise<void> {
    this.#requireSession();
    await this.#queue;
    await this.#auditor.settle?.();
  }

  refreshCodeRules(): void {
    this.#auditor.refreshCodeRules?.();
  }

  updateSettings(settings: Required<AuditSettings>): void {
    this.#auditor.updateSettings?.(settings);
  }

  async audit(input: Parameters<NonNullable<CodeRuleAuditor["audit"]>>[1]): Promise<readonly string[]> {
    const discovery = this.#requireDiscovery();
    const sessionId = this.#requireSession();
    if (this.#auditor.audit === undefined) throw new Error("Policy evaluator does not support scans");
    // Serialize the launch with edits, not the repository traversal with subsequent tool events.
    const { work } = await this.#enqueue(async () => {
      this.#assertDiscovery(discovery);
      return { work: this.#auditor.audit!(sessionId, input) };
    });
    return await work;
  }
  interrupt(options?: AuditorCloseOptions): void {
    this.#auditor.interrupt?.(options);
  }


  async close(options?: AuditorCloseOptions): Promise<void> {
    const discovery = this.#discovery;
    if (discovery === undefined || discovery.cancelled) return;
    // Cancel before reporting and before joining the operation queue: neither the report nor a blocked filesystem read
    // may keep the walk going.
    discovery.cancelled = true;
    // A walk a boundary already gave up on is abandoned here: what it had not reached is never checked. No boundary waits
    // for the worktree walk, so one still due is always such a walk.
    if (sessionWalkAbandoned(discovery) || worktreeWalkAbandoned(discovery)) this.#reportDiscoveryPending("close");
    // No walk of the other worktrees reads through this session, or reaches it, any more.
    const unwatched = discovery.worktrees.watch?.close();
    await this.#enqueue(async () => {
      if (this.#discovery !== discovery) return;
      this.#shareOwnObservations(discovery);
      this.#sessionId = undefined;
      this.#pending.clear();
      this.#rewindable.clear();
      this.#commands.clear();
      this.#observed.clear();
      this.#worktreeView.clear();
      this.#texts.clear();
      this.#resumedHeld.clear();
      this.#textBytes = 0;
      this.#firstSightBytes = 0;
      this.#discovery = undefined;
      // The walk itself is never waited for, but every Git it or the ignore rules started is: on Windows one still running
      // would keep the checkout from being removed or renamed once the session has ended.
      await discovery.git.stop();
      await unwatched;
      await this.#auditor.close(options);
    });
  }

  /**
   * Keeps only the session folder's own sources in the observations later sessions there start from. The other
   * worktrees' sources, and files tool calls wrote outside the folder, stay in them only while a session walks them:
   * the runtime keeps a folder's observations as long as it serves the folder, so with them every folder a session had
   * closed in held an observation of every file of every worktree, and the daemon's heap grew with those folders times
   * the files of all the repository's worktrees.
   */
  #shareOwnObservations(discovery: SourceDiscovery): void {
    const shared = this.#observations?.get(this.#cwd);
    if (shared === undefined) return;
    // On the event loop, which every session's hooks wait on: no copy of the whole map before the filter.
    const own = new Map<string, ObservedSource>();
    for (const [path, source] of shared) if (inSessionFolder(discovery.worktrees.roots, path)) own.set(path, source);
    this.#observations?.set(this.#cwd, own);
  }

  /**
   * Each source's text as the call begins, its preimage, which the session takes as what it observed of the source. A
   * source holding an edit made while the evaluator opened that no walk has checked yet (`#unobservedOpeningEdit`) gives
   * no text to read the call's edit against: the edit is read over its whole file, as the evaluator's first walk reads a
   * change it finds, so the earlier edit's lines are checked once, here, rather than taken as the source's earlier
   * state and never checked.
   */
  async #capturePreimages(discovery: SourceDiscovery, paths: readonly string[]): Promise<ReadonlyMap<string, Preimage>> {
    const root = await realpath(this.#cwd);
    const preimages = new Map<string, Preimage>();
    for (const configuredPath of [...paths].sort(compareDeterministicStrings)) {
      const source = await this.#readSource(root, configuredPath);
      if (source === undefined) continue;
      const preimage = await this.#unobservedOpeningEdit(discovery, source) ? { readError: CHANGED_IN_INITIAL_DISCOVERY } : preimageOf(source);
      this.#updateObservedSource(source);
      if (preimage !== undefined) preimages.set(source.path, preimage);
    }
    return preimages;
  }

  /**
   * Whether `source` holds an edit of a call answered while the evaluator opened, never begun, that no walk or call of
   * the session has observed: the call named the file and ran when it was last written (`ToolCallWindows.namedAt`), and
   * the session holds no observation of it. The evaluator's first walk would find the file changed during initial
   * discovery and check it whole; a capture taken as the file's earlier state would leave that walk nothing to find.
   */
  async #unobservedOpeningEdit(discovery: SourceDiscovery, source: Source): Promise<boolean> {
    if (this.#observedSource(source.path) !== undefined) return false;
    const stamp = await writtenAt(resolve(discovery.root, source.path));
    return stamp !== undefined && discovery.toolCalls.namedAt(source.path, stamp);
  }

  /**
   * The tool call's edits, each read against its preimage (`editPreimage`): the call's own, or the one its result
   * reports. And the digest each source had before it where the call's start captured no preimage, and the sources
   * whose edit was read against anything but the call's own capture.
   */
  async #readEdits(
    discovery: SourceDiscovery,
    pending: PendingEdit,
    reported: ReportedPreimage | undefined,
  ): Promise<{ edits: CodeRuleEdit[]; uncaptured: Map<string, string>; notFromCapture: Set<string> }> {
    const root = await realpath(this.#cwd);
    const edits: CodeRuleEdit[] = [];
    const uncaptured = new Map<string, string>();
    const preimages = { captured: pending.preimages, reported: this.#locatedReport(root, reported), notFromCapture: new Set<string>() };
    for (const configuredPath of [...pending.paths].sort(compareDeterministicStrings)) {
      const edit = await this.#readToolEdit(discovery, root, configuredPath, preimages, uncaptured);
      if (edit !== undefined) edits.push(edit);
    }
    return { edits, uncaptured, notFromCapture: preimages.notFromCapture };
  }

  /** `reported` with its path as the session names its sources; undefined when it names none the auditor takes. */
  #locatedReport(root: string, reported: ReportedPreimage | undefined): ReportedPreimage | undefined {
    const path = reported === undefined ? undefined : this.#sourceLocation(root, reported.path)?.path;
    return reported === undefined || path === undefined ? undefined : { ...reported, path };
  }

  /**
   * The edit of the source at `configuredPath`, read against its preimage (`editPreimage`): the call's own capture, or
   * what the tool's result reports for that source. A source whose edit is read against anything but its capture's
   * text joins `notFromCapture`.
   */
  async #readToolEdit(
    discovery: SourceDiscovery,
    root: string,
    configuredPath: string,
    preimages: {
      readonly captured: ReadonlyMap<string, Preimage>; readonly reported: ReportedPreimage | undefined; readonly notFromCapture: Set<string>;
    },
    uncaptured: Map<string, string>,
  ): Promise<CodeRuleEdit | undefined> {
    const source = await this.#readSource(root, configuredPath);
    if (source === undefined) return undefined;
    this.#touchSource(discovery, source.path);
    const reported = preimages.reported?.path === source.path ? preimages.reported : undefined;
    const captured = preimages.captured.get(source.path);
    const preimage = editPreimage(source, captured, reported);
    if (!readAsCaptured(preimage, captured)) preimages.notFromCapture.add(source.path);
    const before = this.#observedSource(source.path)?.digest;
    const edit = this.#readEdit(source, preimage);
    if (edit === undefined) return undefined;
    const prior = uncapturedPrior(edit, preimage, before);
    if (prior !== undefined) uncaptured.set(edit.path, prior);
    return edit;
  }

  /** Queues a pass for `checkpoint` and resolves once the walk is idle, i.e. every pass requested so far has run. */
  #requestDiscovery(discovery: SourceDiscovery, checkpoint: DiscoveryCheckpoint): Promise<void> {
    if (discovery.cancelled) return Promise.resolve();
    discovery.requested = strongerCheckpoint(checkpoint, discovery.requested);
    // The traversal never occupies #queue. Only completed snapshots join ordered tool revisions.
    discovery.idle ??= this.#runDiscovery(discovery);
    return discovery.idle;
  }

  /** A requested walk, waited for at most `#discoveryWaitMs`; a walk that outlasts it is reported pending at `boundary`. */
  async #awaitDiscovery(
    discovery: SourceDiscovery,
    checkpoint: DiscoveryCheckpoint,
    boundary: Exclude<DiscoveryPendingBoundary, "close">,
  ): Promise<void> {
    const budget = Promise.withResolvers<boolean>();
    const timer = waitTimer(this.#discoveryWaitMs, () => budget.resolve(false));
    try {
      const walked = await Promise.race([this.#requestDiscovery(discovery, checkpoint).then(() => true), budget.promise]);
      if (!walked && !discovery.cancelled) {
        discovery.pendingReported = true;
        this.#reportDiscoveryPending(boundary);
      }
    } finally {
      clearTimeout(timer);
    }
  }

  /** Recording a boundary that answered before its shell-write check is best-effort: it never fails the boundary. */
  #reportDiscoveryPending(boundary: DiscoveryPendingBoundary): void {
    reportBestEffort(() => this.#onDiscoveryPending?.(boundary));
  }

  /**
   * Tells `onDiscoveryGaps` the first `MAX_TOLD_AT_ONCE` of the walks' gaps (`walkGaps`) the record does not hold
   * (`#untold`), in one call, best-effort too: it never fails the checkpoint. A gap is held once the call returned; when
   * it throws, the next checkpoint tells the same gaps again, so a database that refuses writes costs one failed write
   * and one log line a boundary, and loses nothing. Nothing is done once the record holds all it will (`UNTOLD_KEY`).
   */
  #tellGaps(boundary: DiscoveryGap["boundary"], discovery: SourceDiscovery): void {
    const record = this.#onDiscoveryGaps;
    if (record === undefined || this.#toldGaps.has(UNTOLD_KEY)) return;
    const untold = this.#untoldOf(boundary, walkGaps(discovery));
    if (untold.size === 0 || !reportBestEffort(() => record([...untold.values()]))) return;
    for (const key of untold.keys()) this.#toldGaps.add(key);
  }

  /** The first `MAX_TOLD_AT_ONCE` of `gaps` not yet told (`#untold`), by key; none the walk could not yet judge (`WalkGap.unrecorded`). */
  #untoldOf(boundary: DiscoveryGap["boundary"], gaps: readonly WalkGap[]): Map<string, DiscoveryGap> {
    const untold = new Map<string, DiscoveryGap>();
    for (const gap of gaps) {
      if (untold.size === MAX_TOLD_AT_ONCE) break;
      const told = gap.unrecorded === true ? undefined : this.#untold(toldGap(boundary, gap), untold);
      if (told !== undefined) untold.set(told.key, told);
    }
    return untold;
  }

  /**
   * Starts what the record of `sessionId` holds over (`recordedDiscoveryGaps`): the record's bound and its once per place
   * are the session's, whatever bridge or daemon serves it. A record that cannot be read starts empty, as a new one does.
   */
  #readToldGaps(sessionId: string): void {
    this.#toldGaps.clear();
    try {
      for (const gap of this.#recordedDiscoveryGaps?.(sessionId, MAX_TOLD_GAPS + 1) ?? []) this.#toldGaps.add(gapKey(gap.path, reasonKind(gap.reason)));
    } catch (error) {
      process.stderr.write(`rerail: reading the places a session's record holds failed: ${errorMessage(error)}\n`);
    }
  }

  /**
   * The session folder's ignore rule files and its checkouts' as they are when it opens, while their changes are reported;
   * Git is asked about them through the session's reads, `git`.
   */
  async #openIgnoreRules(root: string, git: GitReads): Promise<IgnoreRuleFiles | undefined> {
    if (this.#onIgnoreRulesChanged === undefined) return undefined;
    try {
      return await IgnoreRuleFiles.open(root, this.#auditor.ignoreRuleRoots?.() ?? [], git);
    } catch (error) {
      process.stderr.write(`rerail: cannot read Git's ignore rule files for ${root}: ${errorMessage(error)}\n`);
      return undefined;
    }
  }

  /**
   * Reports what made Git's ignore rules ignore more since the last look: each ignore file a finished tool edit's
   * `paths` name, each `.gitignore` a walk found changed, then each exclude file. Best-effort: it never fails the boundary.
   */
  async #noteIgnoreRules(paths: readonly string[]): Promise<void> {
    const files = await this.#ignoreRules;
    const report = this.#onIgnoreRulesChanged;
    if (files === undefined || report === undefined) return;
    const observed = [...this.#walkedIgnoreChanges];
    this.#walkedIgnoreChanges.clear();
    let changes: IgnoreRulesChange[];
    try {
      changes = await files.changes({ named: paths, observed }, this.#auditor.ignoreRuleRoots?.() ?? []);
    } catch (error) {
      process.stderr.write(`rerail: cannot read Git's ignore rule files: ${errorMessage(error)}\n`);
      return;
    }
    for (const change of changes) reportBestEffort(() => report(change));
  }

  /** Takes what the ignore files among a tool edit's `paths` hold before it runs as their baseline; best-effort. */
  async #rememberIgnoreFiles(paths: readonly string[]): Promise<void> {
    const files = await this.#ignoreRules;
    if (files === undefined || paths.length === 0) return;
    await files.remember(paths).catch((error: unknown) => {
      process.stderr.write(`rerail: cannot read Git's ignore rule files: ${errorMessage(error)}\n`);
    });
  }

  /**
   * Hands a walk's `.gitignore` files to the session's ignore rule files: one a walk first sees (its baseline, a
   * worktree's first walk) is remembered as it is, and one new, changed or gone since the walk before is looked at by
   * the next boundary. A `.gitignore` Git ignores is walked too, so one that ignores itself (`*`) is seen.
   */
  async #noteWalkedIgnoreFiles(discovery: SourceDiscovery, walk: WalkStart, current: ReadonlyMap<string, Source>): Promise<void> {
    if (this.#onIgnoreRulesChanged === undefined) return;
    const first: string[] = [];
    for (const path of new Set([...walk.previous.keys(), ...current.keys()])) {
      const seen = this.#walkedIgnoreFile(discovery, walk, current, path);
      if (seen === undefined) continue;
      if (seen === "first" && !(await writtenThisSession(discovery, walk, path))) first.push(path);
      else this.#walkedIgnoreChanges.add(path);
    }
    await this.#rememberIgnoreFiles(first);
  }

  /** How the walk saw `path`, when it is a `.gitignore` the walk answers for: for the first time, or changed since. */
  #walkedIgnoreFile(discovery: SourceDiscovery, walk: WalkStart, current: ReadonlyMap<string, Source>, path: string): "first" | "changed" | undefined {
    if (basename(path) !== GITIGNORE || !walkAnswersFor(discovery, walk, path)) return undefined;
    const before = walk.previous.get(path);
    if (this.#baselinePath(discovery, path, before, walk.baseline)) return "first";
    return before?.stamp === current.get(path)?.stamp ? undefined : "changed";
  }

  /** Reports each file of a finished tool edit that no rule checked because Git ignores it; best-effort, file by file. */
  #reportIgnoredEdits(root: string, paths: readonly string[]): void {
    const report = this.#onIgnoredEdit;
    if (report === undefined) return;
    for (const configuredPath of [...paths].sort(compareDeterministicStrings)) {
      reportBestEffort(() => {
        const edit = this.#ignoredEditOf(root, configuredPath);
        if (edit !== undefined) report(edit);
      });
    }
  }

  /** A tool edit's `configuredPath` when no rule checked it because Git ignores it; undefined when it was checked, or no rule reads it. */
  #ignoredEditOf(root: string, configuredPath: string): IgnoredEdit | undefined {
    const path = sessionRelativePath(root, configuredPath);
    const match = path === undefined ? undefined : this.#auditor.ignoredEdit?.(path);
    if (path === undefined || match === undefined) return undefined;
    const source = match.source ?? undefined;
    return { path, line: match.line, pattern: match.pattern, ...(source === undefined ? {} : { source: shownIgnoreFile(root, source) }) };
  }

  async #runDiscovery(discovery: SourceDiscovery): Promise<void> {
    try {
      while (!discovery.cancelled && discovery.requested !== undefined) {
        try {
          await this.#discoveryPass(discovery);
        } catch (error) {
          await this.#recordDiscoveryError(discovery, error);
        }
      }
    } finally {
      discovery.idle = undefined;
      discovery.pendingReported = false;
      this.#scheduleWorktreeWalk(discovery);
    }
  }

  async #recordDiscoveryError(discovery: SourceDiscovery, error: unknown): Promise<void> {
    const gaps = await this.#failedWalkGaps(error);
    if (this.#discovery === discovery && !discovery.cancelled) discovery.gaps = gaps;
  }

  /**
   * The gap of a walk that failed whole with `error`. One that failed on an EPERM naming a path is looked at again
   * (`lookedAgain`), unless the record holds such a gap (`#holdsRefusal`): when the path is gone or its delete pending, it
   * was being removed, and the gap is said but never recorded.
   */
  async #failedWalkGaps(error: unknown): Promise<readonly WalkGap[]> {
    const gap: WalkGap = { reason: errorMessage(error) };
    const refused = refusedPath(error);
    if (refused === undefined || !hasErrorCode(error, "EPERM") || this.#holdsRefusal(undefined)) return [gap];
    return (await lookedAgain({ path: "", refused, examined: false })) === "there" ? [gap] : [{ ...gap, unrecorded: true }];
  }

  /**
   * Whether the record holds the EPERM gap of `path` (a walk that failed whole when it is undefined), or holds all it will
   * (`UNTOLD_KEY`): a refusal there is a gap, as the record already says, and is not looked at again.
   */
  #holdsRefusal(path: string | undefined): boolean {
    return this.#toldGaps.has(UNTOLD_KEY) || this.#toldGaps.has(gapKey(path, "EPERM"));
  }

  /**
   * The repository's other worktrees as they are now, each relative to the session folder in `WorktreeDiscovery.roots`. One
   * first seen now is a baseline but for what changed since the session opened, which Git's verdicts judge. One gone took
   * its files with it, and none of them is an edit.
   */
  #syncWorktrees(discovery: SourceDiscovery): readonly string[] {
    const worktrees = (this.#auditor.worktreeRoots?.() ?? []).filter(worktree => worktree !== discovery.root);
    const current = new Set(worktrees.map(worktree => sessionPath(discovery.root, worktree)));
    for (const worktree of current) if (!discovery.worktrees.roots.has(worktree)) discovery.worktrees.fresh.add(worktree);
    const removed = new Set([...discovery.worktrees.roots].filter(worktree => !current.has(worktree)));
    if (removed.size > 0) this.#forgetWorktrees(discovery, removed);
    discovery.worktrees.roots = current;
    return worktrees;
  }

  #forgetWorktrees(discovery: SourceDiscovery, removed: ReadonlySet<string>): void {
    for (const worktree of removed) discovery.worktrees.fresh.delete(worktree);
    for (const path of this.#observed.keys()) if (underBaselineRoot(removed, path)) this.#observed.delete(path);
    this.#worktreeView.forget(removed);
  }

  /**
   * Asks for a walk of the other worktrees, behind the session folder's walk and never in its way: a command does not
   * wait for it, and what it finds is recorded, never steered (`#auditWalkEdits`). The walk is shared with every session
   * watching those worktrees (`WorktreeWalks`). A boundary that comes before the next walk is due sets a timer rather
   * than dropping it, and one that comes while a walk runs has it walk again after, so the last write of a turn is still
   * seen.
   */
  #scheduleWorktreeWalk(discovery: SourceDiscovery): void {
    const auditor = this.#auditor;
    if (discovery.cancelled || auditor.worktreeRoots === undefined) return;
    // An auditor that learns its worktrees ahead has each walk learn them first, so the walk's start asks Git nothing.
    const prepare = auditor.learnWorktreeRoots === undefined ? {} : { prepare: (git: GitReads) => auditor.learnWorktreeRoots?.(git) };
    discovery.worktrees.watch ??= this.#sharedWorktreeWalks().watch({
      clock: filesystemClock,
      ...prepare,
      start: () => this.#startWorktreeWalk(discovery),
    });
    discovery.worktrees.watch.request();
  }

  #sharedWorktreeWalks(): WorktreeWalks {
    // A walk given as a function is taken now, at the first walk, so the repository's walk this session joins is the one
    // its other sessions watch then, not one dropped since the session opened.
    if (typeof this.#worktreeWalks === "function") this.#worktreeWalks = this.#worktreeWalks();
    this.#worktreeWalks ??= new WorktreeWalks(this.#worktreeWalkGapMs === undefined ? {} : { gapMs: this.#worktreeWalkGapMs });
    return this.#worktreeWalks;
  }

  /**
   * The session's part in a walk of the other worktrees as it begins: the worktrees it watches now, and what it takes
   * the walk's findings against. Undefined once the session has closed, or when the worktrees cannot be listed.
   */
  #startWorktreeWalk(discovery: SourceDiscovery): WatcherWalk | undefined {
    if (this.#abandoned(discovery)) return undefined;
    let worktrees: readonly string[];
    try {
      worktrees = this.#syncWorktrees(discovery);
    } catch (error) {
      discovery.worktrees.gaps = [{ reason: errorMessage(error) }];
      return undefined;
    }
    const start: WorktreeStart = { worktrees: new Set(discovery.worktrees.roots), versions: new Map(discovery.versions), callsEnded: discovery.toolCalls.ends };
    const rules = { current: false };
    return {
      worktrees: new Set(worktrees),
      checkout: this.#owningCheckout(discovery.root, ""),
      registry: this.#auditor.worktreeRegistry?.(),
      firstSight: new Set(worktrees.filter(worktree => this.#mayFirstSee(discovery, sessionPath(discovery.root, worktree)))),
      startedAt: discovery.startedAt,
      heads: new Set([...discovery.heads.keys()].filter(root => inWorktrees(discovery, root))),
      unmerged: new Set([...discovery.unmerged.keys()].filter(root => inWorktrees(discovery, root))),
      scan: request => this.#scanWorktree(discovery, request, rules),
      holdsRefusal: (root, path) => this.#holdsRefusal(inWorktree(sessionPath(discovery.root, root), path === "." ? "" : path)),
      take: walk => this.#takeWorktreeWalk(discovery, start, walk),
    };
  }

  /**
   * Waits for the session folder's walk in flight, if any, and every pass a boundary asked for meanwhile, however long
   * they outlast the boundary's wait: what the walk met is then on the session.
   */
  async settleWalk(): Promise<void> {
    await this.#discovery?.idle;
  }

  /** Waits for the worktree walk in flight or timed, if any, any walk a boundary asked for meanwhile, and their taking. */
  async settleWorktreeWalk(): Promise<void> {
    await this.#discovery?.worktrees.watch?.settled();
  }

  async #discoveryPass(discovery: SourceDiscovery): Promise<void> {
    const checkpoint = discovery.requested!;
    discovery.requested = undefined;
    await this.#walkSources(discovery, checkpoint);
  }

  /** One walk of the session folder; the other worktrees' walk is shared (`#scheduleWorktreeWalk`), and neither changes what the other owns. */
  async #walkSources(discovery: SourceDiscovery, checkpoint: DiscoveryCheckpoint): Promise<void> {
    // What the worktrees' list would ask Git is learnt first, as a process no thread waits on, so the list below starts
    // none; while the stored list stands there is nothing to wait for. A session that closed meanwhile has its walk stop
    // there, as at the walk's other waits.
    const learning = this.#learnWorktrees(discovery);
    if (learning !== undefined && !(await learning)) return;
    const worktrees = this.#syncWorktrees(discovery);
    const walk: WalkStart = {
      inSession: true, baseline: !discovery.baselineComplete, checkpoint, worktrees: new Set(discovery.worktrees.roots),
      previous: new Map(this.#observed), versions: new Map(discovery.versions),
      // Read before the walk, so a move or conflict the walk only partly saw is still one to the next.
      ...await this.#readCheckouts(discovery),
      callsEnded: discovery.toolCalls.ends,
      observedAt: await filesystemClock(),
    };
    const scan = await this.#scanSources(discovery, walk, worktrees);
    if (this.#abandoned(discovery)) return;
    await this.#enqueue(() => this.#applyWalk(discovery, walk, scan));
    // Reconcile changes to already-observed files made during the initial traversal.
    if (walk.baseline && discovery.requested === undefined) discovery.requested = checkpoint;
  }

  /**
   * What the worktrees' list would ask Git, learnt through the session's Git reads (`learnWorktreeRoots`): resolves
   * whether the session's walk goes on, false once the session closed meanwhile; undefined, with nothing to wait for,
   * when there is nothing to learn.
   */
  #learnWorktrees(discovery: SourceDiscovery): Promise<boolean> | undefined {
    return this.#auditor.learnWorktreeRoots?.(discovery.git)?.then(() => !this.#abandoned(discovery));
  }

  #abandoned(discovery: SourceDiscovery): boolean {
    return discovery.cancelled || this.#discovery !== discovery;
  }

  async #scanSources(discovery: SourceDiscovery, walk: WalkStart, worktrees: readonly string[]): Promise<SourceScan> {
    // An earlier session's baseline of this workspace lets unchanged sources skip the read.
    const seed = walk.baseline ? this.#observations?.get(this.#cwd) : undefined;
    const scan = await this.#observeSources({
      discovery, checkoutOf: path => this.#owningCheckout(discovery.root, path), observedAt: walk.observedAt,
      previousSources: seed === undefined ? walk.previous : new Map([...seed, ...walk.previous]),
      captureChanges: !walk.baseline,
      commandRuns: () => discovery.toolCalls.commandSince(walk.callsEnded) !== undefined,
      baselineRoots: discovery.baselineRoots,
      buffers: new WalkReadBuffers(MAX_SOURCE_BYTES + 1),
      scan: emptyScan(),
      stack: [discovery.root], roots: new Set([discovery.root, ...worktrees]),
      view: this.#auditor.walkView?.() ?? this.#auditor,
      held: path => this.#holdsRefusal(path),
      withdrawn: discovery.withdrawn,
    });
    // Read off the lane, as the scan is, so tool calls are not held up behind it.
    await this.#readFirstSeenTexts(discovery, walk, scan);
    await this.#readPassedCheckouts(discovery, walk, scan);
    return scan;
  }

  /**
   * Reads another worktree for the shared walk (`WorktreeWalks`), asking this session's auditor which files to read and
   * which Git ignores, as the session would name them. Paths are relative to the worktree, so every session watching it
   * takes the same snapshot; undefined when this session closed while it read. The session's rules are refreshed once
   * per walk, before its first read (`rules`): every session watching takes what it reads, and its rules, as of its
   * last event, may predate a rule another session's event already applies.
   */
  async #scanWorktree(discovery: SourceDiscovery, request: WorktreeScan, rules: { current: boolean }): Promise<WorktreeScanned | undefined> {
    if (this.#abandoned(discovery)) return undefined;
    if (!rules.current) {
      rules.current = true;
      this.refreshCodeRules();
    }
    const prefix = sessionPath(discovery.root, request.root);
    const named = (path: string): string => (path === "" ? prefix : `${prefix}/${path}`);
    const asked = this.#auditor.walkView?.() ?? this.#auditor;
    const view: WalkView = {
      acceptsPath: path => asked.acceptsPath(named(path)),
      ignoresPath: (path, directory) => asked.ignoresPath?.(named(path), directory) === true,
    };
    const abandoned = (): boolean => this.#abandoned(discovery);
    const scope: WalkScope = { root: request.root, startedAt: request.startedAt, get cancelled() { return abandoned(); } };
    const seen = { folders: new Set<string>(), leftOut: new Set<string>() };
    const scan = await this.#observeSources({
      discovery: scope, checkoutOf: path => this.#owningCheckout(discovery.root, named(path)), view,
      observedAt: request.observedAt, previousSources: request.previous, captureChanges: request.captureChanges,
      baselineRoots: request.baselineRoots, buffers: new WalkReadBuffers(MAX_SOURCE_BYTES + 1),
      scan: { ...emptyScan(), seen }, stack: [request.root], roots: request.skip,
      // Its findings serve every session watching the worktree, so only the records of them all spare a refusal its look.
      held: request.held,
      withdrawn: request.withdrawn,
    });
    return abandoned() ? undefined : { ...scan, seen };
  }

  /**
   * Takes a walk of the other worktrees: what changed since the snapshots the session took last is read off the lane,
   * then audited and kept on it, as the session folder's walk is. What fails is the walk's error, reported at each
   * boundary.
   */
  async #takeWorktreeWalk(discovery: SourceDiscovery, start: WorktreeStart, walk: WorktreeWalk): Promise<void> {
    if (this.#abandoned(discovery)) return;
    if ("error" in walk) {
      discovery.worktrees.gaps = [{ reason: walk.error }];
      return;
    }
    try {
      const take = await this.#worktreeTake(discovery, start, walk.found, walk.observedAt);
      if (this.#abandoned(discovery)) return;
      await this.#enqueue(async () => {
        if (this.#abandoned(discovery)) return;
        this.#takeSnapshots(discovery, take);
        await this.#applyWalk(discovery, take.walk, take.scan);
      });
    } catch (error) {
      const gaps = await this.#failedWalkGaps(error);
      if (!this.#abandoned(discovery)) discovery.worktrees.gaps = gaps;
    }
  }

  /**
   * What a walk found that this session has yet to take, read off the lane: in each worktree it had taken a snapshot of,
   * the sources whose observation changed and those its own tool calls saw otherwise; in one new to it, the sources
   * that may say something on first sight (`#firstSightPaths`), every other being its baseline as it is.
   */
  async #worktreeTake(
    discovery: SourceDiscovery,
    start: WorktreeStart,
    found: ReadonlyMap<string, WorktreeFindings>,
    observedAt: number,
  ): Promise<WorktreeTake> {
    const previous = new Map<string, ObservedSource>();
    const scan = emptyScan();
    const snapshots = new Map<string, WorktreeSnapshot>();
    const heads = new Map<string, string>();
    const unmerged = new Map<string, ReadonlyMap<string, string>>();
    const kept = this.#keptWorktreePaths(start.worktrees);
    for (const [root, findings] of found) {
      const worktree = sessionPath(discovery.root, root);
      if (!start.worktrees.has(worktree)) continue;
      snapshots.set(worktree, findings.snapshot);
      takeCheckouts(root, findings, { heads, unmerged, passed: scan.unmerged });
      this.#takeFindings(discovery, { root, worktree }, findings, kept.get(worktree) ?? [], { previous, scan });
    }
    const walk: WalkStart = {
      inSession: false, baseline: false, checkpoint: "edit", worktrees: new Set(snapshots.keys()), previous, versions: start.versions,
      heads, unmerged, callsEnded: start.callsEnded, observedAt,
    };
    const views = await this.#readTakenTexts(discovery, walk, scan, found);
    // Not this take's to record: the session keeps what it saw of them (`#takeSnapshots`).
    for (const path of views.keys()) previous.delete(path);
    return { walk, scan, snapshots, views };
  }

  /** The paths under each of `worktrees` (session-relative) where the session's own view differs from its snapshot. */
  #keptWorktreePaths(worktrees: ReadonlySet<string>): Map<string, string[]> {
    const kept = new Map<string, string[]>();
    const keep = (path: string): void => {
      const worktree = rootHolding(worktrees, path);
      if (worktree !== undefined) kept.set(worktree, [...(kept.get(worktree) ?? []), path]);
    };
    for (const path of this.#observed.keys()) keep(path);
    for (const worktree of worktrees) for (const path of this.#worktreeView.hidden(worktree)) keep(path);
    return kept;
  }

  /**
   * Puts into `taken` what this session takes of the findings in the worktree at `root` (physical; `worktree` as the
   * session names it), named as the session names them.
   */
  #takeFindings(
    discovery: SourceDiscovery,
    { root, worktree }: { readonly root: string; readonly worktree: string },
    findings: WorktreeFindings,
    kept: readonly string[],
    taken: { readonly previous: Map<string, ObservedSource>; readonly scan: SourceScan },
  ): void {
    const next = findings.snapshot;
    const takes = takenFrom(root, discovery.root);
    const paths = discovery.worktrees.fresh.has(worktree)
      ? this.#firstSightPaths(discovery, next)
      : changedPaths(this.#worktreeView.snapshot(worktree), next);
    for (const path of new Set([...paths.filter(takes).map(path => inWorktree(worktree, path)), ...kept])) {
      const before = this.#observedSource(path);
      if (before !== undefined) taken.previous.set(path, before);
      const after = takenSource(worktree, findings, path);
      if (after !== undefined) taken.scan.sources.set(path, after);
    }
    takeGaps(worktree, findings, taken.scan, takes);
  }

  /**
   * The sources of a worktree new to the session that its first walk may say something of: those stamped since the
   * session opened, which may be its writes, and the `.gitignore` files, whose rules it remembers. Every other is a
   * baseline the snapshot holds as it is; a source Git lists as changed is read for its text (`#readTakenTexts`).
   */
  #firstSightPaths(discovery: SourceDiscovery, snapshot: WorktreeSnapshot): string[] {
    const paths: string[] = [];
    const ignoreFiles = this.#onIgnoreRulesChanged !== undefined;
    for (const [path, source] of snapshot.sources) {
      if ((source.modifiedAt ?? -Infinity) >= discovery.startedAt || (ignoreFiles && basename(path) === GITIGNORE)) paths.push(path);
    }
    return paths;
  }

  /**
   * Reads, off the lane, the text of each changed source the walk kept none of for this session (its snapshot was older
   * than the walk's last, or a tool call saw the source otherwise), as the session reads it now (`#reread`), and the
   * first-sight text of each source new to the session that its checkout's Git lists as changed, taken only while the
   * source holds the digest observed. A source the session saw that the walk's snapshot lacks is gone only if it is gone
   * now: a walk that did not read its worktree (the notice of the write came after the walk took the notices) hands on
   * a snapshot from before the session's tool call made it. Returns the sources it leaves to the next walk, with what
   * it saw of each before (`WorktreeTake.views`).
   */
  async #readTakenTexts(
    discovery: SourceDiscovery,
    walk: WalkStart,
    scan: SourceScan,
    found: ReadonlyMap<string, WorktreeFindings>,
  ): Promise<Map<string, ObservedSource | undefined>> {
    const kept = new Map<string, ObservedSource | undefined>();
    if (!await this.#keepSeenStill(discovery, walk, scan, kept) || !await this.#rereadMissingTexts(discovery, walk, scan, kept)) return kept;
    const listed = await this.#firstSeenInWorktrees(discovery, walk, scan, found);
    // A first sight Git lists puts the walk's observation back; the next walk shows these instead.
    for (const path of kept.keys()) scan.sources.delete(path);
    await this.#takeFirstSeenTexts(discovery, listed, scan);
    return kept;
  }

  /**
   * Puts into `kept` each source the session saw itself that the walk's snapshot lacks (its last snapshot held it) and
   * a read finds still there, with what the session saw of it. Returns false once the discovery is abandoned.
   */
  async #keepSeenStill(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan, kept: Map<string, ObservedSource | undefined>): Promise<boolean> {
    for (const [path, before] of walk.previous) {
      // Only what the session saw itself: one its last snapshot held and this one lacks, a later read found gone.
      if (scan.sources.has(path) || !this.#observed.has(path)) continue;
      if (this.#abandoned(discovery)) return false;
      if (await this.#stillThere(discovery.root, path)) kept.set(path, before);
    }
    return true;
  }

  /** Whether a read of `path` finds a file there; a path the session does not read finds none. */
  async #stillThere(root: string, path: string): Promise<boolean> {
    const read = await this.#readSource(root, path);
    return read !== undefined && read.deleted !== true;
  }

  /**
   * Rereads each source of `scan` the walk kept no text of (`#textMissing`) as the session reads it now (`takeReread`).
   * Returns false once the discovery is abandoned.
   */
  async #rereadMissingTexts(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan, kept: Map<string, ObservedSource | undefined>): Promise<boolean> {
    for (const [path, after] of scan.sources) {
      if (this.#abandoned(discovery)) return false;
      if (this.#textMissing(discovery, walk, after)) takeReread(path, await this.#reread(discovery, after), walk, scan, kept);
    }
    return true;
  }

  /**
   * A changed source the walk kept no text of, as the session reads it now: with its text while it holds the digest the
   * walk observed. A file changed again since the walk read it is taken as the session reads it, as the session's own
   * walk would have read it, so the change is recorded now, from what the session saw last, with its text; `gone` once
   * the file is. Without the text, a later walk would record the next change from a digest the session holds no text
   * of. A file the session cannot read now (another process holds it) is `unread`: the next walk takes it, from what
   * the session saw last. A file the session does not read stays as the walk found it.
   */
  async #reread(discovery: SourceDiscovery, after: Source): Promise<Source | "gone" | "unread"> {
    const read = await this.#readSource(discovery.root, after.path);
    if (read === undefined) return after;
    if (read.deleted === true) return "gone";
    if (read.readError !== undefined) return "unread";
    if (read.digest === after.digest) return read.text === undefined ? after : { ...after, text: read.text };
    return stampedAsFound(read, after);
  }

  /** Whether the walk found `after` changed, or first saw it changed since the session opened, and kept no text of it. */
  #textMissing(discovery: SourceDiscovery, walk: WalkStart, after: Source): boolean {
    if (after.text !== undefined || after.digest === undefined || after.readError !== undefined) return false;
    const before = walk.previous.get(after.path);
    if (before?.digest === after.digest) return false;
    return !this.#baselinePath(discovery, after.path, before, false) || changedSinceStart(discovery, after);
  }

  /**
   * The sources the walk is the first to show the session, in a worktree new to it or under a subtree it could not read
   * before, that the Git of the checkout owning them lists as modified or untracked. The walk listed them once for every
   * session first taking the worktree (`WorktreeFindings.listed`); a worktree it did not list for this session is listed
   * here: one the session had not named as the walk began, or one an earlier walk it was still taking then showed it
   * first, where that take did not end its first sight. One the session had not taken is added to `scan.sources` from
   * the snapshot, so its text is checked against the digest observed. A checkout whose Git ran past its time limit lists
   * none: what the walk shows the session first there goes in `scan.lateListings`, for a listing asked again once the
   * walk is applied (`#listLateAgain`), as for the session folder's walk (`#firstSeenModifiedOrUntracked`).
   */
  async #firstSeenInWorktrees(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan, found: ReadonlyMap<string, WorktreeFindings>): Promise<string[]> {
    const listed: string[] = [];
    for (const [root, findings] of found) {
      const worktree = sessionPath(discovery.root, root);
      if (!walk.worktrees.has(worktree) || !this.#mayFirstSee(discovery, worktree)) continue;
      if (!await this.#firstSeenInCheckouts(discovery, walk, { root, worktree, findings, scan }, listed)) return listed;
    }
    return listed;
  }

  /**
   * Puts into `taken.scan` and `listed` what the walk is the first to show the session in each checkout of the worktree
   * at `taken.root` that its Git lists as changed; a checkout that lists none puts what it shows first in
   * `scan.lateListings` (`#lateFirstSeen`). Returns false once the discovery is abandoned.
   */
  async #firstSeenInCheckouts(discovery: SourceDiscovery, walk: WalkStart, taken: TakenWorktree, listed: string[]): Promise<boolean> {
    for (const checkout of taken.findings.checkouts) {
      if (this.#abandoned(discovery)) return false;
      const changed = await listedChanges(discovery, taken.findings, checkout);
      if (changed === undefined) {
        taken.scan.lateListings.set(checkout, this.#lateFirstSeen(discovery, walk, taken, checkout));
        continue;
      }
      const firstSeen = this.#firstSeenOf(discovery, walk, taken, changed.map(path => sessionPath(discovery.root, resolve(checkout, path))));
      for (const [path, source] of firstSeen) taken.scan.sources.set(path, source);
      listed.push(...firstSeen.keys());
    }
    return true;
  }

  /** What the walk is the first to show the session of the sources `checkout` owns in the worktree, for a checkout whose Git listed none. */
  #lateFirstSeen(discovery: SourceDiscovery, walk: WalkStart, taken: TakenWorktree, checkout: string): Map<string, Source> {
    const owned = [...taken.findings.snapshot.sources.keys()].filter(takenFrom(taken.root, discovery.root)).map(path => inWorktree(taken.worktree, path));
    return this.#firstSeenOf(discovery, walk, taken, owned.filter(path => this.#owningCheckout(discovery.root, path) === checkout));
  }

  /** Of `paths` (session-relative), those in `worktree` that the walk is the first to show the session, as it found them. */
  #firstSeenOf(
    discovery: SourceDiscovery,
    walk: WalkStart,
    { worktree, findings, scan }: { worktree: string; findings: WorktreeFindings; scan: SourceScan },
    paths: readonly string[],
  ): Map<string, Source> {
    const firstSeen = new Map<string, Source>();
    for (const path of paths) {
      if (rootHolding(walk.worktrees, path) !== worktree || !this.#firstSeen(discovery, walk, path)) continue;
      const source = scan.sources.get(path) ?? takenSource(worktree, findings, path);
      if (source !== undefined) firstSeen.set(path, source);
    }
    return firstSeen;
  }

  /** Whether a walk may show the session a source of `worktree` for the first time: it is new to the session, or had a subtree unread. */
  #mayFirstSee(discovery: SourceDiscovery, worktree: string): boolean {
    if (discovery.worktrees.fresh.has(worktree)) return true;
    return [...discovery.worktrees.baselineRoots].some(failed => failed === worktree || failed.startsWith(`${worktree}/`));
  }

  /**
   * Moves the session's view of each worktree the walk read to the walk's snapshot. A source a tool call of the session
   * touched while the walk ran is not the walk's to answer for: it keeps what the session saw of it before. So does one
   * the take leaves to the next walk (`WorktreeTake.views`).
   */
  #takeSnapshots(discovery: SourceDiscovery, take: WorktreeTake): void {
    for (const [worktree, snapshot] of take.snapshots) if (discovery.worktrees.roots.has(worktree)) this.#worktreeView.take(worktree, snapshot);
    const toolChanged = this.#toolChanges(discovery, take.walk);
    for (const path of new Set([...take.walk.previous.keys(), ...take.scan.sources.keys()])) {
      if (toolChanged(path, take.scan.sources.get(path))) this.#keepToolView(path, take.walk.previous.get(path));
    }
    for (const [path, before] of take.views) this.#keepToolView(path, before);
  }

  /** Keeps `before`, what the session saw of `path` before this walk, unless it holds a view of its own of it. */
  #keepToolView(path: string, before: ObservedSource | undefined): void {
    if (this.#observed.has(path) || this.#worktreeView.isHidden(path)) return;
    if (before === undefined) this.#unobserve(path);
    else this.#observe(before);
  }

  /** Audits what a finished scan found, then makes it the walk's new baseline. */
  async #applyWalk(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): Promise<void> {
    if (this.#abandoned(discovery)) return;
    // A worktree removed while its own walk ran took its files with it: the files the walk then missed are no deletions.
    if (!walk.inSession) this.#syncWorktrees(discovery);
    const uncaptured = new Map<string, string>();
    const edits = this.#discoverEdits(discovery, walk, scan.sources, uncaptured);
    await this.#noteWalkedIgnoreFiles(discovery, walk, scan.sources);
    await this.#auditWalkEdits(discovery, walk, edits, uncaptured);
    this.#keepFirstSeenTexts(scan.firstSeen);
    for (const [root, head] of walk.heads) discovery.heads.set(root, head);
    for (const [root, entries] of walk.unmerged) discovery.unmerged.set(root, entries);
    // Kept after the audit: a checkout this walk was the first to pass is judged as one no walk has read (`stoppedSince`).
    for (const [root, entries] of scan.unmerged) if (!discovery.unmerged.has(root)) discovery.unmerged.set(root, entries);
    if (walk.inSession) this.#finishSessionWalk(discovery, walk, scan);
    else this.#finishWorktreeWalk(discovery, walk, scan);
    this.#observations?.set(this.#cwd, new Map(this.#observed));
  }

  /**
   * Audits what a walk found. The other worktrees' walk cannot tell which session wrote a file there, so its findings are
   * recorded under the repository in `log` mode and steer no session; the session that made a change is steered on it
   * by its own tool call or its own folder's walk.
   */
  async #auditWalkEdits(discovery: SourceDiscovery, walk: WalkStart, edits: readonly CodeRuleEdit[], uncaptured: Map<string, string>): Promise<void> {
    const { own, notOwn } = await this.#separateOwnWrites(discovery, walk, edits, uncaptured);
    const checkpoints = discoveryCheckpoints(strongerCheckpoint(walk.checkpoint, discovery.requested));
    try {
      if (notOwn.length > 0) await this.#auditEdits(discovery, notOwn, checkpoints, new Map(), "log");
      if (own.length > 0) await this.#auditEdits(discovery, own, checkpoints, uncaptured, walk.inSession ? undefined : "log");
    } catch (error) {
      // A rejected evaluator submission is not an observation we have enforced.
      // Restore these baselines so the next requested scan can retry the changes.
      this.#restoreObservations(discovery, walk, edits);
      throw error;
    }
  }

  /** Puts back what `walk` had observed of `edits`, but in a worktree removed since, which took its files with it (`#forgetWorktrees`). */
  #restoreObservations(discovery: SourceDiscovery, walk: WalkStart, edits: readonly CodeRuleEdit[]): void {
    const gone = new Set([...walk.worktrees].filter(worktree => !discovery.worktrees.roots.has(worktree)));
    for (const edit of edits.filter(edit => !underBaselineRoot(gone, edit.path))) {
      const before = walk.previous.get(edit.path);
      if (before === undefined) this.#unobserve(edit.path);
      else this.#observe(before);
    }
  }

  #finishSessionWalk(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): void {
    discovery.walkedFrom = walk.observedAt;
    discovery.baselineComplete = true;
    discovery.baselineRoots = scan.failedRoots;
    discovery.gaps = scanGaps(scan.errors);
    discovery.unread = scan.unread;
    discovery.passedCheckouts = scan.checkouts;
    // A subtree or a file it could not read may still hold writes of the calls that ended before it: the next walk that
    // reads them judges each by its stamp. A walk that gave up on a path (a folder it could not list, or a path gone as
    // it looked, perhaps moved) keeps every call; a file keeps those that may have written it unseen
    // (`holdMissedFiles`), for its own writes only.
    if (holdMissedFiles(discovery, walk, scan)) discovery.toolCalls.answered(walk.callsEnded, [...discovery.missedFiles.values()]);
    this.#listLateAgain(discovery, scan.lateListings);
  }

  #finishWorktreeWalk(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): void {
    const worktrees = discovery.worktrees;
    worktrees.walkedFrom = walk.observedAt;
    for (const worktree of walk.worktrees) worktrees.fresh.delete(worktree);
    worktrees.baselineRoots = scan.failedRoots;
    // `git worktree remove` takes the worktree's folder before Git stops listing it, so a walk that began with it listed
    // can find the folder gone, or its delete pending, which fails the walk there. A worktree gone since the walk began
    // (`#applyWalk`) took its folders and files with it: nothing the walk could not read there is a place.
    const gone = new Set([...walk.worktrees].filter(worktree => !worktrees.roots.has(worktree)));
    const stays = (path: string): boolean => !gone.has(path) && !underBaselineRoot(gone, path);
    worktrees.gaps = scanGaps(scan.errors.filter(error => stays(error.path)));
    worktrees.unread = new Map([...scan.unread].filter(([path]) => stays(path)));
    this.#listLateAgain(discovery, scan.lateListings);
  }

  /**
   * Asks Git once more which files changed in each of `late`'s checkouts, whose listing ran past its time limit for the
   * walk just applied, off every walk's path: no walk or boundary waits for it, so what that walk and the next find is
   * checked at the boundary each reaches. Each checkout's listing asks Git once it holds one of `LISTINGS_AGAIN_AT_ONCE`
   * slots, in the order asked, and hands its slot on once Git has answered or has run `LISTING_AGAIN_WAIT_MS`: the load
   * that made them late meets no more than that many more Gits at once from this session while each answers in time,
   * and a slow one keeps the rest waiting no longer than that. Each checkout's text goes once into the next boundary's
   * `checkpoint` answer (`newlyLate`), which the daemon does not read. `late` holds the sources the walk saw first in
   * each, whose first-sight text it could not keep; a listing asked again that has not answered yet, running or waiting
   * its turn, takes them on.
   */
  #listLateAgain(discovery: SourceDiscovery, late: SourceScan["lateListings"]): void {
    for (const [checkout, seen] of late) {
      discovery.late.add(lateListing(discovery, checkout, LATE_LISTING));
      const asking = discovery.askingAgain.get(checkout);
      if (asking === undefined) this.#startListingAgain(discovery, checkout, new Map(seen));
      else for (const [path, source] of seen) asking.set(path, source);
    }
  }

  #startListingAgain(discovery: SourceDiscovery, checkout: string, seen: Map<string, Source>): void {
    discovery.askingAgain.set(checkout, seen);
    const done = this.#gitListingInSlot(discovery, checkout)
      .then(answer => this.#takeListedAgain(discovery, checkout, seen, answer))
      .catch((error: unknown) => {
        process.stderr.write(`rerail: cannot read the files ${checkout} listed again: ${errorMessage(error)}\n`);
      })
      .finally(() => discovery.listingsAgain.delete(done));
    discovery.listingsAgain.add(done);
  }

  /**
   * Lists `checkout` again once it holds a slot to ask Git. Only Git holds the slot: it goes to the next listing once Git
   * has answered or has run `LISTING_AGAIN_WAIT_MS`, so reading and keeping the files it lists run beside the next.
   * A listing whose session closed before it got its slot hands the slot on at once and asks Git nothing, answering as
   * a read the session's close stopped does: the close waits for the bridge's queue before it stops the session's Git
   * reads, and an audit can hold that queue for seconds.
   */
  async #gitListingInSlot(discovery: SourceDiscovery, checkout: string): Promise<ChangedPathsListing> {
    await slotTaken(discovery.listingSlots);
    if (this.#abandoned(discovery)) {
      slotHandedOn(discovery.listingSlots);
      return { ended: "timed-out" };
    }
    const listing = this.#gitListingAgain(discovery, checkout);
    void settledOrAfter(listing, LISTING_AGAIN_WAIT_MS).then(() => { slotHandedOn(discovery.listingSlots); });
    return await listing;
  }

  /** Lists `checkout`'s changed files once more, through the session's Git reads, which its close stops; a throw is a failed listing. */
  async #gitListingAgain(discovery: SourceDiscovery, checkout: string): Promise<ChangedPathsListing> {
    return await discovery.git.run(signal => gitModifiedOrUntrackedListing(checkout, { signal }))
      .catch((error: unknown): ChangedPathsListing => {
        process.stderr.write(`rerail: cannot list the changed files of ${checkout} again: ${errorMessage(error)}\n`);
        return { ended: "failed" };
      });
  }

  /**
   * Keeps the first-sight text of each of `seen` that `listing`, of `checkout` asked again, lists and that is still as
   * the late walk saw it and as the session last observed it (`#keepFirstSeenTexts`). A second late listing is the last,
   * as is one that fails or throws: its text goes once into the next `checkpoint` answer, which the daemon does not
   * read, and the checkout's files changed before the session keep no first-sight text. Either way the checkout is no
   * longer waited for, so its next late listing is asked again.
   */
  async #takeListedAgain(discovery: SourceDiscovery, checkout: string, seen: ReadonlyMap<string, Source>, listing: ChangedPathsListing): Promise<void> {
    discovery.askingAgain.delete(checkout);
    if (this.#abandoned(discovery)) return;
    if (listing.ended !== "printed") {
      discovery.late.add(lateListing(discovery, checkout, listing.ended === "failed" ? FAILED_AGAIN : LATE_AGAIN));
      return;
    }
    const listed = listing.paths.map(path => sessionPath(discovery.root, resolve(checkout, path))).filter(path => seen.has(path));
    const taking: FirstSightTake = { sources: seen, firstSeen: new Map() };
    await this.#takeFirstSeenTexts(discovery, listed, taking);
    await this.#enqueue(async () => {
      if (!this.#abandoned(discovery)) this.#keepFirstSeenTexts(taking.firstSeen);
    });
  }

  /** Waits for each listing asked again off the walks' path (`#listLateAgain`), and any asked for meanwhile. */
  async settleListingsAgain(): Promise<void> {
    const discovery = this.#discovery;
    while (discovery !== undefined && discovery.listingsAgain.size > 0) await Promise.all(discovery.listingsAgain);
  }

  /** The changes the walk found; `uncaptured` receives the digest each changed source was observed with before. */
  #discoverEdits(discovery: SourceDiscovery, walk: WalkStart, current: ReadonlyMap<string, Source>, uncaptured: Map<string, string>): CodeRuleEdit[] {
    const toolChanged = this.#toolChanges(discovery, walk);
    const edits: CodeRuleEdit[] = [];
    for (const path of [...new Set([...walk.previous.keys(), ...current.keys()])].sort(compareDeterministicStrings)) {
      if (!walkAnswersFor(discovery, walk, path) || toolChanged(path, current.get(path))) continue;
      const previous = walk.previous.get(path);
      const edit = this.#discoveredEdit(discovery, path, previous, current.get(path), walk.baseline);
      if (edit === undefined) continue;
      edits.push(edit);
      // A source held before a suspension was observed before this walk, though the walk is a baseline (`#baselinePath`).
      const before = uncapturedDigest(edit, previous, this.#baselinePath(discovery, path, previous, walk.baseline));
      if (before !== undefined) uncaptured.set(path, before);
    }
    return edits;
  }

  /**
   * Whether `walk` leaves the change it found to a source, `current` as it found it, to a tool call's result. A
   * begin/finish after scan launch wins even when its bytes happen to match the old snapshot. So does a shell command
   * still running as the walk began, even one that ended since, if it had begun by the time the source last changed (at
   * any time, for a source with no stamp: one gone, or one the walk failed to stat or read): it may have been part way
   * through writing the source as the walk read it. A walk that took it would audit a half-written file as the
   * session's edit, and the command's result would measure the rest of its change from that. That holds only of a
   * source the session has observed; the walk at the command's result, or the next one, takes the change whole, from
   * what the session saw before.
   */
  #toolChanges(discovery: SourceDiscovery, walk: WalkStart): (path: string, current: Source | undefined) => boolean {
    const pendingPaths = this.#pendingPaths();
    const commandSince = discovery.toolCalls.commandSince(walk.callsEnded);
    const commandWriting = (current: Source | undefined) =>
      commandSince !== undefined && (current?.modifiedAt === undefined || commandSince <= current.modifiedAt);
    return (path, current) => discovery.versions.get(path) !== walk.versions.get(path) || pendingPaths.has(path)
      || (walk.previous.has(path) && commandWriting(current));
  }

  /** The paths a tool call in progress has captured: its finish, not a walk, reports them. */
  #pendingPaths(): Set<string> {
    const paths = new Set<string>();
    for (const pending of this.#pending.values())
      for (const path of pending.preimages.keys()) paths.add(path);
    return paths;
  }

  /**
   * `HEAD` and the unmerged entries of the session folder's checkout and of every other checkout a walk has found a
   * change in, but for the other worktrees and the checkouts inside them (`inWorktrees`), whose shared walk reads theirs
   * (`WorktreeWalks`, `InnerCheckouts`). No other walk reads a checkout: a move or a conflict read before the walk that
   * does not find its files would be the last walk's by the time the one that does looks.
   * Each is read at the checkout that owns its files, as `#gitVerdicts` asks for it: a session opened in a subfolder
   * reads the checkout around it, whose unmerged paths Git lists from its top folder.
   * Read on every pass, not only one that finds changes: a modify/delete conflict can stop a merge without changing a
   * file, and the next pass must still know that conflict was there before the edit it finds. So a checkout inside the
   * session folder that no walk has found a change in has its entries read too, and not its `HEAD`: the entries cost a
   * `stat` while its index file is as it was, where `HEAD` costs a Git process. A checkout whose entries Git could not
   * list is left out of `unmerged`, so the last entries read stay its snapshot.
   */
  async #readCheckouts(discovery: SourceDiscovery): Promise<Pick<WalkStart, "heads" | "unmerged">> {
    const unmerged = new Map<string, ReadonlyMap<string, string>>();
    const others = [...discovery.heads.keys()].filter(known => !inWorktrees(discovery, known));
    // When no checkout owns the session folder, no change in it is asked about.
    const session = this.#owningCheckout(discovery.root, "");
    const withHead = new Set(session === undefined ? others : [session, ...others]);
    const heads = await readHeads(discovery, withHead, unmerged);
    for (const root of discovery.passedCheckouts) if (!withHead.has(root) && !inWorktrees(discovery, root)) await readEntriesInto(discovery, root, unmerged);
    return { heads, unmerged };
  }

  /**
   * Puts in `scan.unmerged` the unmerged entries of each checkout the session folder's walk passed that no walk has read:
   * one inside the session folder, which nothing named before the walk. Once the walk is applied they are its snapshot,
   * so a conflict that stops Git there afterwards is new to the walk that follows it.
   */
  async #readPassedCheckouts(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): Promise<void> {
    if (!walk.inSession) return;
    for (const checkout of scan.checkouts) {
      if (discovery.cancelled) return;
      if (walk.unmerged.has(checkout) || discovery.unmerged.has(checkout)) continue;
      const entries = await unmergedNow(discovery, checkout);
      if (entries !== undefined) scan.unmerged.set(checkout, entries);
    }
  }

  /**
   * Splits a walk's changes into the session's own and those it did not write: Git's (`#gitVerdicts`), whoever ran the
   * Git command, and, in the session folder, any other writer's (`#claimOwnWrites`). Neither kind is reported as this
   * session's edit. Of those, only a source this session had changed is still audited, as the source it now is with no
   * generated lines and in `log` mode: the evaluator holds its older text, and steers queued for that text are out of
   * date. Any other is the same to the evaluator as the files it never saw. The walk of the other worktrees drops every
   * change of a checkout in a worktree it may show the session first (`#mayFirstSee`), or in one it knew where it finds
   * a file gone, whose `.git` entry is gone once its Git has been read (`#checkoutStands`).
   */
  async #separateOwnWrites(
    discovery: SourceDiscovery,
    walk: WalkStart,
    edits: readonly CodeRuleEdit[],
    uncaptured: Map<string, string>,
  ): Promise<{ own: CodeRuleEdit[]; notOwn: CodeRuleEdit[] }> {
    const sorted: SortedWrites = { unwritten: [], notOwn: [], uncaptured };
    const { owned, unowned } = this.#byOwningCheckout(discovery.root, edits, edit => edit.path);
    sorted.unwritten.push(...unowned);
    for (const [checkout, group] of owned) {
      const verdicts = await this.#gitVerdicts(discovery, checkout, group, uncaptured);
      if (await this.#checkoutStands(discovery, walk, checkout, group)) this.#sortByVerdicts(group, verdicts, sorted);
    }
    const { unwritten, notOwn } = sorted;
    const own = walk.inSession ? await this.#claimOwnWrites(discovery, walk, unwritten, uncaptured, notOwn) : unwritten;
    return { own, notOwn };
  }

  /**
   * Whether `checkout` is still the checkout whose Git gave the walk its verdicts, given its changes `group`. In another
   * worktree only Git's verdicts tell Git's own files from writes: at the walk's first sight of it, every file stamped
   * since the session opened (`#firstSightPaths`); in one it knew, each file it saw there that is gone. `git worktree
   * remove` deletes the worktree's files, its `.git` entry among them, before Git stops listing the worktree, and a Git
   * started there once the entry is gone reads no checkout, or the one around the worktree, so each of those files
   * would be taken for a write. There the entry is looked for once Git has answered: still there, Git read this
   * checkout; gone, the worktree is being removed or was, its files went with it, as a worktree gone before the walk is
   * applied took its own (`#syncWorktrees`), and none of them is an edit. The removal deletes a folder's entries in the
   * order the file system lists them, so the entry may still be there at the look while the removal goes on: the files
   * it deleted before then are deletions Git's verdicts judge, as a person's are. A walk that finds only writes in a
   * worktree it knew, as it does on most walks, looks for nothing.
   */
  async #checkoutStands(discovery: SourceDiscovery, walk: WalkStart, checkout: string, group: readonly CodeRuleEdit[]): Promise<boolean> {
    if (walk.inSession) return true;
    const named = sessionPath(discovery.root, checkout);
    const worktree = walk.worktrees.has(named) ? named : rootHolding(walk.worktrees, named);
    if (worktree === undefined) return true;
    const mayBeRemoved = this.#mayFirstSee(discovery, worktree) || group.some(edit => edit.deleted === true);
    return !mayBeRemoved || await stillACheckout(checkout);
  }

  /**
   * Sorts one checkout's changes by Git's verdicts: one Git did not write stays to be claimed (`unwritten`), with the
   * text Git's move proves it was changed from when there is one; one Git wrote is audited only as `#sortUnownedWrite`
   * says.
   */
  #sortByVerdicts(group: readonly CodeRuleEdit[], verdicts: ReadonlyMap<string, GitVerdict>, sorted: SortedWrites): void {
    for (const edit of group) {
      const verdict = verdicts.get(edit.path);
      if (verdict === undefined) sorted.unwritten.push(edit);
      else if (verdict.previousText === undefined) this.#sortUnownedWrite(edit, verdict.keptAs, sorted.notOwn);
      else {
        sorted.uncaptured.delete(edit.path);
        sorted.unwritten.push({ ...withoutUncaptured(edit), previousText: verdict.previousText });
      }
    }
  }

  /**
   * Of the session folder's changes Git did not write, the session's own: those stamped while one of its tool calls ran
   * (`ToolCallWindows`), whatever the tool. The rest are another writer's (another session, an editor, a process a
   * command left running): their text is kept, so the session's next shell edit of one is measured from what that writer
   * left. The other worktrees' walk records what it finds in `log` mode anyway.
   */
  async #claimOwnWrites(
    discovery: SourceDiscovery,
    walk: WalkStart,
    edits: readonly CodeRuleEdit[],
    uncaptured: Map<string, string>,
    notOwn: CodeRuleEdit[],
  ): Promise<CodeRuleEdit[]> {
    const own: CodeRuleEdit[] = [];
    for (const edit of edits) {
      if (await this.#writtenByCall(discovery, walk, edit)) {
        own.push(edit);
        continue;
      }
      uncaptured.delete(edit.path);
      this.#sortUnownedWrite(edit, "other-writer", notOwn);
    }
    return own;
  }

  /**
   * Whether one of the session's tool calls ran as `edit`'s file was stamped or, for a file now gone, as its folder was
   * (removing an entry stamps the folder; a later change there moves the stamp on). With no stamp to go by, at some point
   * since the walk that last saw the file began. A file the last walk could not read counts the calls kept for it too.
   */
  async #writtenByCall(discovery: SourceDiscovery, walk: WalkStart, edit: CodeRuleEdit): Promise<boolean> {
    const stamp = this.#observedSource(edit.path)?.modifiedAt ?? (edit.deleted === true ? await folderStamp(discovery.root, edit.path) : undefined);
    const held = discovery.missedFiles.get(edit.path);
    if (stamp !== undefined) return discovery.toolCalls.ranDuring(stamp, stamp, held);
    return discovery.toolCalls.ranDuring(walk.previous.get(edit.path)?.observedAt ?? discovery.startedAt, walk.observedAt, held);
  }

  /**
   * A write the session did not make is audited only for a source the evaluator holds the session's text of: one it
   * edited, or one Git wrote over. Text another writer left, or a first sight, is no change of the session's, so the
   * evaluator never held it. With `keptAs` (another writer's, or Git's text neither the index nor `HEAD` holds) its text
   * is kept for the session's next edit of the source.
   */
  #sortUnownedWrite(edit: CodeRuleEdit, keptAs: "git" | "other-writer" | undefined, notOwn: CodeRuleEdit[]): void {
    if (this.#heldByEvaluator(edit.path))
      notOwn.push(edit.generatedText === undefined ? edit : { ...withoutUncaptured(edit), previousText: edit.generatedText });
    // The next shell edit measures from the text left, which neither the index nor `HEAD` may hold.
    else if (keptAs !== undefined && edit.generatedText !== undefined) this.#rememberText(edit.path, edit.generatedText, keptAs);
  }

  /**
   * Which of a checkout's changes Git wrote, and for the rest the text Git's move proves each was changed from.
   *
   * Git wrote a file that now holds exactly `HEAD`'s blob, unless only commits moved `HEAD` since the last walk: a commit
   * writes no file, so that file was written by something else and then committed, and stays the session's (measured
   * from the tree before the commit). `HEAD` not having moved, the file holding its blob adds nothing to what is
   * committed (`git restore`, `git stash`). A file a move left absent from `HEAD` was removed by it. A merge, rebase or
   * cherry-pick stopped on a conflict has not moved `HEAD` for its work so far: a file it just made unmerged is Git's, and
   * in the same walk so is one that holds its index blob or that the index no longer has. A later change to a file with
   * the same unmerged entries as the last walk is no longer Git's stop, even when the file held the blob of its side
   * before it. A file holds a blob in the CRLF line breaks as well where the repository's settings have a checkout write
   * them (`gitCrlfPaths`, `line-endings.ts`); other filters' bytes never match, and stay the session's.
   * Of the rest, a file restored from a branch is Git's too (`#restoredFromBranches`).
   *
   * The verdicts hold for one `HEAD`, and Git is read in several processes, `HEAD` first. A command that writes files
   * and commits them can commit in between: `HEAD` then reads as unmoved while the files hold the blob of its new
   * commit, which `HEAD` and the branch have by the next read, so they would read as Git's and none would be checked. So
   * a reading that finds a change Git's stands only once `HEAD` still names what it read, and Git is read again while it
   * does not (`#settledReading`). When `HEAD` moves through every reading, the first one stands, as one reading would.
   */
  async #gitVerdicts(
    discovery: SourceDiscovery,
    checkout: string,
    edits: readonly CodeRuleEdit[],
    uncaptured: ReadonlyMap<string, string>,
  ): Promise<Map<string, GitVerdict>> {
    const named = edits.map(edit => ({ edit, path: sessionPath(checkout, resolve(discovery.root, edit.path)), prior: uncaptured.get(edit.path) }));
    const first = await this.#readGitVerdicts(discovery, checkout, named);
    if (first === undefined) return new Map();
    const standing = await this.#settledReading(discovery, checkout, named, first) ?? first;
    keepFirstSnapshot(discovery, checkout, standing.view);
    return standing.verdicts;
  }

  /**
   * The first reading of the verdicts, from `first` on, that holds for one `HEAD`: one that finds no change Git's, or
   * whose `HEAD` has not moved since it was read. Undefined when none does in `GIT_VERDICT_READS`, or Git cannot say, and
   * once the session closes: the walk's step holds the queue that the close waits on, so a reading made then would hold
   * up the close, and the first reading stands, as one reading would.
   */
  async #settledReading(
    discovery: SourceDiscovery,
    checkout: string,
    named: readonly CheckoutEdit[],
    first: VerdictReading,
  ): Promise<VerdictReading | undefined> {
    let reading: VerdictReading | undefined = first;
    for (let reads = 1; reading !== undefined; reads += 1) {
      if (!findsGitWrite(reading.verdicts) || await headStill(discovery, checkout, reading.view.head)) return reading;
      if (reads === GIT_VERDICT_READS || discovery.cancelled) return undefined;
      reading = await this.#readGitVerdicts(discovery, checkout, named);
    }
    return undefined;
  }

  /** One reading of `#gitVerdicts`, from Git as it is now; undefined where Git cannot say. */
  async #readGitVerdicts(discovery: SourceDiscovery, checkout: string, named: readonly CheckoutEdit[]): Promise<VerdictReading | undefined> {
    const read = await readGitView(discovery, checkout, named.map(({ path }) => path));
    if (read === undefined) return undefined;
    const verdicts = new Map<string, GitVerdict>();
    const view: GitView = { ...read, stopped: stoppedSince(read, named) };
    for (const { edit, path, prior } of named) {
      const verdict = edit.readError === undefined ? this.#gitVerdict(view, edit, path, prior) : undefined;
      if (verdict !== undefined) verdicts.set(edit.path, verdict);
    }
    await this.#addRestoredVerdicts(discovery.git, checkout, view, named, verdicts);
    return { verdicts, view: read };
  }

  /** Of the changes Git has not explained, those restored from a branch are Git's, their text kept for the next edit. */
  async #addRestoredVerdicts(
    git: GitReads,
    checkout: string,
    view: GitBlobs,
    named: ReadonlyArray<{ edit: CodeRuleEdit; path: string }>,
    verdicts: Map<string, GitVerdict>,
  ): Promise<void> {
    const unexplained = named.filter(({ edit }) => !verdicts.has(edit.path) && edit.readError === undefined && edit.generatedText !== undefined);
    if (unexplained.length === 0) return;
    for (const path of await this.#restoredFromBranches(git, checkout, view, unexplained)) verdicts.set(path, { keptAs: "git" });
  }

  #gitVerdict(view: GitView, edit: CodeRuleEdit, path: string, prior: string | undefined): GitVerdict | undefined {
    if (conflictedByGit(view, path, prior)) return { keptAs: "git" };
    if (this.#heldByGit(view, edit, path)) return {};
    return movedFromText(view, path, prior);
  }

  #heldByGit(view: GitView, edit: CodeRuleEdit, path: string): boolean {
    if (edit.deleted === true) return removedByGit(view, path);
    const after = this.#observedSource(edit.path)?.digest;
    return after !== undefined && holdsGitBlob(view, path, after);
  }

  /**
   * The changes that now hold exactly the blob a branch or remote-tracking branch has at their path: restored from it
   * (`git show origin/main:f > f`, `git checkout main -- f`, `git restore --source main f`), not written. A branch whose
   * tip is one of the commits made since the last walk (`commitsMade`) is left out, so a file written, committed and pushed
   * in one command stays an edit; when Git cannot list them, nothing is restored. Text that is not valid UTF-8 never matches, and
   * stays the session's.
   *
   * Each file's blob id is computed here, and only text the repository already holds as an object is looked for in the
   * branches: new text, which most edits write, costs one `cat-file --batch-check` and no branch is read. Text with CRLF
   * line breaks is also looked for as the LF text a checkout wrote it from, where the repository's settings have a
   * checkout write CRLF.
   */
  async #restoredFromBranches(
    git: GitReads,
    checkout: string,
    view: GitBlobs,
    named: ReadonlyArray<{ edit: CodeRuleEdit; path: string }>,
  ): Promise<string[]> {
    // Git names a blob by the SHA-1, or in a SHA-256 repository the SHA-256, of its header and bytes; `HEAD`'s id says which.
    const algorithm = view.head.length === 64 ? "sha256" : "sha1";
    const ids = new Map(named.map(({ edit, path }) => [edit.path, blobIds(edit.generatedText!, algorithm, view.crlf.get(path))]));
    const known = await git.run(signal => readGitBlobIds(checkout, [...new Set([...ids.values()].flat())], { signal }));
    const candidates = named.filter(({ edit }) => ids.get(edit.path)!.some(id => known.has(id)));
    if (candidates.length === 0) return [];
    const excluded = await commitsMade(git, checkout, view);
    if (excluded === undefined) return [];
    const tips = (await git.run(signal => gitBranchTips(checkout, { signal }))).filter(tip => !excluded.includes(tip));
    if (tips.length === 0) return [];
    const branchRevisions = candidates.flatMap(({ path }) => tips.map(tip => `${tip}:./${path}`));
    const held = await git.run(signal => readGitBlobIds(checkout, branchRevisions, { signal }));
    return candidates
      .filter(({ edit, path }) => tips.some(tip => ids.get(edit.path)!.includes(held.get(`${tip}:./${path}`) ?? "")))
      .map(({ edit }) => edit.path);
  }

  #discoveredEdit(
    discovery: SourceDiscovery,
    path: string,
    before: ObservedSource | undefined,
    after: Source | undefined,
    baseline: boolean,
  ): CodeRuleEdit | undefined {
    const baselinePath = this.#baselinePath(discovery, path, before, baseline);
    // A first observation made after a shell write is not a trustworthy preimage.
    // Preserve it as an uncaptured change, never as newly generated source.
    if ((baselinePath && !changedSinceStart(discovery, after)) || sameSource(before, after)) {
      if (after !== undefined) this.#observe(after);
      return undefined;
    }
    if (after === undefined) return this.#deletedEdit(path, before);
    this.#observe(after);
    return uncapturedEdit(path, after, uncapturedPreimage(baselinePath, before));
  }

  /**
   * Keeps `source` as what the session last observed of its path, keyed by the path string the observation holds: an
   * unchanged source's observation is the one every session sharing baseline observations holds, and so is its path. In
   * another worktree it is kept only where it differs from the snapshot the session took, so the session holds nothing
   * of its own for the files no tool call of it saw.
   */
  #observe(source: Source): void {
    const observed = observation(source);
    this.#worktreeView.unhide(observed.path);
    if (this.#worktreeView.holds(observed.path, observed)) this.#observed.delete(observed.path);
    else this.#observed.set(observed.path, observed);
  }

  /**
   * The walk is a baseline, but for a source the evaluator held before a suspension (`#resumedHeld`), or it first sees
   * `path` under a subtree a walk could not read or in a worktree whose first walk is still to finish.
   */
  #baselinePath(discovery: SourceDiscovery, path: string, before: ObservedSource | undefined, baseline: boolean): boolean {
    // A source held before a suspension is compared with what was last observed of it, as a walk of the open session would.
    if (baseline) return !this.#resumedHeld.has(path);
    return before === undefined
      && [discovery.baselineRoots, discovery.worktrees.baselineRoots, discovery.worktrees.fresh].some(roots => underBaselineRoot(roots, path));
  }

  #deletedEdit(path: string, before: ObservedSource | undefined): CodeRuleEdit | undefined {
    this.#unobserve(path);
    if (before === undefined) return undefined;
    return { path, language: before.language, previousReadError: "Source was deleted without a captured preimage", deleted: true };
  }

  async #observeSources(walk: SourceWalk): Promise<SourceScan> {
    const { scan } = walk;
    await this.#observeStack(walk);
    const judged = await judgeRefusals(walk);
    for (const source of scan.sources.values()) recordReadError(scan, source);
    settleRefusals(scan, judged);
    return scan;
  }

  async #observeStack(walk: SourceWalk): Promise<void> {
    const { discovery, stack } = walk;
    while (!discovery.cancelled && stack.length > 0) {
      const wave = stack.splice(-OBSERVATION_CONCURRENCY);
      await Promise.all(wave.map(absolute => this.#observePath(walk, absolute)));
    }
  }

  async #observePath(walk: SourceWalk, absolute: string): Promise<void> {
    let info: Stats | undefined;
    try {
      info = await lstat(absolute);
      await this.#observeEntry(walk, absolute, info);
    } catch (error) {
      if (walk.discovery.cancelled) return;
      if (goneWhenExamined(walk, absolute, error)) {
        gaveUpOn(walk.scan);
        return;
      }
      this.#recordObservationFailure(walk, absolute, error, info?.isFile() === true);
      noteRefusal(walk, absolute, error, info);
    }
  }

  async #observeEntry(walk: SourceWalk, absolute: string, info: Stats): Promise<void> {
    if (walk.discovery.cancelled || info.isSymbolicLink()) return;
    if (info.isDirectory()) await this.#expandDirectory(walk, absolute);
    else if (info.isFile()) await this.#observeRegularFile(walk, absolute, info);
  }

  /**
   * Folders Git ignores (when the session follows `.gitignore`) are never entered, so nothing below them is observed.
   * A folder's `.git` is not entered either, and marks the checkout that owns the folder's files (`SourceScan.checkouts`).
   */
  async #expandDirectory(walk: SourceWalk, absolute: string): Promise<void> {
    const listing = await opendir(absolute);
    const seen = walk.scan.seen;
    seen?.folders.add(sessionPath(walk.discovery.root, absolute));
    for await (const entry of listing) {
      if (walk.discovery.cancelled) return;
      if (entry.name === ".git") this.#noteCheckout(walk, join(absolute, entry.name));
      else if (this.#walksInto(walk, absolute, entry)) walk.stack.push(join(absolute, entry.name));
      else seen?.leftOut.add(sessionPath(walk.discovery.root, join(absolute, entry.name)));
    }
  }

  /**
   * Asks for the owner of the `.git` entry, not of its folder: it is owned as the files beside it are. One removed as the
   * walk asks (`vanished`) marks no checkout, and never fails the folder that listed it. The walk never enters the entry,
   * and each other entry the listing named is examined, so a folder moved as the walk lists it is found there
   * (`goneWhenExamined`).
   */
  #noteCheckout(walk: SourceWalk, gitEntry: string): void {
    let checkout: string | undefined;
    try {
      checkout = walk.checkoutOf(sessionPath(walk.discovery.root, gitEntry));
    } catch (error) {
      if (vanished(error)) return;
      throw error;
    }
    if (checkout !== undefined) walk.scan.checkouts.add(checkout);
  }

  /** A folder another walk starts at is not walked twice. */
  #walksInto(walk: SourceWalk, absolute: string, entry: Dirent): boolean {
    if (neverWalked(entry)) return false;
    if (!entry.isDirectory()) return entry.isFile();
    const child = join(absolute, entry.name);
    return !walk.roots.has(child) && !ignoredFolder(walk, child);
  }

  /**
   * A file Git ignores is not observed at all: no baseline entry, so no later read or edit of it is audited. A
   * `.gitignore` is observed by its stamp whatever Git says of it, since it decides what is ignored.
   */
  async #observeRegularFile(walk: SourceWalk, absolute: string, info: Stats): Promise<void> {
    const path = sessionPath(walk.discovery.root, absolute);
    if (basename(path) === GITIGNORE || !walk.view.ignoresPath?.(path, false)) await this.#observeFile(walk, sourceLocation(absolute, path), info);
    else walk.scan.seen?.leftOut.add(path);
  }

  async #observeFile(walk: SourceWalk, location: SourceLocation, info: Stats): Promise<void> {
    const observed = await this.#observedFile(walk, location, info);
    if (observed !== undefined) walk.scan.sources.set(location.path, observed);
  }

  /**
   * What the walk observes of a file; undefined once the walk is cancelled. A file the auditor does not read keeps the
   * digest it had, and one whose stamp holds keeps its observation. An unlisted binary file is observed without a read,
   * nor a read error, so it is not read again while its stamp holds.
   */
  async #observedFile(walk: SourceWalk, location: SourceLocation, info: Stats): Promise<Source | undefined> {
    const stamped = { path: location.path, language: location.language, stamp: fileStamp(info), observedAt: walk.observedAt };
    const previous = walk.previousSources.get(location.path);

    const modifiedAt = Math.max(info.mtimeMs, info.ctimeMs);
    if (staleStamp(previous, stamped.stamp, modifiedAt)) return previous;
    if (!walk.view.acceptsPath(location.path)) return { ...stamped, ...knownDigest(previous) };
    const unread = await unreadFile(location, info);
    if (unread !== undefined) return { ...stamped, modifiedAt, ...unread };
    return await walk.buffers.lend(
      location.absolute, info.size, async read => await this.#readObservedFile(walk, location, { ...stamped, modifiedAt }, previous, read),
    );
  }

  async #readObservedFile(
    walk: SourceWalk,
    location: SourceLocation,
    stamped: StampedSource,
    previous: ObservedSource | undefined,
    read: LentBytes,
  ): Promise<Source | undefined> {
    if (walk.discovery.cancelled) return undefined;
    const modifiedAt = await stampOfRead(walk, location.absolute, stamped.modifiedAt, at => undatedRead(previous, read.bytes, at));
    if (walk.discovery.cancelled) return undefined;
    // A baseline walk keeps what was seen of a file a command may be writing, and leaves the file to the walk after.
    if (modifiedAt === undefined) return previous;
    const content = nonText(location, read.bytes);
    return content === undefined ? observedText(walk, { ...stamped, modifiedAt }, previous, read) : { ...stamped, modifiedAt, ...content };
  }

  /** `file` when the path is known to be a file: an unlisted one fails as a source, not as a folder walked. */
  #recordObservationFailure(walk: SourceWalk, absolute: string, error: unknown, file: boolean): void {
    const path = sessionPath(walk.discovery.root, absolute);
    const language = sourceLanguageOfPath(path) ?? (file ? UNLISTED_TEXT_LANGUAGE : undefined);
    const readError = errorMessage(error);
    if (language === undefined) failFolder(walk, path, readError);
    else walk.scan.sources.set(path, { path, language, readError });
  }

  #readEdit(source: Source, preimage: Preimage | undefined): CodeRuleEdit | undefined {
    if (source.text !== undefined && unchangedSince(source, preimage)) {
      this.#observe(source);
      return undefined;
    }
    // Absent before the call and after it, as a path the input named that is no file: nothing was edited.
    if (absentThroughout(source, preimage)) return undefined;
    const edit = this.#editFromToolSource(source, preimage);
    this.#updateObservedSource(source);
    return edit;
  }

  #editFromToolSource(source: Source, preimage: Preimage | undefined): CodeRuleEdit | undefined {
    const edit = { path: source.path, language: source.language, ...previousOf(preimage) };
    if (source.text !== undefined) return { ...edit, generatedText: source.text };
    if (source.deleted === true) return { ...edit, deleted: true };
    return source.readError === undefined ? undefined : { ...edit, readError: source.readError };
  }

  #updateObservedSource(source: Source): void {
    if (source.readError !== undefined) return;
    if (source.deleted) this.#unobserve(source.path);
    else this.#observe(source);
  }

  /** What the session last observed of `path`: its own observation, else the snapshot of the worktree holding it. */
  #observedSource(path: string): ObservedSource | undefined {
    return this.#observed.get(path) ?? this.#worktreeView.get(path);
  }

  /** Forgets `path`, gone; in another worktree, it is marked gone where the snapshot the session took holds it. */
  #unobserve(path: string): void {
    this.#observed.delete(path);
    this.#worktreeView.hide(path);
  }

  async #readSource(root: string, configuredPath: string): Promise<Source | undefined> {
    const location = this.#sourceLocation(root, configuredPath);
    if (location === undefined) return undefined;
    try {
      return await this.#readLocatedSource(root, location);
    } catch (error) {
      return this.#failedSourceRead(location, error);
    }
  }

  /** Paths outside the session folder are kept, keyed relative to it (`../sibling/x.ts`); the auditor decides. */
  #sourceLocation(root: string, configuredPath: string): SourceLocation | undefined {
    const path = sessionRelativePath(root, configuredPath);
    if (path === undefined || !this.#auditor.acceptsPath(path)) return undefined;
    return sourceLocation(resolve(root, configuredPath), path);
  }

  /**
   * Reads the file through one descriptor opened without following a final symlink. Its real path must stay inside
   * the checkout that owns it (the session folder for an auditor that judges one folder), and still name the file
   * that was opened, so a folder swapped for a symlink mid-read cannot bring outside bytes in. A file the auditor
   * finds no owner for is not read, and an unlisted path that is no regular file, or a binary one, is no source. The
   * file is judged by where it really is as well as by its spelled path (`judgedWhereItLeads`), as the auditor judges it.
   */
  async #readLocatedSource(root: string, location: SourceLocation): Promise<Source | undefined> {
    const handle = await openSource(location.absolute);
    if (handle === undefined) return undefined;
    try {
      const info = await handle.stat();
      if (!info.isFile()) return location.unlisted ? undefined : { path: location.path, language: location.language, readError: "Source is not a regular file" };
      const real = await this.#realPathInPlace(root, location, info);
      return real === undefined ? undefined : judgedWhereItLeads(await readOpenedSource(handle, location, info), real);
    } finally {
      await handle.close();
    }
  }

  /**
   * The file's real path, `/`-separated from the checkout that owns it, when it stays inside that checkout and still
   * names the file that was opened; undefined otherwise.
   */
  async #realPathInPlace(root: string, location: SourceLocation, info: Stats): Promise<string | undefined> {
    const owner = this.#owningCheckout(root, location.path);
    if (owner === undefined) return undefined;
    const canonical = await realpath(location.absolute);
    if (!withinFolder(owner, canonical)) return undefined;
    const named = await lstat(canonical);
    return named.dev === info.dev && named.ino === info.ino ? relative(owner, canonical).split(sep).join("/") : undefined;
  }

  #failedSourceRead(location: SourceLocation, error: unknown): Source {
    if (hasErrorCode(error, "ENOENT")) return { path: location.path, language: location.language, deleted: true };
    return { path: location.path, language: location.language, readError: errorMessage(error) };
  }

  async #auditEdits(
    discovery: SourceDiscovery,
    captured: CodeRuleEditBatch["edits"],
    checkpoints: CodeRuleCheckpoint[],
    uncaptured: ReadonlyMap<string, string>,
    mode?: AuditMode,
  ): Promise<CodeRuleResponse> {
    const sessionId = this.#requireSession();
    const edits = await this.#recoverPreimages(discovery, captured, uncaptured);
    for (const edit of edits) {
      // The evaluator keeps the text it held of a file it could not read, so the session keeps it too: another
      // program's later write there is then audited for it (`#sortUnownedWrite`).
      if (edit.readError !== undefined) continue;
      this.#forgetText(edit.path);
      if (edit.generatedText !== undefined) this.#rememberText(edit.path, edit.generatedText, "edit");
    }
    const paths = edits.map(edit => edit.path).sort(compareDeterministicStrings);
    this.#revision += 1;
    this.#onRevision?.(sessionId, this.#revision, paths);
    this.#lastResponse = await this.#auditor.auditGeneratedEdits(sessionId, {
      revision: this.#revision, checkpoints, edits, ...(mode === undefined ? {} : { mode }),
    });
    return this.#lastResponse;
  }

  /**
   * Gives each edit `uncaptured` names (made with no preimage captured: a shell write, a tool call whose start was not
   * seen) the source it changed, where that is known exactly: the text this session last saw the file with, else its
   * Git index or `HEAD` blob, each taken only when its SHA-256 is the digest observed before the change. Any other
   * keeps its `previousReadError`, so the generated lines stay unknown rather than guessed.
   */
  async #recoverPreimages(
    discovery: SourceDiscovery,
    edits: CodeRuleEditBatch["edits"],
    uncaptured: ReadonlyMap<string, string>,
  ): Promise<CodeRuleEditBatch["edits"]> {
    if (uncaptured.size === 0) return edits;
    const recovered = new Map<string, string>();
    const unresolved = this.#recoverFromTexts(batchDigests(edits, uncaptured), recovered);
    for (const [checkout, paths] of this.#byOwningCheckout(discovery.root, unresolved, path => path).owned)
      await recoverFromGit(discovery, checkout, paths, uncaptured, recovered);
    return edits.map(edit => {
      const previousText = recovered.get(edit.path);
      if (previousText === undefined) return edit;
      const { previousReadError: _uncaptured, ...rest } = edit;
      return { ...rest, previousText };
    });
  }

  /** Puts in `recovered` each source this session last saw with the digest `uncaptured` names; the rest it returns. */
  #recoverFromTexts(uncaptured: ReadonlyMap<string, string>, recovered: Map<string, string>): string[] {
    const unresolved: string[] = [];
    for (const [path, digest] of uncaptured) {
      const text = this.#texts.get(path)?.text;
      if (text !== undefined && createHash("sha256").update(text).digest("hex") === digest) recovered.set(path, text);
      else unresolved.push(path);
    }
    return unresolved;
  }

  /**
   * `items` by the checkout that owns each one's path, whose Git holds its blobs (a worktree in the session's folder is
   * not the session's checkout); `unowned` has those no checkout owns, which no lane audits.
   */
  #byOwningCheckout<T>(root: string, items: readonly T[], pathOf: (item: T) => string): { owned: Map<string, T[]>; unowned: T[] } {
    const owned = new Map<string, T[]>();
    const unowned: T[] = [];
    for (const item of items) {
      const checkout = this.#owningCheckout(root, pathOf(item));
      if (checkout === undefined) unowned.push(item);
      else owned.set(checkout, [...(owned.get(checkout) ?? []), item]);
    }
    return { owned, unowned };
  }

  /** The checkout that owns `path`: the session folder for an auditor that judges one folder. */
  #owningCheckout(root: string, path: string): string | undefined {
    return this.#auditor.sourceRoot === undefined ? root : this.#auditor.sourceRoot(path);
  }

  /**
   * Puts in `scan.firstSeen` the text of each source the walk saw first, in the session folder or a worktree new to it,
   * that its checkout's index may not hold (`gitModifiedOrUntrackedPaths`): changed before the session, neither its
   * index nor its `HEAD` blob is the preimage of the session's next shell write to it.
   */
  async #readFirstSeenTexts(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): Promise<void> {
    await this.#takeFirstSeenTexts(discovery, await this.#firstSeenModifiedOrUntracked(discovery, walk, scan), scan);
  }

  /**
   * Reads each of `listed` again, while the session's first-sight budget lasts, and puts in `taking.firstSeen` each that
   * still has the digest `taking.sources` holds for it.
   */
  async #takeFirstSeenTexts(discovery: SourceDiscovery, listed: string[], taking: FirstSightTake): Promise<void> {
    let bytes = this.#firstSightBytes;
    while (listed.length > 0 && bytes < this.#firstSightTextBytes && !this.#abandoned(discovery)) {
      const wave = listed.splice(0, OBSERVATION_CONCURRENCY);
      bytes += takeFirstSeen(taking, await Promise.all(wave.map(path => this.#readSource(discovery.root, path))));
    }
  }

  /**
   * The sources `walk` saw first that their checkout's Git lists as modified or untracked. A checkout whose Git ran past
   * its time limit lists none: the walk goes on, and puts what it saw first there in `scan.lateListings`, for a listing
   * asked again once the walk is applied (`#listLateAgain`). Had the walk taken it as listing none, each such file
   * would be measured whole at the session's next shell write to it; had it waited for a second listing, or failed,
   * what it found would be checked at a later boundary.
   */
  async #firstSeenModifiedOrUntracked(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): Promise<string[]> {
    const firstSeen = [...scan.sources.keys()].filter(path => this.#firstSeen(discovery, walk, path));
    const listed: string[] = [];
    for (const [checkout, paths] of this.#byOwningCheckout(discovery.root, firstSeen, path => path).owned) {
      const seen = new Set(paths);
      const changed = await discovery.git.run(signal => gitModifiedOrUntrackedPaths(checkout, { signal }));
      if (changed === undefined) {
        scan.lateListings.set(checkout, sourcesAt(scan, paths));
        continue;
      }
      for (const path of changed) {
        const inSession = sessionPath(discovery.root, resolve(checkout, path));
        if (seen.has(inSession)) listed.push(inSession);
      }
    }
    return listed;
  }

  /** The walk is the first to see `path` (`#baselinePath`), which it answers for, and the session holds no text for it. */
  #firstSeen(discovery: SourceDiscovery, walk: WalkStart, path: string): boolean {
    if (this.#texts.has(path) || !walkAnswersFor(discovery, walk, path)) return false;
    return this.#baselinePath(discovery, path, walk.previous.get(path), walk.baseline);
  }

  /**
   * Keeps each of a walk's `firstSeen` texts (no change of the session's) that the session holds no text for, while it
   * is still what the session last observed of the source and the session's first-sight budget holds it.
   */
  #keepFirstSeenTexts(firstSeen: SourceScan["firstSeen"]): void {
    for (const [path, { text, digest }] of firstSeen) {
      if (this.#firstSightBytes + Buffer.byteLength(text) > this.#firstSightTextBytes) continue;
      if (!this.#texts.has(path) && this.#observedSource(path)?.digest === digest) this.#rememberText(path, text, "first-sight");
    }
  }

  /** Keeps `text`, from `origin`, as the source's latest; past `MAX_SESSION_TEXT_BYTES`, the least recently changed go. */
  #rememberText(path: string, text: string, origin: TextOrigin): void {
    this.#forgetText(path);
    const bytes = Buffer.byteLength(text);
    this.#texts.set(path, { text, bytes, origin });
    this.#textBytes += bytes;
    if (origin === "first-sight") this.#firstSightBytes += bytes;
    for (const [oldest] of this.#texts) {
      if (this.#textBytes <= this.#sessionTextBytes) break;
      this.#forgetText(oldest);
    }
  }

  #forgetText(path: string): void {
    this.#resumedHeld.delete(path);
    const entry = this.#texts.get(path);
    if (entry !== undefined) this.#dropText(path, entry);
  }

  #dropText(path: string, entry: { bytes: number; origin: TextOrigin }): void {
    this.#texts.delete(path);
    this.#textBytes -= entry.bytes;
    if (entry.origin === "first-sight") this.#firstSightBytes -= entry.bytes;
  }

  #touchSource(discovery: SourceDiscovery, path: string): void {
    discovery.versions.set(path, (discovery.versions.get(path) ?? 0) + 1);
  }

  #requireDiscovery(): SourceDiscovery {
    const discovery = this.#discovery;
    if (discovery === undefined || discovery.cancelled) throw new Error("Policy workspace is not open");
    return discovery;
  }

  #assertDiscovery(discovery: SourceDiscovery): void {
    if (this.#discovery !== discovery) throw new Error("Policy workspace is not open");
  }

  /**
   * The walks' gaps as a `source-incomplete` failure on the last response (`saidGaps`); the last response alone when there
   * are none. Each gap the walks found is told to `onDiscoveryGaps` too, as its own gap, until the record holds it
   * (`#tellGaps`), whether or not the text says it again; a walk still to finish is none.
   */
  #discoveryResponse(discovery: SourceDiscovery, boundary: DiscoveryGap["boundary"]): CodeRuleResponse {
    this.#tellGaps(boundary, discovery);
    const message = discoveryGap(discovery, saidGaps(discovery));
    if (message === undefined) return this.#lastResponse;
    return {
      ...this.#lastResponse,
      failures: [...this.#lastResponse.failures, {
        clauseId: "source-discovery", code: "source-incomplete", message, stage: "scan",
      }],
    };
  }

  /**
   * `gap` when the session's record holds no gap of its key (`#toldGaps`), nor is about to (`telling`); past
   * `MAX_TOLD_GAPS` gaps, the one that says the rest go untold (`untoldGaps`), until it is told. A gap that heals and
   * comes back is not told again.
   */
  #untold(gap: DiscoveryGap, telling: ReadonlyMap<string, DiscoveryGap>): DiscoveryGap | undefined {
    if (this.#toldGaps.has(gap.key) || telling.has(gap.key)) return undefined;
    const told = this.#toldGaps.size + telling.size < MAX_TOLD_GAPS ? gap : untoldGaps(gap.boundary);
    return this.#toldGaps.has(told.key) ? undefined : told;
  }

  #requireSession(): string {
    if (this.#sessionId === undefined) throw new Error("Policy workspace is not open");
    return this.#sessionId;
  }
  async #enqueue<T>(operation: () => Promise<T>): Promise<T> {
    const pending = this.#queue.then(operation, operation);
    this.#queue = pending.then(() => undefined, () => undefined);
    return await pending;
  }
}
