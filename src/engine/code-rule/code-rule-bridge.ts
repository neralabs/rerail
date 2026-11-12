import { createHash, randomBytes } from "node:crypto";
import { constants, type Dirent, type Stats } from "node:fs";
import { type FileHandle, lstat, mkdtemp, open, opendir, readFile, realpath, unlink } from "node:fs/promises";
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
import { fileStamp } from "../file-stamp.js";
import {
  gitBranchTips,
  gitCommitsBetween,
  gitCrlfPaths,
  gitHead,
  gitModifiedOrUntrackedPaths,
  GitReads,
  gitUnmergedReader,
  headMovedOnlyByCommits,
  readGitBlobIds,
  readGitBlobs,
  type CrlfRule,
} from "../git.js";
import type { RewoundPaths } from "./rerail-steer.js";
import { compareDeterministicStrings } from "../stable-json.js";
import { removeScratch, renameWithRetry } from "../file-replace.js";
import { blobIds, heldAs, textAs } from "./line-endings.js";
import { openNoFollow } from "../no-follow.js";
import { sourceLanguageOfPath, UNLISTED_TEXT_LANGUAGE } from "./code-rule-languages.js";
import { IGNORED_DIRECTORIES } from "./code-rule-enforcer.js";
import { binaryPrefix } from "./binary-prefix.js";
import { sensitiveByName } from "../path-boundary.js";
import { containsSecret, scrubSecrets } from "../secret-scrub.js";
import { IgnoreRuleFiles, type IgnoreRulesChange, shownIgnoreFile } from "./ignore-rule-files.js";
import { ToolCallWindows } from "./tool-call-windows.js";


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
 * A source as a tool call found it: its text, why it could not be read, or neither when it did not exist. A file named
 * like one holding secrets (`sensitiveByName`) keeps only its SHA-256 and size (`withheld`): preimages go into the
 * session ledger in the checkout, so its text is never captured, and `rewind` refuses to put it back. A preimage whose
 * text holds a secret is written to the ledger the same way ({@link ledgerPreimage}), though the session keeps its text.
 */
export interface Preimage {
  readonly text?: string;
  readonly readError?: string;
  readonly withheld?: { readonly sha256: string; readonly bytes: number };
}

/**
 * `preimage` as the session ledger records it: text holding a secret (`containsSecret`) is withheld, its SHA-256 and
 * size kept, so a session resumed from the ledger still finds the file unchanged by its digest but cannot put it back;
 * a read error is scrubbed. The live session keeps the text it captured.
 */
export function ledgerPreimage(preimage: Preimage): Preimage {
  if (preimage.text !== undefined && containsSecret(preimage.text)) {
    return { withheld: { sha256: createHash("sha256").update(preimage.text).digest("hex"), bytes: Buffer.byteLength(preimage.text) } };
  }
  return preimage.readError === undefined ? preimage : { ...preimage, readError: scrubSecrets(preimage.readError) };
}

interface Source {
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
}
export type ObservedSource = Omit<Source, "text" | "deleted">;
/** Baseline observations keyed by workspace directory, shared so later sessions reuse digests of unchanged sources. */
export type SourceObservationCache = Map<string, ReadonlyMap<string, ObservedSource>>;
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
/** A finished edit `rewind` may still roll back: the revision its finish audited, and each path's digest after it (none when absent). */
interface RewindableEdit {
  readonly revision: number;
  readonly preimages: ReadonlyMap<string, Preimage>;
  readonly after: ReadonlyMap<string, string | undefined>;
}
/** A source as a preimage: a file named like one holding secrets keeps only its digest and size. Undefined when it is none. */
function preimageOf(source: Source): Preimage | undefined {
  if (source.text !== undefined)
    return sensitiveByName(source.path) ? { withheld: { sha256: source.digest!, bytes: Buffer.byteLength(source.text) } } : { text: source.text };
  if (source.readError !== undefined) return { readError: source.readError };
  return source.deleted === true ? {} : undefined;
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
/** What Git holds for a walk's changes in one checkout, read once (`#gitVerdicts`). */
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
  const head = await git.run(signal => gitHead(checkout, { signal }));
  if (head === undefined) return undefined;
  const from = discovery.heads.get(checkout);
  if (from === undefined) discovery.heads.set(checkout, head);
  const unmerged = await readUnmergedView(discovery, checkout);
  const moved = from !== head;
  const committed = moved && await git.run(signal => headMovedOnlyByCommits(checkout, from, { signal }));
  const before = from === head ? undefined : from;
  const [blobs, crlf] = await Promise.all([
    // A blob larger than a source could never match its digest: it is skipped, never read into memory.
    git.run(signal => readGitBlobs(checkout, paths.flatMap(path => gitRevisions(path, before)), { maxBlobBytes: MAX_SOURCE_BYTES, signal })),
    git.run(signal => gitCrlfPaths(checkout, paths, { signal })),
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
/** A checkout's unmerged entries now and as the last walk saw them; the first entries read become its snapshot. */
async function readUnmergedView(discovery: SourceDiscovery, checkout: string): Promise<Pick<GitBlobs, "unmerged" | "unmergedBefore">> {
  const unmerged = await unmergedNow(discovery, checkout);
  const unmergedBefore = discovery.unmerged.get(checkout);
  if (unmergedBefore === undefined && unmerged !== undefined) discovery.unmerged.set(checkout, unmerged);
  return { unmerged, unmergedBefore };
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
/** The latest stamp of a file a baseline walk read, which a shell may have written while it read. */
async function modifiedSince(absolute: string, modifiedAt: number): Promise<number> {
  const after = await lstat(absolute);
  return Math.max(modifiedAt, after.mtimeMs, after.ctimeMs);
}
/** A text file as a walk observes it, with its text when it changed since the session's start. */
function observedText(walk: SourceWalk, stamped: StampedSource, previous: ObservedSource | undefined, source: Buffer): Source {
  const digest = sha256(source);
  const changed = (walk.captureChanges || stamped.modifiedAt >= walk.discovery.startedAt) && previous?.digest !== digest;
  return { ...stamped, digest, ...(changed ? { text: source.toString("utf8") } : {}) };
}
/** An unlisted file too large to read is unread, reported once per stamp (`newlyUnread`); any other read error is the walk's. */
function recordReadError(scan: SourceScan, source: Source): void {
  if (source.readError === undefined) return;
  if (unreadOversize(source)) scan.unread.set(source.path, { stamp: source.stamp, readError: source.readError });
  else scan.errors.push(`${source.path}: ${source.readError}`);
}
function unreadOversize(source: Source): source is Source & { readonly stamp: string } {
  return source.readError === OVERSIZE_ERROR && source.stamp !== undefined && sourceLanguageOfPath(source.path) === undefined;
}
/** A folder the walk could not read: its first sight next time is a baseline, and each source seen under it keeps the error. */
function failFolder(walk: SourceWalk, path: string, readError: string): void {
  const { previousSources, scan } = walk;
  scan.errors.push(`${path || "."}: ${readError}`);
  scan.failedRoots.add(path);
  const failed = new Set([path]);
  for (const [sourcePath, source] of previousSources)
    if (underBaselineRoot(failed, sourcePath)) scan.sources.set(sourcePath, { path: sourcePath, language: source.language, readError });
}
/** Whether the tool call left the source as its preimage: the same text, or a secret-like file's same digest. */
function unchangedSince(source: Source, preimage: Preimage | undefined): boolean {
  return preimage?.withheld === undefined ? preimage?.text === source.text : preimage.withheld.sha256 === source.digest;
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

/** The walk of the repository's other worktrees (`#scheduleWorktreeWalk`), kept apart from the session folder's. */
interface WorktreeDiscovery {
  /** The other worktrees, relative to the session folder. */
  roots: ReadonlySet<string>;
  /** Those whose first walk is still to finish: as with `SourceDiscovery.baselineRoots`, what it finds is a baseline. */
  readonly fresh: Set<string>;
  /** Subtrees the last walk could not read: what it first finds there is a baseline too. */
  baselineRoots: Set<string>;
  /** The walk in flight or timed; undefined while none is due. */
  walk: Promise<void> | undefined;
  /** A boundary came after the walk in flight had started, so it may have passed what changed: walk again after it. */
  requested: boolean;
  /** When the next walk may start. */
  nextWalkAt: number;
  /** What the last walk could not check, reported at each boundary as the session walk's `error` is. */
  error: string | undefined;
  /** The unlisted files the last walk found too large to read, as `SourceDiscovery.unread`. */
  unread: ReadonlyMap<string, { readonly stamp: string; readonly readError: string }>;
}

function worktreeDiscovery(): WorktreeDiscovery {
  return {
    roots: new Set(), fresh: new Set(), baselineRoots: new Set(), walk: undefined, requested: false, nextWalkAt: 0, error: undefined, unread: new Map(),
  };
}

interface SourceDiscovery {
  /** Filesystem-clock reading taken before the walk; files stamped at or after it may hold this session's shell writes. */
  readonly startedAt: number;
  readonly root: string;
  cancelled: boolean;
  /** Resolves once every requested pass has run; undefined while no walk is in flight. */
  idle: Promise<void> | undefined;
  baselineComplete: boolean;
  requested: DiscoveryCheckpoint | undefined;
  readonly versions: Map<string, number>;
  /** Subtrees whose first successful observation must still be a baseline. */
  baselineRoots: Set<string>;
  readonly worktrees: WorktreeDiscovery;
  error: string | undefined;
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

interface SourceScan {
  readonly sources: Map<string, Source>;
  readonly failedRoots: Set<string>;
  readonly errors: string[];
  /** Unlisted files too large to read, by path (`SourceDiscovery.unread`); left out of `errors`. */
  readonly unread: Map<string, { readonly stamp: string; readonly readError: string }>;
  /**
   * The text of each source the walk saw first that its checkout's Git may not hold, read again after the scan, by path
   * (`#readFirstSeenTexts`): the preimage of the session's next shell write to it, which no blob could give.
   */
  readonly firstSeen: Map<string, { readonly text: string; readonly digest: string }>;
  /** The checkout that owns each folder the walk passed a `.git` entry in: a checkout inside the session folder is found here. */
  readonly checkouts: Set<string>;
  /** The unmerged entries of each of `checkouts` that no walk had read, read after the scan (`#readPassedCheckouts`). */
  readonly unmerged: Map<string, ReadonlyMap<string, string>>;
}
/**
 * Where a text `CodeRuleBridge` keeps came from: an edit the session made, text Git wrote that neither the index nor
 * `HEAD` holds, another writer's change in the session folder, or a walk's first sight of a source Git may not hold,
 * which is no change of the session's.
 */
type TextOrigin = "edit" | "git" | "other-writer" | "first-sight";
/** Whether the evaluator holds a text of that origin as the session's: what it last audited of the source. */
const HELD_BY_EVALUATOR: Readonly<Record<TextOrigin, boolean>> = { edit: true, git: true, "other-writer": false, "first-sight": false };
/** One traversal of the workspace: what it compares against, when it started, and what it has found so far. */
interface SourceWalk {
  readonly discovery: SourceDiscovery;
  readonly observedAt: number;
  readonly previousSources: ReadonlyMap<string, ObservedSource>;
  readonly captureChanges: boolean;
  readonly scan: SourceScan;
  readonly stack: string[];
  /** Where the walk starts: a folder reached again from another is not walked twice. */
  readonly roots: ReadonlySet<string>;
  /** What the walk asks of each file and folder it finds: the auditor's `walkView` for this walk, or the auditor. */
  readonly view: WalkView;
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
  /** Filesystem-clock reading taken as the walk started. */
  readonly observedAt: number;
  /**
   * What the walk asks of each file and folder it finds (`SourceWalk.view`), and of the checkout of each file it found or
   * found gone, from its scan to its audit: the auditor's `walkView` for this walk, or the auditor.
   */
  readonly view: WalkView;
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
/** The digest a changed source was observed with before, when the walk found it changed and could not read what it was. */
function uncapturedDigest(edit: CodeRuleEdit, before: ObservedSource | undefined, baseline: boolean): string | undefined {
  if (baseline || edit.previousReadError === undefined || edit.generatedText === undefined) return undefined;
  return before?.digest;
}
/** A first observation made after the session started may hold its shell writes, and is not a trustworthy preimage. */
function changedSinceStart(discovery: SourceDiscovery, after: Source | undefined): boolean {
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
/** A worktree walk a boundary asked for has yet to finish, and there are worktrees for it to check. */
function worktreeWalkAbandoned(discovery: SourceDiscovery): boolean {
  return discovery.worktrees.walk !== undefined && discovery.worktrees.roots.size > 0;
}
/** Each of `unread` this session has not yet reported at its stamp, now marked reported. */
function newlyUnread(discovery: SourceDiscovery, unread: SourceDiscovery["unread"]): string[] {
  const errors: string[] = [];
  for (const [path, { stamp, readError }] of unread) {
    if (discovery.reported.get(path) === stamp) continue;
    discovery.reported.set(path, stamp);
    errors.push(`${path}: ${readError}`);
  }
  return errors;
}
/**
 * What the last walks of the session folder and of the other worktrees could not check, with each unlisted file too large
 * to read that this session has not yet reported at its stamp; else whether the session folder's walk is still to finish.
 */
function discoveryGap(discovery: SourceDiscovery): string | undefined {
  const errors = [discovery.error, discovery.worktrees.error].filter(error => error !== undefined);
  for (const unread of [discovery.unread, discovery.worktrees.unread]) errors.push(...newlyUnread(discovery, unread));
  if (errors.length > 0) return errors.join("; ");
  if (!discovery.baselineComplete) return "Initial source discovery is still pending";
  return discovery.idle === undefined ? undefined : "Source reconciliation is still pending";
}
function hasErrorCode(error: unknown, code: string): boolean {
  return error instanceof Error && "code" in error && error.code === code;
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
/** Recording what a boundary found is best-effort: a record that throws is logged, and the boundary goes on. */
function reportBestEffort(report: () => void): void {
  try {
    report();
  } catch (error) {
    process.stderr.write(`rerail: policy execution recording failed: ${errorMessage(error)}\n`);
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
function underBaselineRoot(roots: ReadonlySet<string>, path: string): boolean {
  for (const root of roots) if (root === "" || path.startsWith(`${root}/`)) return true;
  return false;
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
function uncapturedPreimage(baselinePath: boolean, before: ObservedSource | undefined): Pick<CodeRuleEdit, "previousReadError"> {
  if (baselinePath) return { previousReadError: "Source changed during initial discovery without a captured preimage" };
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
function takeFirstSeen(scan: SourceScan, sources: ReadonlyArray<Source | undefined>): number {
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
    discovery.git.run(signal => gitCrlfPaths(checkout, named.map(({ inCheckout }) => inCheckout), { signal })),
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
 * The least time between walks of the repository's other worktrees, which take time per worktree and nothing waits for;
 * a walk that took longer than this waits as long as it took, so walking never takes more than half the time.
 */
export const WORKTREE_WALK_GAP_MS = 5_000;

/**
 * A boundary answered while a source walk was still running: its shell writes are checked later, or never for `close`,
 * which a worktree walk still due also makes pending.
 */
export type DiscoveryPendingBoundary = "turn" | "final" | "command" | "close";

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
  readonly #observed = new Map<string, ObservedSource>();
  /**
   * The text each source this session changed was last seen with: the preimage of its next change made without one
   * captured, once its digest proves it (`#recoverPreimages`). The baseline is not kept, but for a source a walk first
   * sees with text Git may not hold (`#keepFirstSeenTexts`), which no other preimage could give; that text is no change
   * of the session's (`first-sight`), and takes at most `FIRST_SIGHT_TEXT_BYTES`. At most `MAX_SESSION_TEXT_BYTES` of
   * them are kept, the least recently changed evicted first: an evicted source falls back to Git.
   */
  readonly #texts = new Map<string, { text: string; bytes: number; origin: TextOrigin }>();
  #textBytes = 0;
  /** The calls the runtime answered while the evaluator opened (`unwatchedCall`), until the workspace opens and takes them. */
  #unwatched = new ToolCallWindows();
  #lastResponse = emptyResponse(0);
  #sessionId: string | undefined;
  #revision = 0;
  #queue: Promise<void> = Promise.resolve();
  #discovery: SourceDiscovery | undefined;
  readonly #discoveryWaitMs: number;
  readonly #onDiscoveryPending: ((boundary: DiscoveryPendingBoundary) => void) | undefined;
  readonly #onIgnoredEdit: ((edit: IgnoredEdit) => void) | undefined;
  readonly #onIgnoreRulesChanged: ((change: IgnoreRulesChange) => void) | undefined;
  /**
   * The session's ignore rule files, read as it opens while `onIgnoreRulesChanged` is told of their changes. The open
   * never waits for them; the session's first tool call does, so what they hold before it runs is the baseline.
   */
  #ignoreRules: Promise<IgnoreRuleFiles | undefined> = Promise.resolve(undefined);
  /** The `.gitignore` files walks found new, changed or gone since the last look at the ignore rules, session-relative. */
  readonly #walkedIgnoreChanges = new Set<string>();
  readonly #worktreeWalkGapMs: number;
  /** The most of `#texts` first-sight text may take (`FIRST_SIGHT_TEXT_BYTES`), and how much it takes now. */
  readonly #firstSightTextBytes: number;
  #firstSightBytes = 0;

  constructor(
    cwd: string,
    auditor: CodeRuleAuditor,
    options: {
      onRevision?(sessionId: string, revision: number, paths: readonly string[]): void;
      observations?: SourceObservationCache;
      /** How long a boundary waits for a source walk; `BOUNDARY_DISCOVERY_WAIT_MS` by default. */
      discoveryWaitMs?: number;
      /** Told when a boundary answers, or the bridge closes, while a source walk is still running. */
      onDiscoveryPending?(boundary: DiscoveryPendingBoundary): void;
      /** Told of each file a finished tool edit wrote that no rule checked because Git ignores it. */
      onIgnoredEdit?(edit: IgnoredEdit): void;
      /** Told when the session changes Git's ignore rules; without it they are not watched. */
      onIgnoreRulesChanged?(change: IgnoreRulesChange): void;
      /** `WORKTREE_WALK_GAP_MS` by default. */
      worktreeWalkGapMs?: number;
      /** `FIRST_SIGHT_TEXT_BYTES` by default. */
      firstSightTextBytes?: number;
    } = {},
  ) {
    this.#cwd = resolve(cwd);
    this.#auditor = auditor;
    this.#onRevision = options.onRevision;
    this.#observations = options.observations;
    this.#discoveryWaitMs = options.discoveryWaitMs ?? BOUNDARY_DISCOVERY_WAIT_MS;
    this.#onDiscoveryPending = options.onDiscoveryPending;
    this.#onIgnoredEdit = options.onIgnoredEdit;
    this.#onIgnoreRulesChanged = options.onIgnoreRulesChanged;
    this.#worktreeWalkGapMs = options.worktreeWalkGapMs ?? WORKTREE_WALK_GAP_MS;
    this.#firstSightTextBytes = options.firstSightTextBytes ?? FIRST_SIGHT_TEXT_BYTES;
  }

  /** Resolves once the evaluator is open; the baseline walk runs behind it and delays only the next checkpoint or command result. */
  async openWorkspace(sessionId: string): Promise<void> {
    await this.#enqueue(async () => {
      const startedAt = await filesystemClock();
      const previous = this.#discovery;
      if (previous !== undefined) previous.cancelled = true;
      if (this.#sessionId !== undefined) {
        this.#sessionId = undefined;
        await this.#auditor.close();
      }
      this.#pending.clear();
      this.#rewindable.clear();
      this.#commands.clear();
      this.#observed.clear();
      this.#texts.clear();
      this.#textBytes = 0;
      this.#firstSightBytes = 0;
      this.#discovery = undefined;
      await previous?.git.stop();
      const root = await realpath(this.#cwd);
      await this.#auditor.openWorkspace(sessionId);
      const git = new GitReads();
      // After the auditor opens, so it can name the checkouts whose ignore rules apply.
      this.#ignoreRules = this.#openIgnoreRules(root, git);
      this.#sessionId = sessionId;
      this.#revision = 0;
      this.#lastResponse = emptyResponse(0);
      const discovery: SourceDiscovery = {
        startedAt, root, cancelled: false, idle: undefined, baselineComplete: false, requested: undefined,
        versions: new Map(), baselineRoots: new Set(), worktrees: worktreeDiscovery(), error: undefined, unread: new Map(), reported: new Map(), pendingReported: false,
        heads: new Map(), git, unmerged: new Map(), passedCheckouts: new Set(), readUnmerged: gitUnmergedReader(), toolCalls: this.#unwatched,
      };
      this.#unwatched = new ToolCallWindows();
      this.#discovery = discovery;
      // The lowest priority: whichever boundary first waits on the baseline labels what it finds.
      void this.#requestDiscovery(discovery, "edit");
    });
  }

  async begin(
    toolCallId: string,
    toolName: string,
    input: unknown,
    restoredPreimages?: Readonly<Record<string, Preimage>>,
  ): Promise<Readonly<Record<string, Preimage>>> {
    const discovery = this.#requireDiscovery();
    discovery.toolCalls.begin(toolCallId, await filesystemClock());
    this.refreshCodeRules();
    const paths = generatedEditPaths(toolName, input);
    // What the ignore files a tool is about to write hold now is the baseline its edit is compared with.
    await this.#rememberIgnoreFiles(paths);
    if (paths.length === 0) {
      if (isCommandTool(toolName)) this.#commands.add(toolCallId);
      return {};
    }
    return await this.#enqueue(async () => {
      this.#assertDiscovery(discovery);
      const preimages = restoredPreimages === undefined
        ? await this.#capturePreimages(paths)
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
   * whose edit tool no hook shows opens one for each turn the same way, as its writes are seen by no call.
   */
  async unwatchedCall(toolCallId: string): Promise<void> {
    (this.#discovery?.toolCalls ?? this.#unwatched).begin(toolCallId, await filesystemClock());
  }

  /** Ends the window `unwatchedCall` opened for the call, at its result. */
  async unwatchedResult(toolCallId: string): Promise<void> {
    (this.#discovery?.toolCalls ?? this.#unwatched).finish(toolCallId, await filesystemClock());
  }

  /**
   * A failed command may still have written files, so command results reconcile regardless of `failed`. With
   * `rewindable` the edit's preimages are kept for `rewind` until `release`.
   */
  async finish(toolCallId: string, failed = false, options: { rewindable?: boolean } = {}): Promise<readonly CodeRuleResponse[]> {
    const discovery = this.#requireDiscovery();
    discovery.toolCalls.finish(toolCallId, await filesystemClock());
    if (this.#commands.delete(toolCallId)) {
      const responses = await this.#reconcile(discovery);
      await this.#noteIgnoreRules([]);
      return responses;
    }
    return await this.#enqueue(async () => {
      this.#assertDiscovery(discovery);
      const pending = this.#pending.get(toolCallId);
      this.#pending.delete(toolCallId);
      if (pending === undefined || failed) return [];
      this.refreshCodeRules();
      this.#reportIgnoredEdits(discovery.root, pending.paths);
      await this.#noteIgnoreRules(pending.paths);
      const { edits, uncaptured } = await this.#readEdits(discovery, pending.paths, pending.preimages);
      if (edits.length === 0) return [];
      // Kept before the audit starts, so a steer its evaluation publishes finds it.
      if (options.rewindable === true) {
        const after = new Map([...pending.preimages.keys()].map(path => [path, this.#observed.get(path)?.digest]));
        this.#rewindable.set(toolCallId, { revision: this.#revision + 1, preimages: pending.preimages, after });
      }
      return [await this.#auditEdits(discovery, edits, ["edit", "function"], uncaptured)];
    });
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
   * The call's edit, out of the window `rewind` may roll it back in, when a steer at `revision` over `scope` may: the scope
   * lies within its captured paths and each preimage was read. One holding secrets has no copy to put back, and throws.
   */
  #takeRewindable(toolCallId: string, revision: number, scope: readonly string[]): RewindableEdit | undefined {
    const rewindable = this.#rewindable.get(toolCallId);
    if (rewindable === undefined || rewindable.revision !== revision) return undefined;
    this.#rewindable.delete(toolCallId);
    if (!coversScope(rewindable.preimages, scope)) return undefined;
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
    return this.#discoveryResponse(discovery);
  }

  async #reconcile(discovery: SourceDiscovery): Promise<readonly CodeRuleResponse[]> {
    const revision = this.#revision;
    this.refreshCodeRules();
    await this.#awaitDiscovery(discovery, "edit", "command");
    this.#assertDiscovery(discovery);
    return this.#revision === revision ? [] : [this.#lastResponse];
  }
  async settleGeneratedEdits(): Promise<void> {
    this.#requireSession();
    await this.#queue;
    if (this.#auditor.settleGeneratedEdits !== undefined) {
      await this.#auditor.settleGeneratedEdits();
    } else {
      await this.#auditor.settle?.();
    }
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
    await this.#enqueue(async () => {
      if (this.#discovery !== discovery) return;
      this.#sessionId = undefined;
      this.#pending.clear();
      this.#rewindable.clear();
      this.#commands.clear();
      this.#observed.clear();
      this.#texts.clear();
      this.#textBytes = 0;
      this.#firstSightBytes = 0;
      this.#discovery = undefined;
      // The walk itself is never waited for, but every Git it or the ignore rules started is: on Windows one still running
      // would keep the checkout from being removed or renamed once the session has ended.
      await discovery.git.stop();
      await this.#auditor.close(options);
    });
  }
  async #capturePreimages(paths: readonly string[]): Promise<ReadonlyMap<string, Preimage>> {
    const root = await realpath(this.#cwd);
    const preimages = new Map<string, Preimage>();
    for (const configuredPath of [...paths].sort(compareDeterministicStrings)) {
      const source = await this.#readSource(root, configuredPath);
      if (source === undefined) continue;
      this.#updateObservedSource(source);
      const preimage = preimageOf(source);
      if (preimage !== undefined) preimages.set(source.path, preimage);
    }
    return preimages;
  }

  /** The tool call's edits, and the digest each source had before it where the call's start captured no preimage. */
  async #readEdits(
    discovery: SourceDiscovery,
    paths: readonly string[],
    preimages: ReadonlyMap<string, Preimage>,
  ): Promise<{ edits: CodeRuleEdit[]; uncaptured: Map<string, string> }> {
    const root = await realpath(this.#cwd);
    const edits: CodeRuleEdit[] = [];
    const uncaptured = new Map<string, string>();
    for (const configuredPath of [...paths].sort(compareDeterministicStrings)) {
      const edit = await this.#readToolEdit(discovery, root, configuredPath, preimages, uncaptured);
      if (edit !== undefined) edits.push(edit);
    }
    return { edits, uncaptured };
  }

  async #readToolEdit(
    discovery: SourceDiscovery,
    root: string,
    configuredPath: string,
    preimages: ReadonlyMap<string, Preimage>,
    uncaptured: Map<string, string>,
  ): Promise<CodeRuleEdit | undefined> {
    const source = await this.#readSource(root, configuredPath);
    if (source === undefined) return undefined;
    this.#touchSource(discovery, source.path);
    const preimage = preimages.get(source.path);
    const before = this.#observed.get(source.path)?.digest;
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
    const timer = setTimeout(() => budget.resolve(false), this.#discoveryWaitMs);
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
          this.#recordDiscoveryError(discovery, error);
        }
      }
    } finally {
      discovery.idle = undefined;
      discovery.pendingReported = false;
      this.#scheduleWorktreeWalk(discovery);
    }
  }

  #recordDiscoveryError(discovery: SourceDiscovery, error: unknown): void {
    if (this.#discovery === discovery && !discovery.cancelled) discovery.error = errorMessage(error);
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
  }

  /**
   * Walks the other worktrees once, behind the session folder's walk and never in its way: a command does not wait for
   * it, and what it finds is recorded, never steered (`#auditWalkEdits`). A boundary that comes before the next walk is due sets a
   * timer rather than dropping it, and one that comes while a walk runs has it walk again after, so the last write of a
   * turn is still seen.
   */
  #scheduleWorktreeWalk(discovery: SourceDiscovery): void {
    if (discovery.cancelled || this.#auditor.worktreeRoots === undefined) return;
    const worktrees = discovery.worktrees;
    if (worktrees.walk !== undefined) {
      worktrees.requested = true;
      return;
    }
    const delay = Math.max(0, worktrees.nextWalkAt - Date.now());
    worktrees.walk = this.#walkWorktrees(discovery, delay).finally(() => {
      worktrees.walk = undefined;
      if (worktrees.requested) this.#scheduleWorktreeWalk(discovery);
    });
  }

  async #walkWorktrees(discovery: SourceDiscovery, delay: number): Promise<void> {
    if (delay > 0) await new Promise(wake => { setTimeout(wake, delay).unref?.(); });
    if (discovery.cancelled) return;
    // This walk sees whatever a boundary before it asked about.
    discovery.worktrees.requested = false;
    const startedAt = Date.now();
    try {
      await this.#walkSources(discovery, "worktrees", "edit");
    } catch (error) {
      if (!this.#abandoned(discovery)) discovery.worktrees.error = errorMessage(error);
    }
    discovery.worktrees.nextWalkAt = Date.now() + Math.max(this.#worktreeWalkGapMs, Date.now() - startedAt);
  }

  /** Waits for the worktree walk in flight or timed, if any, and any walk a boundary asked for meanwhile. */
  async settleWorktreeWalk(): Promise<void> {
    const discovery = this.#discovery;
    while (discovery?.worktrees.walk !== undefined) await discovery.worktrees.walk;
  }

  async #discoveryPass(discovery: SourceDiscovery): Promise<void> {
    const checkpoint = discovery.requested!;
    discovery.requested = undefined;
    await this.#walkSources(discovery, "session", checkpoint);
  }

  /** One walk of `scope`: the session folder, or the repository's other worktrees; neither changes what the other owns. */
  async #walkSources(discovery: SourceDiscovery, scope: "session" | "worktrees", checkpoint: DiscoveryCheckpoint): Promise<void> {
    const worktrees = this.#syncWorktrees(discovery);
    const inSession = scope === "session";
    const walk: WalkStart = {
      inSession, baseline: inSession && !discovery.baselineComplete, checkpoint, worktrees: new Set(discovery.worktrees.roots),
      previous: new Map(this.#observed), versions: new Map(discovery.versions),
      // Read before the walk, so a move or conflict the walk only partly saw is still one to the next.
      ...await this.#readCheckouts(discovery, inSession ? undefined : worktrees),
      observedAt: await filesystemClock(),
      view: this.#auditor.walkView?.() ?? this.#auditor,
    };
    const scan = await this.#scanSources(discovery, walk, worktrees);
    if (this.#abandoned(discovery)) return;
    await this.#enqueue(() => this.#applyWalk(discovery, walk, scan));
    // Reconcile changes to already-observed files made during the initial traversal.
    if (walk.baseline && discovery.requested === undefined) discovery.requested = checkpoint;
  }

  #abandoned(discovery: SourceDiscovery): boolean {
    return discovery.cancelled || this.#discovery !== discovery;
  }

  async #scanSources(discovery: SourceDiscovery, walk: WalkStart, worktrees: readonly string[]): Promise<SourceScan> {
    // An earlier session's baseline of this workspace lets unchanged sources skip the read.
    const seed = walk.baseline || (!walk.inSession && discovery.worktrees.fresh.size > 0) ? this.#observations?.get(this.#cwd) : undefined;
    const scan = await this.#observeSources({
      discovery, observedAt: walk.observedAt,
      previousSources: seed === undefined ? walk.previous : new Map([...seed, ...walk.previous]),
      captureChanges: !walk.baseline,
      scan: { sources: new Map(), failedRoots: new Set(), errors: [], unread: new Map(), firstSeen: new Map(), checkouts: new Set(), unmerged: new Map() },
      stack: walk.inSession ? [discovery.root] : [...worktrees], roots: new Set([discovery.root, ...worktrees]),
      view: walk.view,
    });
    // Read off the lane, as the scan is, so tool calls are not held up behind it.
    await this.#readFirstSeenTexts(discovery, walk, scan);
    await this.#readPassedCheckouts(discovery, walk, scan);
    return scan;
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
      if (own.length > 0) await this.#auditEdits(discovery, own, checkpoints, uncaptured, walk.inSession ? undefined : "log", walk.view);
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
      if (before === undefined) this.#observed.delete(edit.path);
      else this.#observed.set(edit.path, before);
    }
  }

  #finishSessionWalk(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): void {
    discovery.baselineComplete = true;
    discovery.baselineRoots = scan.failedRoots;
    discovery.error = scan.errors.length === 0 ? undefined : scan.errors.join("; ");
    discovery.unread = scan.unread;
    discovery.passedCheckouts = scan.checkouts;
    // A subtree it could not read may still hold writes made since the session opened, which its next walk judges.
    if (scan.failedRoots.size === 0) discovery.toolCalls.answered(walk.observedAt);
  }

  #finishWorktreeWalk(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): void {
    const worktrees = discovery.worktrees;
    for (const worktree of walk.worktrees) worktrees.fresh.delete(worktree);
    worktrees.baselineRoots = scan.failedRoots;
    worktrees.error = scan.errors.length === 0 ? undefined : scan.errors.join("; ");
    worktrees.unread = scan.unread;
  }

  /** The changes the walk found; `uncaptured` receives the digest each changed source was observed with before. */
  #discoverEdits(discovery: SourceDiscovery, walk: WalkStart, current: ReadonlyMap<string, Source>, uncaptured: Map<string, string>): CodeRuleEdit[] {
    const pendingPaths = this.#pendingPaths();
    const edits: CodeRuleEdit[] = [];
    for (const path of [...new Set([...walk.previous.keys(), ...current.keys()])].sort(compareDeterministicStrings)) {
      if (!walkAnswersFor(discovery, walk, path) || this.#toolChanged(discovery, walk, path, pendingPaths)) continue;
      const edit = this.#discoveredEdit(discovery, path, walk.previous.get(path), current.get(path), walk.baseline);
      if (edit === undefined) continue;
      edits.push(edit);
      const before = uncapturedDigest(edit, walk.previous.get(path), walk.baseline);
      if (before !== undefined) uncaptured.set(path, before);
    }
    return edits;
  }

  /** A begin/finish after scan launch wins even when its bytes happen to match the old snapshot. */
  #toolChanged(discovery: SourceDiscovery, walk: WalkStart, path: string, pendingPaths: ReadonlySet<string>): boolean {
    return discovery.versions.get(path) !== walk.versions.get(path) || pendingPaths.has(path);
  }

  /** The paths a tool call in progress has captured: its finish, not a walk, reports them. */
  #pendingPaths(): Set<string> {
    const paths = new Set<string>();
    for (const pending of this.#pending.values())
      for (const path of pending.preimages.keys()) paths.add(path);
    return paths;
  }

  /**
   * `HEAD` and the unmerged entries of each of `worktrees`, for their walk; without them, of the session folder's checkout
   * and of every other checkout a walk has found a change in, but for the worktrees, whose own walk answers for theirs.
   * Each is read at the checkout that owns its files, as `#gitVerdicts` asks for it: a session opened in a subfolder
   * reads the checkout around it, whose unmerged paths Git lists from its top folder.
   * Read on every pass, not only one that finds changes: a modify/delete conflict can stop a merge without changing a
   * file, and the next pass must still know that conflict was there before the edit it finds. So a checkout inside the
   * session folder that no walk has found a change in has its entries read too, and not its `HEAD`: the entries cost a
   * `stat` while its index file is as it was, where `HEAD` costs a Git process. A checkout whose entries Git could not
   * list is left out of `unmerged`, so the last entries read stay its snapshot.
   */
  async #readCheckouts(discovery: SourceDiscovery, worktrees?: readonly string[]): Promise<Pick<WalkStart, "heads" | "unmerged">> {
    const heads = new Map<string, string>();
    const unmerged = new Map<string, ReadonlyMap<string, string>>();
    const readEntries = async (root: string): Promise<void> => {
      const entries = await unmergedNow(discovery, root);
      if (entries !== undefined) unmerged.set(root, entries);
    };
    const walkedApart = (root: string): boolean => discovery.worktrees.roots.has(sessionPath(discovery.root, root));
    const others = [...discovery.heads.keys()].filter(known => !walkedApart(known));
    // When no checkout owns the session folder, no change in it is asked about.
    const session = this.#owningCheckout(discovery.root, "");
    const withHead = new Set(worktrees ?? (session === undefined ? others : [session, ...others]));
    for (const root of withHead) {
      const head = await discovery.git.run(signal => gitHead(root, { signal }));
      if (head === undefined) continue;
      heads.set(root, head);
      await readEntries(root);
    }
    if (worktrees !== undefined) return { heads, unmerged };
    for (const root of discovery.passedCheckouts) if (!withHead.has(root) && !walkedApart(root)) await readEntries(root);
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
   * date. Any other is the same to the evaluator as the files it never saw.
   */
  async #separateOwnWrites(
    discovery: SourceDiscovery,
    walk: WalkStart,
    edits: readonly CodeRuleEdit[],
    uncaptured: Map<string, string>,
  ): Promise<{ own: CodeRuleEdit[]; notOwn: CodeRuleEdit[] }> {
    const unwritten: CodeRuleEdit[] = [];
    const notOwn: CodeRuleEdit[] = [];
    const { owned, unowned } = this.#byOwningCheckout(discovery.root, edits, edit => edit.path, walk.view);
    unwritten.push(...unowned);
    for (const [checkout, group] of owned) {
      const verdicts = await this.#gitVerdicts(discovery, checkout, group, uncaptured);
      for (const edit of group) {
        const verdict = verdicts.get(edit.path);
        if (verdict === undefined) unwritten.push(edit);
        else if (verdict.previousText === undefined) this.#sortUnownedWrite(edit, verdict.keptAs, notOwn);
        else {
          uncaptured.delete(edit.path);
          unwritten.push({ ...withoutUncaptured(edit), previousText: verdict.previousText });
        }
      }
    }
    const own = walk.inSession ? await this.#claimOwnWrites(discovery, walk, unwritten, uncaptured, notOwn) : unwritten;
    return { own, notOwn };
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
   * since the walk that last saw the file began.
   */
  async #writtenByCall(discovery: SourceDiscovery, walk: WalkStart, edit: CodeRuleEdit): Promise<boolean> {
    const stamp = this.#observed.get(edit.path)?.modifiedAt ?? (edit.deleted === true ? await folderStamp(discovery.root, edit.path) : undefined);
    if (stamp !== undefined) return discovery.toolCalls.ranDuring(stamp, stamp);
    return discovery.toolCalls.ranDuring(walk.previous.get(edit.path)?.observedAt ?? discovery.startedAt, walk.observedAt);
  }

  /**
   * A write the session did not make is audited only for a source the evaluator holds the session's text of: one it
   * edited, or one Git wrote over. Text another writer left, or a first sight, is no change of the session's, so the
   * evaluator never held it. With `keptAs` (another writer's, or Git's text neither the index nor `HEAD` holds) its text
   * is kept for the session's next edit of the source.
   */
  #sortUnownedWrite(edit: CodeRuleEdit, keptAs: "git" | "other-writer" | undefined, notOwn: CodeRuleEdit[]): void {
    const held = this.#texts.get(edit.path);
    if (held !== undefined && HELD_BY_EVALUATOR[held.origin])
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
   */
  async #gitVerdicts(
    discovery: SourceDiscovery,
    checkout: string,
    edits: readonly CodeRuleEdit[],
    uncaptured: ReadonlyMap<string, string>,
  ): Promise<Map<string, GitVerdict>> {
    const verdicts = new Map<string, GitVerdict>();
    const named = edits.map(edit => ({ edit, path: sessionPath(checkout, resolve(discovery.root, edit.path)), prior: uncaptured.get(edit.path) }));
    const read = await readGitView(discovery, checkout, named.map(({ path }) => path));
    if (read === undefined) return verdicts;
    const view: GitView = { ...read, stopped: stoppedSince(read, named) };
    for (const { edit, path, prior } of named) {
      const verdict = edit.readError === undefined ? this.#gitVerdict(view, edit, path, prior) : undefined;
      if (verdict !== undefined) verdicts.set(edit.path, verdict);
    }
    await this.#addRestoredVerdicts(discovery.git, checkout, view, named, verdicts);
    return verdicts;
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
    const after = this.#observed.get(edit.path)?.digest;
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
   * Keeps `source` as this session's baseline of its path, keyed by the path string the observation holds: an unchanged
   * source's observation is the one every session sharing baseline observations holds, and so is its path.
   */
  #observe(source: Source): void {
    const observed = observation(source);
    this.#observed.set(observed.path, observed);
  }

  /**
   * The walk is a baseline, or it first sees `path` under a subtree a walk could not read or in a worktree whose first walk
   * is still to finish.
   */
  #baselinePath(discovery: SourceDiscovery, path: string, before: ObservedSource | undefined, baseline: boolean): boolean {
    if (baseline) return true;
    return before === undefined
      && [discovery.baselineRoots, discovery.worktrees.baselineRoots, discovery.worktrees.fresh].some(roots => underBaselineRoot(roots, path));
  }

  #deletedEdit(path: string, before: ObservedSource | undefined): CodeRuleEdit | undefined {
    this.#observed.delete(path);
    if (before === undefined) return undefined;
    return { path, language: before.language, previousReadError: "Source was deleted without a captured preimage", deleted: true };
  }

  async #observeSources(walk: SourceWalk): Promise<SourceScan> {
    const { discovery, stack, scan } = walk;
    while (!discovery.cancelled && stack.length > 0) {
      const wave = stack.splice(-OBSERVATION_CONCURRENCY);
      await Promise.all(wave.map(absolute => this.#observePath(walk, absolute)));
    }
    for (const source of scan.sources.values()) recordReadError(scan, source);
    return scan;
  }

  async #observePath(walk: SourceWalk, absolute: string): Promise<void> {
    let info: Stats | undefined;
    try {
      info = await lstat(absolute);
      await this.#observeEntry(walk, absolute, info);
    } catch (error) {
      if (!walk.discovery.cancelled) this.#recordObservationFailure(walk, absolute, error, info?.isFile() === true);
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
    for await (const entry of await opendir(absolute)) {
      if (walk.discovery.cancelled) return;
      if (entry.name === ".git") this.#noteCheckout(walk, join(absolute, entry.name));
      else if (this.#walksInto(walk, absolute, entry)) walk.stack.push(join(absolute, entry.name));
    }
  }

  /** Asks for the owner of the `.git` entry, not of its folder: it is owned as the files beside it are. */
  #noteCheckout(walk: SourceWalk, gitEntry: string): void {
    const checkout = this.#owningCheckout(walk.discovery.root, sessionPath(walk.discovery.root, gitEntry), walk.view);
    if (checkout !== undefined) walk.scan.checkouts.add(checkout);
  }

  /** A folder another walk starts at is not walked twice. */
  #walksInto(walk: SourceWalk, absolute: string, entry: Dirent): boolean {
    if (neverWalked(entry)) return false;
    if (!entry.isDirectory()) return entry.isFile();
    const child = join(absolute, entry.name);
    return !walk.roots.has(child) && walk.view.ignoresPath?.(sessionPath(walk.discovery.root, child), true) !== true;
  }

  /**
   * A file Git ignores is not observed at all: no baseline entry, so no later read or edit of it is audited. A
   * `.gitignore` is observed by its stamp whatever Git says of it, since it decides what is ignored.
   */
  async #observeRegularFile(walk: SourceWalk, absolute: string, info: Stats): Promise<void> {
    const path = sessionPath(walk.discovery.root, absolute);
    if (basename(path) === GITIGNORE || !walk.view.ignoresPath?.(path, false)) await this.#observeFile(walk, sourceLocation(absolute, path), info);
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
    return await this.#readObservedFile(walk, location, { ...stamped, modifiedAt }, previous);
  }

  async #readObservedFile(
    walk: SourceWalk,
    location: SourceLocation,
    stamped: StampedSource,
    previous: ObservedSource | undefined,
  ): Promise<Source | undefined> {
    const source = await readFile(location.absolute);
    if (walk.discovery.cancelled) return undefined;
    // A shell may mutate the file while readFile is pending. Keep the original stamp
    // so the next reconciliation re-reads it, but do not silently baseline those bytes.
    const modifiedAt = walk.captureChanges ? stamped.modifiedAt : await modifiedSince(location.absolute, stamped.modifiedAt);
    if (walk.discovery.cancelled) return undefined;
    const content = nonText(location, source);
    return content === undefined ? observedText(walk, { ...stamped, modifiedAt }, previous, source) : { ...stamped, modifiedAt, ...content };
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
      this.#observed.set(source.path, observation(source));
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
    if (source.deleted) this.#observed.delete(source.path);
    else this.#observed.set(source.path, observation(source));
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
   * finds no owner for is not read, and an unlisted path that is no regular file, or a binary one, is no source.
   */
  async #readLocatedSource(root: string, location: SourceLocation): Promise<Source | undefined> {
    const handle = await openSource(location.absolute);
    if (handle === undefined) return undefined;
    try {
      const info = await handle.stat();
      if (!info.isFile()) return location.unlisted ? undefined : { path: location.path, language: location.language, readError: "Source is not a regular file" };
      return await this.#readsInPlace(root, location, info) ? await readOpenedSource(handle, location, info) : undefined;
    } finally {
      await handle.close();
    }
  }

  /** Whether the file's real path stays inside the checkout that owns it and still names the file that was opened. */
  async #readsInPlace(root: string, location: SourceLocation, info: Stats): Promise<boolean> {
    const owner = this.#owningCheckout(root, location.path);
    if (owner === undefined) return false;
    const canonical = await realpath(location.absolute);
    if (!withinFolder(owner, canonical)) return false;
    const named = await lstat(canonical);
    return named.dev === info.dev && named.ino === info.ino;
  }

  #failedSourceRead(location: SourceLocation, error: unknown): Source {
    if (hasErrorCode(error, "ENOENT")) return { path: location.path, language: location.language, deleted: true };
    return { path: location.path, language: location.language, readError: errorMessage(error) };
  }

  /** `asked` names the checkouts of the files `uncaptured` holds: the auditor, or a walk's view for the files it found. */
  async #auditEdits(
    discovery: SourceDiscovery,
    captured: CodeRuleEditBatch["edits"],
    checkpoints: CodeRuleCheckpoint[],
    uncaptured: ReadonlyMap<string, string>,
    mode?: AuditMode,
    asked: Pick<CodeRuleAuditor, "sourceRoot"> = this.#auditor,
  ): Promise<CodeRuleResponse> {
    const sessionId = this.#requireSession();
    const edits = await this.#recoverPreimages(discovery, captured, uncaptured, asked);
    for (const edit of edits) {
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
    asked: Pick<CodeRuleAuditor, "sourceRoot">,
  ): Promise<CodeRuleEditBatch["edits"]> {
    if (uncaptured.size === 0) return edits;
    const recovered = new Map<string, string>();
    const unresolved = this.#recoverFromTexts(uncaptured, recovered);
    for (const [checkout, paths] of this.#byOwningCheckout(discovery.root, unresolved, path => path, asked).owned)
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
   * not the session's checkout); `unowned` has those no checkout owns, which no lane audits. `asked` is the auditor, or a
   * walk's view for the files the walk found (`#owningCheckout`).
   */
  #byOwningCheckout<T>(
    root: string,
    items: readonly T[],
    pathOf: (item: T) => string,
    asked: Pick<CodeRuleAuditor, "sourceRoot"> = this.#auditor,
  ): { owned: Map<string, T[]>; unowned: T[] } {
    const owned = new Map<string, T[]>();
    const unowned: T[] = [];
    for (const item of items) {
      const checkout = this.#owningCheckout(root, pathOf(item), asked);
      if (checkout === undefined) {
        unowned.push(item);
        continue;
      }
      const known = owned.get(checkout);
      if (known === undefined) owned.set(checkout, [item]);
      else known.push(item);
    }
    return { owned, unowned };
  }

  /**
   * The checkout that owns `path`, as `asked` names it: the session folder for an auditor that judges one folder. A
   * walk's view (`walkView`) answers for the files and `.git` entries the walk found from what it knows of their folder.
   */
  #owningCheckout(root: string, path: string, asked: Pick<CodeRuleAuditor, "sourceRoot"> = this.#auditor): string | undefined {
    return asked.sourceRoot === undefined ? root : asked.sourceRoot(path);
  }

  /**
   * Puts in `scan.firstSeen` the text of each source the walk saw first, in the session folder or a worktree new to it,
   * that its checkout's index may not hold (`gitModifiedOrUntrackedPaths`): changed before the session, neither its
   * index nor its `HEAD` blob is the preimage of the session's next shell write to it. Each is read again, while the
   * session's first-sight budget lasts, and taken only while it still has the digest the scan observed.
   */
  async #readFirstSeenTexts(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): Promise<void> {
    const listed = await this.#firstSeenModifiedOrUntracked(discovery, walk, scan);
    let bytes = this.#firstSightBytes;
    while (listed.length > 0 && bytes < this.#firstSightTextBytes && !this.#abandoned(discovery)) {
      const wave = listed.splice(0, OBSERVATION_CONCURRENCY);
      bytes += takeFirstSeen(scan, await Promise.all(wave.map(path => this.#readSource(discovery.root, path))));
    }
  }

  /**
   * The sources `walk` saw first that their checkout's Git lists as modified or untracked. The walk's view gives each
   * source its folder's checkout, which it looked up as it walked, so the sources cost no lookup of their own: the
   * first walk of a worktree sees every one of its files first.
   */
  async #firstSeenModifiedOrUntracked(discovery: SourceDiscovery, walk: WalkStart, scan: SourceScan): Promise<string[]> {
    const firstSeen = [...scan.sources.keys()].filter(path => this.#firstSeen(discovery, walk, path));
    const listed: string[] = [];
    for (const [checkout, paths] of this.#byOwningCheckout(discovery.root, firstSeen, path => path, walk.view).owned) {
      const seen = new Set(paths);
      for (const path of await discovery.git.run(signal => gitModifiedOrUntrackedPaths(checkout, { signal }))) {
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
      if (!this.#texts.has(path) && this.#observed.get(path)?.digest === digest) this.#rememberText(path, text, "first-sight");
    }
  }

  /** Keeps `text`, from `origin`, as the source's latest; past `MAX_SESSION_TEXT_BYTES`, the least recently changed go. */
  #rememberText(path: string, text: string, origin: TextOrigin): void {
    this.#forgetText(path);
    const bytes = Buffer.byteLength(text);
    this.#texts.set(path, { text, bytes, origin });
    this.#textBytes += bytes;
    if (origin === "first-sight") this.#firstSightBytes += bytes;
    for (const [oldest, entry] of this.#texts) {
      if (this.#textBytes <= MAX_SESSION_TEXT_BYTES) break;
      this.#dropText(oldest, entry);
    }
  }

  #forgetText(path: string): void {
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

  /** The walks' gaps as a `source-incomplete` failure on the last response; the last response alone when there are none. */
  #discoveryResponse(discovery: SourceDiscovery): CodeRuleResponse {
    const message = discoveryGap(discovery);
    if (message === undefined) return this.#lastResponse;
    return {
      ...this.#lastResponse,
      failures: [...this.#lastResponse.failures, {
        clauseId: "source-discovery", code: "source-incomplete", message, stage: "scan",
      }],
    };
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
