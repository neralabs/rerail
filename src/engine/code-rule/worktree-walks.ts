/**
 * The walk of a repository's other worktrees, shared by every session that watches them. A shell write in another
 * worktree is seen only by walking it, and every session of the repository walked them all for itself: each kept an
 * observation of every file of every other worktree, so with sessions in many worktrees the daemon's memory, and the time
 * it spent walking, grew with the sessions times the worktrees times their files. Here one walk reads each worktree
 * once, for every session watching it, and keeps what it saw as a snapshot; each session takes from a walk only the
 * sources that changed since the snapshot it took last (`CodeRuleBridge`).
 *
 * A walk reads only the worktrees that may have changed since it last read them: each it has no read of, each a change
 * notice named or that cannot be watched (`WorktreeNotices`), each where something no notice tells of moved (its
 * `HEAD`, its index, the worktrees inside it, and what `WorktreeLook` looks at), and each whose last read is
 * `WORKTREE_FULL_WALK_MS` old. It hands each session what it saw of the others last time. Reading them all stat'ed
 * every file of every worktree on each walk: with 190 worktrees, about 5 s of the daemon's main thread a walk.
 */
import { relative, sep } from "node:path";
import { gitModifiedOrUntrackedPaths, GitReads, gitTrackedIgnoredReader, gitUnmergedReader } from "../git.js";
import { readGitHead } from "../git-head-reader.js";
import type { ObservedSource, ScanError, Source, SourceScan } from "./code-rule-bridge.js";
import type { TreeWatch } from "./tree-watch.js";
import { lookMoved, type WalkFiles, type WorktreeLook, WorktreeLooks } from "./worktree-look.js";
import { SharedTreeWatch, WorktreeNotices, type WorktreeRead } from "./worktree-notices.js";

/**
 * The least time between two walks of the other worktrees that answer one session's asks, which take time per worktree
 * and nothing waits for; after a walk that took longer than this, the session waits as long as it took. So no session
 * keeps walks going more than half the time, as when each session walked them on its own, and one session's walk never
 * holds back another's.
 */
export const WORKTREE_WALK_GAP_MS = 5_000;

/**
 * How long walks trust change notices for a worktree: a walk reads one whose last read began this long ago, and one is
 * timed for it though every session watching it is idle. A write no notice tells of (through a hard link, a memory
 * map, a file another program holds open on Windows, or one the system lost without saying so) is found then.
 */
export const WORKTREE_FULL_WALK_MS = 600_000;

/**
 * How often walks take the change notices while they watch a worktree, between walks too. A notice comes some time
 * after its write (FSEvents holds them up to a tenth of a second, inotify and ReadDirectoryChangesW hand them over on a
 * thread of their own), so one may come just after a walk took the notices and decided not to read its worktree: the
 * walk it asks for then reads it, once each session watching it is past its gap.
 */
export const WORKTREE_NOTICE_MS = 250;

/**
 * What one walk saw of a worktree, each source by its path in the worktree (`/`-separated). A snapshot never changes: a
 * walk that finds a change makes a new one and keeps the observation of every source it found unchanged, so a session
 * finds what changed since the snapshot it took last by comparing observations by identity.
 */
export interface WorktreeSnapshot {
  readonly sources: ReadonlyMap<string, ObservedSource>;
}

/** What one walk found in one worktree; paths are relative to the worktree, `""` for the worktree itself. */
export interface WorktreeFindings {
  readonly snapshot: WorktreeSnapshot;
  /** The text of each source whose text the walk read as changed since the last snapshot, or as first seen after a watching session opened. */
  readonly texts: ReadonlyMap<string, string>;
  /** The worktree's `HEAD` as the walk began; undefined when Git could not say. */
  readonly head: string | undefined;
  /** Its index's unmerged entries as the walk began (`gitUnmergedReader`); undefined when Git could not list them. */
  readonly unmerged: ReadonlyMap<string, string> | undefined;
  /** The checkouts that own the files the walk found, each one whose `.git` entry it passed (the worktree's own first). */
  readonly checkouts: ReadonlySet<string>;
  /** What Git said of the checkouts inside the worktree: `checkouts` but the worktree's own. */
  readonly inner: InnerCheckouts;
  /**
   * For each of `checkouts`, the files its Git lists as modified or untracked (`gitModifiedOrUntrackedPaths`), relative
   * to it: listed once for every session that takes the worktree for the first time in this walk, and only then
   * (`firstSeenIn`). Undefined for a checkout whose Git ran past its time limit: it did not say which files changed.
   */
  readonly listed: ReadonlyMap<string, readonly string[] | undefined>;
  /** Subtrees the walk could not read. */
  readonly failedRoots: ReadonlySet<string>;
  readonly errors: readonly ScanError[];
  /** The unlisted files it found too large to read, with their stamps. */
  readonly unread: ReadonlyMap<string, { readonly stamp: string; readonly readError: string }>;
  /**
   * Filesystem-clock reading taken as the read these findings come from began: of a worktree the walk did not read, the
   * read before it. It saw every write made before then.
   */
  readonly readFrom: number;
  /** What this read found changed since the read before it; none for a worktree's first read, or one this walk did not read. */
  readonly changes: readonly WorktreeChange[];
}

/**
 * What Git said of the checkouts inside a worktree (a submodule there, or a repository of its own), each by its physical
 * path. Read for the walk that finds their files, as the worktree's own `HEAD` and entries are: one read before another
 * walk would be that walk's by the time this one looks.
 */
export interface InnerCheckouts {
  /** `HEAD` as the walk began, of each one a watching session holds a `HEAD` of (`WatcherWalk.heads`). */
  readonly heads: ReadonlyMap<string, string>;
  /**
   * The unmerged entries as the walk began (`gitUnmergedReader`), of each one the last read passed or a watching session
   * holds entries or a `HEAD` of (`WatcherWalk.unmerged`, `WatcherWalk.heads`).
   */
  readonly unmerged: ReadonlyMap<string, ReadonlyMap<string, string>>;
  /**
   * The unmerged entries as the read ended, of each one no read had passed: its snapshot, so a conflict that stops Git
   * there afterwards is new to the walk that follows it.
   */
  readonly passed: ReadonlyMap<string, ReadonlyMap<string, string>>;
}

/** What a walk found in each worktree it read, by the worktree's physical path, as of `observedAt`; or why it stopped. */
export type WorktreeWalk =
  | { readonly found: ReadonlyMap<string, WorktreeFindings>; readonly observedAt: number }
  | { readonly error: string };

/** What a walk asks the session it reads one worktree through. */
export interface WorktreeScan {
  /** The worktree, physical. */
  readonly root: string;
  /** The last snapshot's sources: one whose stamp holds is not read again. */
  readonly previous: ReadonlyMap<string, ObservedSource>;
  /** Keep the text of every source whose digest changed; without it, only of those stamped at or after `startedAt`. */
  readonly captureChanges: boolean;
  readonly startedAt: number;
  /** Subtrees the last read could not read: a source first seen under one keeps no text unless stamped since `startedAt`. */
  readonly baselineRoots: ReadonlySet<string>;
  /**
   * Whether the record of every session the read serves holds the EPERM gap of `path` (`WatcherWalk.holdsRefusal`): a
   * refusal there is a gap, and is not looked at again, so each read looks at those some session has not recorded yet.
   */
  readonly held: (path: string) => boolean;
  /**
   * The refusals the worktree's reads withdrew (`SourceWalk.withdrawn`), through whichever session read it: kept here,
   * by the walk, so a read through another session orders its looks by the same facts.
   */
  readonly withdrawn: Set<string>;
  /** Filesystem-clock reading taken as the walk began. */
  readonly observedAt: number;
  /**
   * Folders the walk leaves out: every worktree it reads and the checkout of every session taking part, so a worktree
   * reads the same whichever sessions take part.
   */
  readonly skip: ReadonlySet<string>;
}

/** A read of one worktree: what it found, and what it walked into and left out, which notices are weighed against. */
export type WorktreeScanned = SourceScan & { readonly seen: WorktreeRead };

/** A session's part in one walk, taken as the walk begins. */
export interface WatcherWalk {
  /** The worktrees the session watches, physical. */
  readonly worktrees: ReadonlySet<string>;
  /** The checkout the session folder is in, physical; undefined when none is. */
  readonly checkout: string | undefined;
  /** The session folder, physical: in `checkout`, which it may be. */
  readonly folder: string;
  /**
   * The folder where Git registers the repository's linked worktrees, physical (`CodeRuleAuditor.worktreeRegistry`):
   * watched, so Git adding a worktree, or one removed and added again at its path, asks for a walk. None when unknown.
   */
  readonly registry?: string | undefined;
  /** Those of `worktrees` the session takes for the first time, or had a subtree of it could not read. */
  readonly firstSight: ReadonlySet<string>;
  /**
   * Those of `firstSight` the session takes for the first time: it holds no read of them yet. One whose subtree it could
   * not read it holds a read of, so it can record a change elsewhere there for the repository (`#recorders`).
   */
  readonly fresh: ReadonlySet<string>;
  /** When the session opened, on the filesystem's clock. */
  readonly startedAt: number;
  /**
   * The checkouts inside its worktrees that it holds a `HEAD` of, physical: those a walk has found a change in. The walk
   * reads theirs as it begins (`InnerCheckouts.heads`); until one does, a checkout there starts no Git process for it.
   */
  readonly heads: ReadonlySet<string>;
  /**
   * The checkouts inside its worktrees that it holds unmerged entries of, physical. The walk reads theirs as it begins
   * even when it kept nothing of their worktree, so the entries a session holds never stay older than the walk.
   */
  readonly unmerged: ReadonlySet<string>;
  /**
   * Reads a worktree for the walk, asking this session which files it reads and which Git ignores; undefined when the
   * session closed meanwhile, and another watching session reads it instead.
   */
  scan(request: WorktreeScan): Promise<WorktreeScanned | undefined>;
  /**
   * Whether the session's record holds the EPERM gap of `path` in the worktree at `root` (physical; `path` relative to it,
   * `.` for it): a look spared there adds no place to the session's record.
   */
  holdsRefusal(root: string, path: string): boolean;
  /**
   * Which of the changes a read found in the worktree at `root` (physical) the session made: one of its tool calls ran in
   * that worktree as each was written. Never rejects.
   */
  claims(root: string, findings: WorktreeFindings): Promise<ReadonlySet<string>>;
  /**
   * Takes what the walk found. `records` holds, by worktree (physical), the changes no watching session made that this
   * session records once for the repository (`WorktreeWalks`). Never rejects: the session records its own failures.
   */
  take(walk: WorktreeWalk, records: WorktreeRecords): Promise<void>;
}

/** A change a read found in a worktree since the read before it: the source's path in the worktree, and its new observation (undefined once removed). */
export interface WorktreeChange {
  readonly path: string;
  readonly after: ObservedSource | undefined;
  /** What the read before saw of it; undefined for a source new since. */
  readonly before: ObservedSource | undefined;
}

/** By worktree (physical), the paths in it of the changes a session records for the repository. */
export type WorktreeRecords = ReadonlyMap<string, ReadonlySet<string>>;

/** A session that watches the other worktrees of its repository. */
export interface WorktreeWatcher {
  /** The filesystem's clock (`filesystemClock`). */
  clock(): Promise<number>;
  /** Called as each walk begins, before it reads anything; undefined once the session has closed or cannot say. */
  start(): WatcherWalk | undefined;
}

/** A session's watch of the shared walk. */
export interface WorktreeWatch {
  /**
   * Asks for a walk, which reads the session's worktrees and those of every session sharing one with them. It starts
   * once the last walk that answered this session's asks has been over for the gap (`WORKTREE_WALK_GAP_MS`), whatever
   * walk another session had; one asked for while a walk runs starts as that walk ends, or once that gap is over.
   */
  request(): void;
  /** Whether a walk the session asked for has yet to be taken. */
  pending(): boolean;
  /** Resolves once no walk the session asked for is timed, running, or still being taken. */
  settled(): Promise<void>;
  /**
   * Stops watching: no walk reads through the session or reaches it again. Once no session watches, it settles when
   * every Git the walk started has exited, so on Windows none holds a worktree after the last session ended.
   */
  close(): Promise<void>;
}

interface Watching {
  readonly watcher: WorktreeWatcher;
  /** Asked for a walk that has yet to begin. */
  requested: boolean;
  /** How many walks it asked for, and how many of those asks the walks it took answered. */
  asked: number;
  answered: number;
  /** When a walk may next begin for its asks: the gap after the last walk that answered one (`WORKTREE_WALK_GAP_MS`). */
  nextWalkAt: number;
  /** Taking walks, one after the other; the newest it has yet to take waits in `waiting`, replacing any older. */
  taking: Promise<void> | undefined;
  waiting: { readonly walk: WatcherWalk; readonly found: WorktreeWalk; readonly answers: number; readonly records: WorktreeRecords } | undefined;
  /** Its part in the walk it is taking now (`taking`): it takes that walk before any that begins meanwhile. */
  takingPart: WatcherWalk | undefined;
  /** The worktrees it watched as the last walk began. */
  worktrees: ReadonlySet<string>;
  /** The checkout its session folder was in, and that folder, as the last walk began (`WatcherWalk`). */
  checkout: string | undefined;
  folder: string | undefined;
  /** `WatcherWalk.registry` as the last walk began. */
  registry: string | undefined;
  closed: boolean;
}

/** A session that stopped watching: its folder, the checkout holding it, and when it stopped once `known` settles. */
interface Departed {
  readonly checkout: string;
  readonly folder: string;
  closedAt: number;
  readonly known: Promise<void>;
}

/** What the last read of a worktree found, kept to hand to sessions while nothing tells of a change there. */
interface Kept {
  readonly findings: Omit<WorktreeFindings, "texts" | "listed" | "inner" | "changes">;
  /** Whether the read left something to try again: a folder it could not list, or a file it could not read. */
  readonly retry: boolean;
  /** The other worktrees and sessions' checkouts inside it that the read left out (`leftOut`), one per line. */
  readonly skipped: string;
  /** The look taken before the read; undefined for a worktree that was not watched, which every walk reads. */
  readonly look: WorktreeLook | undefined;
  /** When the read began (`Date.now()`). */
  readonly readAt: number;
  /** The files its index tracked though Git ignores them, as the read began (`gitTrackedIgnoredReader`). */
  readonly trackedIgnored: string | undefined;
}

/** What Git said of a worktree as a walk began: its `HEAD`, its index's unmerged entries and its force-added files. */
type CheckoutState = Pick<WorktreeFindings, "head" | "unmerged"> & Pick<Kept, "trackedIgnored">;

/** What a walk saw of a worktree before deciding whether to read it. */
type WorktreeState = CheckoutState & {
  readonly observedAt: number;
  readonly skipped: string;
  readonly look: WorktreeLook | undefined;
};

/** What a walk read once, before it decides on any worktree (`#read`), with the sessions it is for. */
interface WalkBegun {
  readonly checkouts: ReadonlyMap<string, CheckoutState>;
  /** For each worktree, what Git said of the checkouts inside it, but `passed` (`#readInner`). */
  readonly inner: ReadonlyMap<string, Omit<InnerCheckouts, "passed">>;
  readonly observedAt: number;
  /** The other worktrees and sessions' checkouts each read leaves out (`leftOut`). */
  readonly skip: ReadonlySet<string>;
  readonly files: WalkFiles;
  readonly walking: ReadonlyMap<Watching, WatcherWalk>;
  /** The worktrees whose checkouts' changed files the walk lists (`firstSeenIn`). */
  readonly firstSeen: ReadonlySet<string>;
}

/**
 * Whether what no notice tells of moved since `kept` was read: the worktrees and sessions' checkouts inside it, what Git
 * says of it (`gitMovedSince`), or what its look saw.
 */
function movedSince(kept: Kept, state: WorktreeState): boolean {
  if (kept.skipped !== state.skipped || gitMovedSince(kept, state)) return true;
  return kept.look === undefined || state.look === undefined || lookMoved(kept.look, state.look);
}

/**
 * Whether `HEAD` moved, or the index changed what a read finds. Its readers hand back the same answer while the index
 * says the same, so a `git status` that rewrote the index to refresh its files' stat data moves nothing. Its unmerged
 * entries are a new map when a conflict began or ended; its force-added files are other text when a file Git ignores
 * became tracked, and a path the index tracks is never ignored, so a file the last read left out may count now. An index
 * that cannot be stamped is a new map each walk, and the worktree is read on every walk, as before notices.
 */
function gitMovedSince(kept: Kept, state: CheckoutState): boolean {
  return kept.findings.head !== state.head || kept.findings.unmerged !== state.unmerged || kept.trackedIgnored !== state.trackedIgnored;
}

const OBSERVED_FIELDS = ["language", "stamp", "digest", "readError", "modifiedAt"] as const;

/** Whether two observations of a source say the same: a walk keeps the earlier, so sessions see no change. */
export function sameObservation(left: ObservedSource, right: ObservedSource): boolean {
  if (!OBSERVED_FIELDS.every(field => left[field] === right[field])) return false;
  // When a file was read is what tells a stamp from one written within the same clock tick; a file not read has none.
  return left.modifiedAt === undefined || left.observedAt === right.observedAt;
}

/** `source` without its text. */
function observed({ text: _text, deleted: _deleted, ...kept }: Source): ObservedSource {
  return kept;
}

/** The snapshot a walk's scan of a worktree makes: the last one when nothing changed, else one keeping each unchanged observation. */
function nextSnapshot(previous: WorktreeSnapshot | undefined, scan: SourceScan): { snapshot: WorktreeSnapshot; texts: Map<string, string> } {
  const sources = new Map<string, ObservedSource>();
  const texts = new Map<string, string>();
  for (const [path, source] of scan.sources) {
    if (source.text !== undefined) texts.set(path, source.text);
    sources.set(path, keptObservation(previous?.sources.get(path), source));
  }
  return { snapshot: previous !== undefined && holdsAll(previous, sources) ? previous : { sources }, texts };
}

/** What changed from `before` to `after`, two snapshots of one worktree: each source whose observation is another, or gone. */
function changesBetween(before: WorktreeSnapshot, after: WorktreeSnapshot): WorktreeChange[] {
  if (before === after) return [];
  const changes: WorktreeChange[] = [];
  for (const [path, source] of after.sources) {
    const was = before.sources.get(path);
    if (was !== source) changes.push({ path, after: source, before: was });
  }
  for (const [path, was] of before.sources) if (!after.sources.has(path)) changes.push({ path, after: undefined, before: was });
  return changes;
}

/** `folder` as a path in the checkout at `root` that holds it (`/`-separated; `""` for the checkout itself). */
function folderIn(root: string, folder: string): string {
  return relative(root, folder).split(sep).join("/");
}

/** Records of a walk a session skips, then the next one's, by worktree: the paths of either. */
function mergedRecords(skipped: WorktreeRecords, next: WorktreeRecords): WorktreeRecords {
  const merged = new Map(skipped);
  for (const [root, paths] of next) merged.set(root, new Set([...(merged.get(root) ?? []), ...paths]));
  return merged;
}

/** The earlier observation of a source when the walk found it the same, else the walk's own, without its text. */
function keptObservation(before: ObservedSource | undefined, source: Source): ObservedSource {
  return before !== undefined && sameObservation(before, source) ? before : observed(source);
}

/** Whether `snapshot` holds exactly these observations. */
function holdsAll(snapshot: WorktreeSnapshot, sources: ReadonlyMap<string, ObservedSource>): boolean {
  if (snapshot.sources.size !== sources.size) return false;
  for (const [path, source] of sources) if (snapshot.sources.get(path) !== source) return false;
  return true;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * What every read of a walk leaves out: the worktrees it reads, which are read on their own, and the checkout of each
 * session taking part, which every other session watches. Each checkout of the repository is one or the other, so a
 * worktree around another reads the same whichever sessions take part: a session never sees a nested worktree's files
 * come and go from its snapshot as other sessions start and close.
 */
function leftOut(roots: ReadonlySet<string>, walks: Iterable<WatcherWalk>): Set<string> {
  const skip = new Set(roots);
  for (const walk of walks) if (walk.checkout !== undefined) skip.add(walk.checkout);
  return skip;
}

/** What a session's walk has the notices watch: its worktrees, and the folder where Git registers them. */
function rootsOf({ worktrees, registry }: { readonly worktrees: ReadonlySet<string>; readonly registry?: string | undefined }): ReadonlySet<string> {
  return registry === undefined ? worktrees : new Set([...worktrees, registry]);
}

/** The innermost of `roots` that `path` lies inside; undefined when it lies in none. A worktree's read leaves out those inside it. */
function worktreeAround(roots: ReadonlySet<string>, path: string): string | undefined {
  let around: string | undefined;
  for (const root of roots) if (path.startsWith(`${root}${sep}`) && (around === undefined || root.length > around.length)) around = root;
  return around;
}

/** The folders of `skip` inside the worktree at `root`, one per line. */
function skippedIn(root: string, skip: ReadonlySet<string>): string {
  return [...skip].filter(folder => folder.startsWith(`${root}${sep}`)).sort().join("\n");
}

/**
 * Whether a read must be tried again: it gave up on a path it met (`SourceScan.gaveUp`: a folder it could not list, or a
 * path gone as it looked, which may have moved where it did not look), or a file's read failed (an error with no stamp).
 */
function retried(scan: SourceScan): boolean {
  if (scan.gaveUp === true) return true;
  for (const source of scan.sources.values()) if (source.readError !== undefined && source.stamp === undefined) return true;
  return false;
}

/** What Git lists as modified or untracked in each of `checkouts`, one Git process each. */
async function listChanged(git: GitReads, checkouts: ReadonlySet<string>): Promise<Map<string, readonly string[] | undefined>> {
  const listed = new Map<string, readonly string[] | undefined>();
  for (const checkout of checkouts) listed.set(checkout, await git.run(signal => gitModifiedOrUntrackedPaths(checkout, { signal })));
  return listed;
}

/**
 * The worktrees whose checkouts' changed files a walk lists (`WorktreeFindings.listed`): each one a session taking part
 * takes for the first time (`WatcherWalk.firstSight`), but where the walk that session is still taking shows it that
 * worktree first too. That walk listed it, and the session takes that walk before this one, which ends its first sight:
 * listing it again for each walk that began meanwhile started a Git process per worktree on every one of them. Should
 * that take not end it, the session lists the worktree itself as it takes this walk (`CodeRuleBridge`).
 */
function firstSeenIn(walking: ReadonlyMap<Watching, WatcherWalk>): Set<string> {
  const roots = new Set<string>();
  for (const [watching, walk] of walking) {
    for (const root of walk.firstSight) if (watching.takingPart?.firstSight.has(root) !== true) roots.add(root);
  }
  return roots;
}

/** `WorktreeFindings.listed` of the worktree at `root`: listed when the walk shows a session it first (`firstSeen`). */
async function listedFor(
  git: GitReads,
  root: string,
  firstSeen: ReadonlySet<string>,
  checkouts: ReadonlySet<string>,
): Promise<Map<string, readonly string[] | undefined>> {
  return firstSeen.has(root) ? await listChanged(git, checkouts) : new Map<string, readonly string[] | undefined>();
}

/**
 * The walks of a repository's other worktrees for every session that watches them: one per repository, shared by the
 * runtimes of an engine service (`RepositoryWorktreeWalks`), or one per session where nothing shares it. Each walk reads
 * each worktree that may have changed once, through one session watching it, and hands what it found, and what it
 * found last of the others, to every session watching them; it keeps the last findings of each worktree some session
 * watches, and drops them, and stops watching it, once none does.
 */
export class WorktreeWalks {
  readonly #gapMs: number;
  readonly #fullWalkMs: number;
  readonly #notices: WorktreeNotices;
  readonly #looks: WorktreeLooks;
  /** Every session watching, in the order they started: a worktree is read through the first that watches it. */
  readonly #watching: Watching[] = [];
  /**
   * The folders of sessions that closed, with the checkout each is in and when it closed on the filesystem's clock, which
   * `known` settles: a change made there before is that session's folder walk's, which recorded it if it was the
   * session's (`#judgedElsewhere`), until a read of the checkout sees past it or no session watches the checkout.
   */
  #departed: Departed[] = [];
  readonly #kept = new Map<string, Kept>();
  /** Worktrees a walk began to read and has not kept findings of: the next walk reads them whatever the notices say. */
  readonly #owed = new Set<string>();
  /**
   * The refusals each worktree's reads withdrew (`WorktreeScan.withdrawn`), by its path, dropped with what was kept of it
   * (`#forget`). A read begun while a session watched it can put back an empty set as the session closes, which no read
   * through a closed session fills (`WatcherWalk.scan`), until the next walk or close drops it.
   */
  readonly #withdrawn = new Map<string, Set<string>>();
  readonly #readUnmerged = gitUnmergedReader();
  readonly #readTrackedIgnored = gitTrackedIgnoredReader();
  /**
   * Every Git the walks start, stopped once no session watches: a walk is never waited for, and on Windows a Git it left
   * running would hold a worktree after the last session ended. A new one serves the next session.
   */
  #git = new GitReads();
  /** The walk running or timed; undefined while no session asks for one. */
  #walk: Promise<void> | undefined;
  /** Starts the timed walk now, to answer an ask whose gap is over; undefined while no walk is timed. */
  #wake: (() => void) | undefined;
  /** Times the walk that reads the worktree whose last read is next due (`WORKTREE_FULL_WALK_MS`). */
  #reread: NodeJS.Timeout | undefined;
  /** Takes the notices every `WORKTREE_NOTICE_MS` while a session watches and a worktree or a registry folder is watched. */
  #pump: NodeJS.Timeout | undefined;
  /** Whether a walk is reading: it takes the notices itself, and asks for the walk they call for as it ends. */
  #walking = false;
  readonly #onWatching: ((watching: boolean) => void) | undefined;

  /**
   * `notices`: none by default, so every worktree is read on every walk; `RepositoryWorktreeWalks` gives each walk the
   * engine's. `looks`: what each walk looks at in a watched worktree (`WorktreeLooks`). `onWatching`: told `true` as a
   * first session starts watching, and `false` as the last stops.
   */
  constructor(options: {
    gapMs?: number;
    fullWalkMs?: number;
    notices?: WorktreeNotices;
    looks?: WorktreeLooks;
    onWatching?: (watching: boolean) => void;
  } = {}) {
    this.#gapMs = options.gapMs ?? WORKTREE_WALK_GAP_MS;
    this.#fullWalkMs = options.fullWalkMs ?? WORKTREE_FULL_WALK_MS;
    this.#notices = options.notices ?? new WorktreeNotices();
    this.#looks = options.looks ?? new WorktreeLooks();
    this.#onWatching = options.onWatching;
  }

  watch(watcher: WorktreeWatcher): WorktreeWatch {
    const watching: Watching = {
      watcher, requested: false, asked: 0, answered: 0, nextWalkAt: 0, taking: undefined, waiting: undefined, takingPart: undefined,
      worktrees: new Set(), checkout: undefined, folder: undefined, registry: undefined, closed: false,
    };
    this.#watching.push(watching);
    if (this.#watching.length === 1) this.#onWatching?.(true);
    return {
      request: () => this.#request(watching),
      pending: () => !watching.closed && watching.answered < watching.asked,
      settled: () => this.#settled(watching),
      close: () => this.#unwatch(watching),
    };
  }

  /** How many observations the snapshots hold, over every worktree a session watches. */
  observations(): number {
    let count = 0;
    for (const { findings } of this.#kept.values()) count += findings.snapshot.sources.size;
    return count;
  }

  /** Resolves once no walk is timed or running, whichever sessions it was for. */
  async idle(): Promise<void> {
    while (this.#walk !== undefined) await this.#walk.catch(() => undefined);
  }

  #request(watching: Watching): void {
    if (watching.closed) return;
    watching.requested = true;
    watching.asked += 1;
    this.#schedule();
  }

  /**
   * Times the next walk for the session asking whose gap ends first, or starts the timed one now when an ask's gap is
   * already over. A walk running is never joined: an ask made meanwhile is the next walk's.
   */
  #schedule(): void {
    const due = this.#dueAt();
    if (this.#walk !== undefined) {
      if (due !== undefined && due <= Date.now()) this.#wake?.();
      return;
    }
    if (due === undefined) return;
    this.#walk = this.#walkWhenDue().finally(() => {
      this.#walk = undefined;
      this.#schedule();
    });
  }

  /** When the next walk may begin: the earliest a session asking for one may have it; undefined while none asks. */
  #dueAt(): number | undefined {
    let due: number | undefined;
    for (const watching of this.#watching) if (watching.requested) due = Math.min(due ?? Infinity, watching.nextWalkAt);
    return due;
  }

  /**
   * Waits until a session asking for a walk may have one, then walks; returns without one once none is left asking. The
   * walk starts when its timer fires, as timed: only a wake before it (an ask already due, a session gone) times it
   * again. However it ends, the reread and the notices are timed again.
   */
  async #walkWhenDue(): Promise<void> {
    try {
      for (let due = this.#dueAt(); due !== undefined && due > Date.now(); due = this.#dueAt()) {
        if (!(await this.#sleep(due - Date.now()))) break;
      }
      if (this.#dueAt() === undefined) return;
      const startedAt = Date.now();
      let askers: readonly Watching[] = [];
      this.#walking = true;
      try {
        askers = await this.#walkOnce();
      } finally {
        this.#walking = false;
        const end = Date.now();
        for (const watching of askers) watching.nextWalkAt = end + Math.max(this.#gapMs, end - startedAt);
        this.#askNoticed();
      }
    } finally {
      this.#armReread();
      this.#armPump();
    }
  }

  /**
   * Times a walk for when the oldest read kept of a watched worktree is `WORKTREE_FULL_WALK_MS` old, so the reread
   * comes though every session watching that worktree is idle. Walks that read every worktree (no period), and
   * worktrees no notice tells of, which every walk reads, need no timer.
   */
  #armReread(): void {
    clearTimeout(this.#reread);
    this.#reread = undefined;
    const trusted = this.#trusted();
    if (this.#fullWalkMs <= 0 || trusted.length === 0) return;
    const oldest = Math.min(...trusted.map(([, kept]) => kept.readAt));
    this.#reread = setTimeout(() => this.#rereadDue(), Math.max(0, oldest + this.#fullWalkMs - Date.now()));
    this.#reread.unref?.();
  }

  /** Each worktree kept whose read was taken while it was watched, so walks read it again only as notices say. */
  #trusted(): Array<[string, Kept]> {
    return [...this.#kept].filter(([, kept]) => kept.look !== undefined);
  }

  /** Has each session watching a worktree whose read is due take part in a walk, which it did not ask for. */
  #rereadDue(): void {
    this.#reread = undefined;
    const due = new Set(this.#trusted().filter(([, kept]) => this.#due(kept)).map(([root]) => root));
    for (const watching of this.#watching) if ([...watching.worktrees].some(root => due.has(root))) watching.requested = true;
    // The walk, or the one running, times the next reread as it ends.
    this.#schedule();
    // Fired before any read was due (a timer may fire a little early): time it again.
    if (this.#walk === undefined) this.#armReread();
  }

  #due(kept: Kept): boolean {
    return Date.now() - kept.readAt >= this.#fullWalkMs;
  }

  /** Takes the notices every `WORKTREE_NOTICE_MS` while a session watches and a worktree or a registry folder is watched, and stops once not. */
  #armPump(): void {
    const pumping = this.#watching.length > 0 && this.#notices.watchingAny();
    if (pumping && this.#pump === undefined) {
      this.#pump = setInterval(() => {
        this.#notices.collect();
        // A walk reading asks for what its notices call for as it ends.
        if (!this.#walking) this.#askNoticed();
        this.#armPump();
      }, WORKTREE_NOTICE_MS);
      this.#pump.unref?.();
    } else if (!pumping && this.#pump !== undefined) {
      clearInterval(this.#pump);
      this.#pump = undefined;
    }
  }

  /**
   * Has each session watching a worktree a notice counted in since its last read began take part in a walk, which it
   * did not ask for: the change reaches every session watching it, once each is past its gap.
   */
  #askNoticed(): void {
    const noticed = this.#notices.noticed();
    if (noticed.size === 0) return;
    for (const watching of this.#watching) if ([...rootsOf(watching)].some(root => noticed.has(root))) watching.requested = true;
    this.#schedule();
  }

  /** Waits `ms`, or until `#wake` is called; resolves whether it was woken first. */
  #sleep(ms: number): Promise<boolean> {
    return new Promise(done => {
      const timer = setTimeout(() => {
        this.#wake = undefined;
        done(false);
      }, ms);
      timer.unref?.();
      this.#wake = () => {
        clearTimeout(timer);
        this.#wake = undefined;
        done(true);
      };
    });
  }

  async #settled(watching: Watching): Promise<void> {
    while (!watching.closed && (watching.answered < watching.asked || watching.taking !== undefined)) {
      await this.#walk?.catch(() => undefined);
      await watching.taking;
    }
  }

  async #unwatch(watching: Watching): Promise<void> {
    if (watching.closed) return;
    watching.closed = true;
    this.#leave(watching);
    watching.requested = false;
    watching.waiting = undefined;
    this.#watching.splice(this.#watching.indexOf(watching), 1);
    this.#forget(this.#watching.map(other => rootsOf(other)));
    // A walk timed for its ask alone is called off; one timed for another session's waits on.
    this.#wake?.();
    if (this.#watching.length > 0) return;
    clearTimeout(this.#reread);
    this.#reread = undefined;
    this.#armPump();
    this.#onWatching?.(false);
    const git = this.#git;
    this.#git = new GitReads();
    await git.stop();
  }

  /**
   * Keeps the folder of a session that stops watching, with when it stopped (`#departed`), in place of an earlier
   * session's of the same folder. A clock that cannot say leaves no entry: a change is then recorded twice at worst.
   */
  #leave(watching: Watching): void {
    const { checkout, folder } = watching;
    if (checkout === undefined || folder === undefined) return;
    this.#departed = this.#departed.filter(other => other.checkout !== checkout || other.folder !== folder);
    const departed: Departed = {
      checkout, folder, closedAt: Infinity,
      known: watching.watcher.clock().then(at => {
        departed.closedAt = at;
      }, () => {
        this.#departed = this.#departed.filter(other => other !== departed);
      }),
    };
    this.#departed.push(departed);
  }

  /**
   * Drops what was kept of each worktree no session watches, and stops watching it, and each folder where Git registers
   * worktrees that no session names.
   */
  #forget(watched: ReadonlyArray<ReadonlySet<string>>): void {
    const roots = new Set(watched.flatMap(worktrees => [...worktrees]));
    for (const held of [this.#kept, this.#owed, this.#withdrawn]) for (const root of held.keys()) if (!roots.has(root)) held.delete(root);
    // No walk reads such a checkout now, and its next read, a first one, offers no change to judge.
    this.#departed = this.#departed.filter(departed => roots.has(departed.checkout));
    this.#notices.retain(roots);
    this.#looks.retain(roots);
  }

  /** One walk, for every session asking and every session sharing a worktree with one; returns the sessions that asked. */
  async #walkOnce(): Promise<Watching[]> {
    const starts = this.#start();
    this.#forget([...starts.values()].map(walk => rootsOf(walk)));
    this.#watchRegistries(starts.values());
    const askers = [...starts.keys()].filter(watching => watching.requested);
    const walking = this.#walkers(starts);
    if (walking.size === 0) return askers;
    // The asks each session had made as the walk began are what this walk answers.
    const answers = new Map([...walking.keys()].map(watching => [watching, watching.asked]));
    const found = await this.#read(walking);
    const records = "error" in found ? new Map<Watching, WorktreeRecords>() : await this.#recorders(starts, walking, found.found);
    for (const [watching, walk] of walking) this.#deliver(watching, { walk, found, answers: answers.get(watching)!, records: records.get(watching) ?? new Map() });
    return askers;
  }

  /**
   * Of the changes each read found, those no session made (`WatcherWalk.claims`), handed to one session watching the
   * worktree to record once for the repository: the first that has taken the worktree before, as only a session that
   * holds the read before it takes a change as one, though a subtree of it was unread (`WatcherWalk.fresh`). None when
   * every session watching it takes it for the first time: the read is their baseline (`#unclaimed` says which sessions
   * are asked). None of a change in a session's folder: that session's own walk judges what changes there, as every
   * session's does in its folder; nor of one made in the folder of a session that closed since, before it closed
   * (`#departed`).
   */
  async #recorders(
    starts: ReadonlyMap<Watching, WatcherWalk>,
    walking: ReadonlyMap<Watching, WatcherWalk>,
    found: ReadonlyMap<string, WorktreeFindings>,
  ): Promise<Map<Watching, WorktreeRecords>> {
    const records = new Map<Watching, Map<string, ReadonlySet<string>>>();
    // When each session that closed did: a change after it is no longer that session's to judge.
    await Promise.all(this.#departed.map(departed => departed.known));
    for (const [root, findings] of found) {
      if (findings.changes.length === 0) continue;
      const recorder = [...walking].find(([, walk]) => walk.worktrees.has(root) && !walk.fresh.has(root))?.[0];
      if (recorder === undefined) continue;
      const unclaimed = await this.#unclaimed(starts, root, findings);
      if (unclaimed.size > 0) records.set(recorder, (records.get(recorder) ?? new Map<string, ReadonlySet<string>>()).set(root, unclaimed));
    }
    this.#pruneDeparted(found);
    return records;
  }

  /**
   * The paths of the changes a read found in the worktree at `root` that no session claims and no session's own folder
   * walk judges (`#judgedElsewhere`). The sessions watching the worktree are asked, and so are those whose folder is in it.
   */
  async #unclaimed(starts: ReadonlyMap<Watching, WatcherWalk>, root: string, findings: WorktreeFindings): Promise<Set<string>> {
    const askable = [...starts.values()].filter(walk => walk.worktrees.has(root) || walk.checkout === root);
    const judged = this.#judgedElsewhere(root);
    const claimed = new Set<string>();
    // A session that cannot say claims nothing: the change is still recorded, once, for the repository.
    for (const walk of askable) for (const path of await walk.claims(root, findings).catch(() => new Set<string>())) claimed.add(path);
    return new Set(findings.changes.filter(change => !claimed.has(change.path) && !judged(change)).map(change => change.path));
  }

  /**
   * Whether a session's own folder walk judges `change`, in the worktree at `root`: it is in the folder of a session open
   * now, as its last walk that could start named it (a session whose start failed this walk is still open), or in that
   * of one that closed since, written before it closed. A removed file has no stamp to tell, so it is the closed
   * session's too.
   */
  #judgedElsewhere(root: string): (change: WorktreeChange) => boolean {
    const folders = this.#watching
      .filter(watching => !watching.closed && watching.checkout === root && watching.folder !== undefined)
      .map(watching => ({ prefix: folderIn(root, watching.folder!), closedAt: Infinity }));
    for (const departed of this.#departed) if (departed.checkout === root) folders.push({ prefix: folderIn(root, departed.folder), closedAt: departed.closedAt });
    return change => folders.some(({ prefix, closedAt }) =>
      (prefix === "" || change.path === prefix || change.path.startsWith(`${prefix}/`))
      && (closedAt === Infinity || (change.after?.modifiedAt ?? -Infinity) <= closedAt));
  }

  /** Forgets each closed session's folder in a checkout a read has seen past its close: no change it offers was made before. */
  #pruneDeparted(found: ReadonlyMap<string, WorktreeFindings>): void {
    this.#departed = this.#departed.filter(departed => (found.get(departed.checkout)?.readFrom ?? -Infinity) <= departed.closedAt);
  }

  /** Each open session's part in the walk beginning now; one that cannot say has its request dropped. */
  #start(): Map<Watching, WatcherWalk> {
    const starts = new Map<Watching, WatcherWalk>();
    for (const watching of [...this.#watching]) {
      let walk: WatcherWalk | undefined;
      try {
        walk = watching.watcher.start();
      } catch {
        walk = undefined;
      }
      if (walk === undefined) {
        // Nothing is left for it to wait for.
        watching.requested = false;
        watching.answered = watching.asked;
        continue;
      }
      watching.worktrees = walk.worktrees;
      watching.checkout = walk.checkout;
      watching.folder = walk.folder;
      watching.registry = walk.registry;
      starts.set(watching, walk);
    }
    return starts;
  }

  /** Watches the folder where Git registers the worktrees, as each session named it as this walk began. */
  #watchRegistries(walks: Iterable<WatcherWalk>): void {
    for (const { registry } of walks) if (registry !== undefined) this.#notices.watchRegistry(registry);
  }

  /**
   * The sessions this walk is for: each that asked for one, and each that watches a worktree one of them does, which
   * takes it too, so no session holds an older snapshot of a worktree than the walk has made. Each takes every
   * worktree it watches.
   */
  #walkers(starts: ReadonlyMap<Watching, WatcherWalk>): Map<Watching, WatcherWalk> {
    const asked = new Set<string>();
    for (const [watching, walk] of starts) if (watching.requested) for (const root of walk.worktrees) asked.add(root);
    const walking = new Map<Watching, WatcherWalk>();
    for (const [watching, walk] of starts) {
      if (!watching.requested && ![...walk.worktrees].some(root => asked.has(root))) continue;
      watching.requested = false;
      walking.set(watching, walk);
    }
    return walking;
  }

  /**
   * Reads each worktree the walk's sessions watch that may have changed, and hands on what was kept of each other. What
   * a read found is kept only of a worktree some session still watches: one whose sessions all closed meanwhile is not
   * held for nobody.
   */
  async #read(walking: ReadonlyMap<Watching, WatcherWalk>): Promise<WorktreeWalk> {
    const roots = new Set([...walking.values()].flatMap(walk => [...walk.worktrees]));
    // As the sessions named their first sights: against the walks they were taking as this one began.
    const firstSeen = firstSeenIn(walking);
    try {
      // Read before the walk, so a move or conflict the walk only partly saw is still one to the next.
      const checkouts = await this.#readCheckouts(roots);
      const inner = await this.#readInner(roots, walking);
      const observedAt = await [...walking.keys()][0]!.watcher.clock();
      const skip = leftOut(roots, walking.values());
      // Every notice the system queued before the walk began counts in it.
      this.#notices.collect();
      const begun: WalkBegun = { checkouts, inner, observedAt, skip, files: this.#looks.begin(), walking, firstSeen };
      const found = new Map<string, WorktreeFindings>();
      for (const root of roots) {
        // Every session watching it closed meanwhile: watching it now would hold it for nobody.
        if (!this.#watched(root)) continue;
        const findings = await this.#find(root, begun);
        if (findings !== undefined) found.set(root, findings);
      }
      return { found, observedAt };
    } catch (error) {
      return { error: errorMessage(error) };
    }
  }

  /**
   * What the walk finds of `root`: what was kept of it while no notice and nothing a notice cannot tell of says it may
   * have changed (`#mustRead`), else what a read of it finds now.
   */
  async #find(root: string, begun: WalkBegun): Promise<WorktreeFindings | undefined> {
    const kept = this.#kept.get(root);
    // Asked first, of every worktree: it starts the watch before a first read.
    const noticed = await this.#notices.changed(root);
    const look = await this.#lookAt(root, begun.files, kept);
    const state: WorktreeState = { ...begun.checkouts.get(root)!, observedAt: begun.observedAt, skipped: skippedIn(root, begun.skip), look };
    const findings = kept !== undefined && !noticed && !this.#mustRead(root, kept, state)
      ? await this.#keptFindings(root, kept, begun.firstSeen)
      : await this.#readAndKeep(root, begun, state);
    return findings === undefined ? undefined : { ...findings, inner: await this.#innerFound(root, findings.checkouts, begun.inner.get(root)!) };
  }

  /**
   * What Git says of the checkouts inside each of `roots` as the walk begins: the unmerged entries of each one its last
   * read passed or a session holds anything of, which cost a `stat` while its index file is as it was, and the `HEAD` of
   * each a session asks for (`WatcherWalk.heads`), which costs a Git process only once a file Git reads it from changed
   * (`readGitHead`). Those a session holds are read even when
   * nothing of their worktree was kept (a walk none of its sessions took part in dropped it), or the session would
   * take the walk against entries and a `HEAD` older than it. A checkout whose entries Git could not list is left out,
   * so the last entries a session took stay its snapshot.
   */
  async #readInner(roots: ReadonlySet<string>, walking: ReadonlyMap<Watching, WatcherWalk>): Promise<Map<string, Omit<InnerCheckouts, "passed">>> {
    const asked = new Set([...walking.values()].flatMap(walk => [...walk.heads]));
    const held = new Map<string, Set<string>>();
    for (const checkout of new Set([...walking.values()].flatMap(walk => [...walk.heads, ...walk.unmerged]))) {
      const root = roots.has(checkout) ? undefined : worktreeAround(roots, checkout);
      if (root !== undefined) held.set(root, (held.get(root) ?? new Set()).add(checkout));
    }
    const inner = new Map<string, Omit<InnerCheckouts, "passed">>();
    const git = this.#git;
    for (const root of roots) {
      const checkouts = new Set([...this.#kept.get(root)?.findings.checkouts ?? [], ...held.get(root) ?? []]);
      checkouts.delete(root);
      inner.set(root, await this.#readCheckoutsInside(git, checkouts, asked));
    }
    return inner;
  }

  /**
   * `#readInner` for one worktree's `checkouts`: the `HEAD` of each one in `asked`, read through the daemon's one HEAD
   * reader like every other walk's, and the unmerged entries of each. One whose `HEAD` names no commit is left out.
   */
  async #readCheckoutsInside(git: GitReads, checkouts: ReadonlySet<string>, asked: ReadonlySet<string>): Promise<Omit<InnerCheckouts, "passed">> {
    const heads = new Map<string, string>();
    const unmerged = new Map<string, ReadonlyMap<string, string>>();
    for (const checkout of checkouts) {
      if (asked.has(checkout)) {
        const head = await git.run(signal => readGitHead(checkout, { signal }));
        if (head === undefined) continue;
        heads.set(checkout, head);
      }
      const entries = await git.run(signal => this.#readUnmerged(checkout, { signal }));
      if (entries !== undefined) unmerged.set(checkout, entries);
    }
    return { heads, unmerged };
  }

  /** `before`, with the unmerged entries now of each checkout inside `root` that `before` had none of: a read passed it first. */
  async #innerFound(root: string, checkouts: ReadonlySet<string>, before: Omit<InnerCheckouts, "passed">): Promise<InnerCheckouts> {
    const passed = new Map<string, ReadonlyMap<string, string>>();
    for (const checkout of checkouts) {
      if (checkout === root || before.unmerged.has(checkout)) continue;
      const entries = await this.#git.run(signal => this.#readUnmerged(checkout, { signal }));
      if (entries !== undefined) passed.set(checkout, entries);
    }
    return { ...before, passed };
  }

  /** The look at a watched `root`; a worktree no notice can tell of is read on every walk: nothing it would look at decides that. */
  async #lookAt(root: string, files: WalkFiles, kept: Kept | undefined): Promise<WorktreeLook | undefined> {
    return this.#notices.watching(root) ? await this.#looks.look(this.#git, files, root, kept?.findings.checkouts ?? []) : undefined;
  }

  /**
   * Whether `root` is read though no notice named it: its last read did not finish, left something to try again or is
   * `WORKTREE_FULL_WALK_MS` old, or what no notice tells of moved (`movedSince`).
   */
  #mustRead(root: string, kept: Kept, state: WorktreeState): boolean {
    return this.#owed.has(root) || kept.retry || this.#due(kept) || movedSince(kept, state);
  }

  /** Reads `root` and keeps what the read found while a session watches it; undefined when no session could read it. */
  async #readAndKeep(root: string, begun: WalkBegun, state: WorktreeState): Promise<Omit<WorktreeFindings, "inner"> | undefined> {
    this.#owed.add(root);
    this.#notices.reading(root);
    const readAt = Date.now();
    const result = await this.#readWorktree(root, begun, state);
    if (result === undefined) return undefined;
    const { texts: _texts, listed: _listed, changes: _changes, ...findings } = result.findings;
    if (this.#watched(root)) {
      this.#kept.set(root, {
        findings, retry: retried(result.scan), skipped: state.skipped, look: state.look, readAt, trackedIgnored: state.trackedIgnored,
      });
      this.#owed.delete(root);
    }
    return result.findings;
  }

  /** What a walk hands on of a worktree it did not read: what was kept, no text, and Git's listing for a session's first sight. */
  async #keptFindings(root: string, kept: Kept, firstSeen: ReadonlySet<string>): Promise<Omit<WorktreeFindings, "inner">> {
    return { ...kept.findings, texts: new Map(), listed: await listedFor(this.#git, root, firstSeen, kept.findings.checkouts), changes: [] };
  }

  /** Whether an open session watched `root` as the last walk began. */
  #watched(root: string): boolean {
    return this.#watching.some(watching => watching.worktrees.has(root));
  }

  async #readCheckouts(roots: ReadonlySet<string>): Promise<Map<string, CheckoutState>> {
    const checkouts = new Map<string, CheckoutState>();
    const git = this.#git;
    for (const root of roots) {
      const head = await git.run(signal => readGitHead(root, { signal }));
      if (head === undefined) {
        checkouts.set(root, { head, unmerged: undefined, trackedIgnored: undefined });
        continue;
      }
      const unmerged = await git.run(signal => this.#readUnmerged(root, { signal }));
      const trackedIgnored = await git.run(signal => this.#readTrackedIgnored(root, { signal }));
      checkouts.set(root, { head, unmerged, trackedIgnored });
    }
    return checkouts;
  }

  /** One worktree, read through the first session watching it that stays open while it reads. */
  async #readWorktree(
    root: string,
    begun: WalkBegun,
    read: Pick<WorktreeFindings, "head" | "unmerged"> & { readonly observedAt: number },
  ): Promise<{ readonly findings: Omit<WorktreeFindings, "inner">; readonly scan: WorktreeScanned } | undefined> {
    const watchers = [...begun.walking.values()].filter(walk => walk.worktrees.has(root));
    const previous = this.#kept.get(root)?.findings.snapshot;
    const request: WorktreeScan = {
      root,
      ...this.#lastRead(root),
      held: path => watchers.every(walk => walk.holdsRefusal(root, path)),
      withdrawn: this.#withdrawnIn(root),
      startedAt: Math.min(...watchers.map(walk => walk.startedAt)),
      observedAt: read.observedAt,
      skip: begun.skip,
    };
    for (const walk of watchers) {
      const scan = await walk.scan(request);
      if (scan === undefined) continue;
      this.#notices.read(root, scan.seen);
      const { snapshot, texts } = nextSnapshot(previous, scan);
      const changes = previous === undefined ? [] : changesBetween(previous, snapshot);
      const listed = await listedFor(this.#git, root, begun.firstSeen, scan.checkouts);
      const findings = {
        snapshot, texts, head: read.head, unmerged: read.unmerged, checkouts: scan.checkouts, listed,
        failedRoots: scan.failedRoots, errors: scan.errors, unread: scan.unread, readFrom: read.observedAt, changes,
      };
      return { findings, scan };
    }
    return undefined;
  }

  /** The refusals the reads of `root` withdrew (`WorktreeScan.withdrawn`): an empty set before its first read. */
  #withdrawnIn(root: string): Set<string> {
    const withdrawn = this.#withdrawn.get(root) ?? new Set<string>();
    this.#withdrawn.set(root, withdrawn);
    return withdrawn;
  }

  /** What a read of `root` asks with of what the last read kept (`WorktreeScan`): nothing for a first read. */
  #lastRead(root: string): Pick<WorktreeScan, "previous" | "captureChanges" | "baselineRoots"> {
    const kept = this.#kept.get(root)?.findings;
    if (kept === undefined) return { previous: new Map(), captureChanges: false, baselineRoots: new Set() };
    return { previous: kept.snapshot.sources, captureChanges: true, baselineRoots: kept.failedRoots };
  }

  #deliver(watching: Watching, delivery: NonNullable<Watching["waiting"]>): void {
    if (watching.closed) return;
    // A session still taking an earlier walk takes only the newest after it: what changed since its snapshot is all there,
    // and so is what it records of the walk it skips.
    watching.waiting = watching.waiting === undefined ? delivery : { ...delivery, records: mergedRecords(watching.waiting.records, delivery.records) };
    watching.taking ??= this.#takeAll(watching);
  }

  async #takeAll(watching: Watching): Promise<void> {
    try {
      for (let next = watching.waiting; next !== undefined && !watching.closed; next = watching.waiting) {
        watching.waiting = undefined;
        watching.takingPart = next.walk;
        await next.walk.take(next.found, next.records).catch(() => undefined);
        watching.answered = Math.max(watching.answered, next.answers);
      }
    } finally {
      watching.taking = undefined;
      watching.takingPart = undefined;
    }
  }
}

/**
 * One `WorktreeWalks` per repository, for the sessions of an engine service's runtimes or of one runtime. Sessions of
 * one repository share its walk; sessions of two repositories never share one, so a slow walk of one repository's
 * worktrees never holds back what another repository's sessions record. Every walk takes its notices from one engine
 * watch (`treeWatch`), which `close` releases; without one, every walk reads every worktree.
 */
export class RepositoryWorktreeWalks {
  readonly #options: { gapMs?: number; fullWalkMs?: number };
  readonly #treeWatch: SharedTreeWatch | undefined;
  /**
   * By repository, the walk its sessions watch, dropped once none does: nothing of an idle walk is worth keeping, and
   * a folder the catalog does not know is a repository of its own.
   */
  readonly #walks = new Map<string, WorktreeWalks>();

  constructor(options: { gapMs?: number; fullWalkMs?: number; treeWatch?: TreeWatch | undefined } = {}) {
    const { treeWatch, ...walks } = options;
    this.#options = walks;
    this.#treeWatch = treeWatch === undefined ? undefined : new SharedTreeWatch(treeWatch);
  }

  /** The walk of the repository `repository` names, the same for every checkout of it (its clone id). */
  of(repository: string): WorktreeWalks {
    const known = this.#walks.get(repository);
    if (known !== undefined) return known;
    const walks: WorktreeWalks = new WorktreeWalks({
      ...this.#options,
      notices: new WorktreeNotices({ treeWatch: this.#treeWatch?.user() }),
      // A session handed this walk before it was dropped brings it back while no other took its place.
      onWatching: watching => {
        if (!watching && this.#walks.get(repository) === walks) this.#walks.delete(repository);
        else if (watching && !this.#walks.has(repository)) this.#walks.set(repository, walks);
      },
    });
    this.#walks.set(repository, walks);
    return walks;
  }

  /** Releases the engine watch and every handle it holds; the sessions, closed first, have let go of their worktrees. */
  close(): void {
    this.#treeWatch?.close();
  }
}

/**
 * A session's view of the other worktrees: the snapshot of each it took last from the shared walk, and the sources it saw
 * gone since though that snapshot holds them. Where the session saw a source otherwise, it keeps that observation itself
 * (`CodeRuleBridge`), until a snapshot it takes agrees with it.
 */
export class WorktreeView {
  /** By the worktree's path relative to the session folder (`/`-separated). */
  readonly #snapshots = new Map<string, WorktreeSnapshot>();
  readonly #gone = new Set<string>();

  /** The snapshot taken of the worktree at `root` (relative to the session folder). */
  snapshot(root: string): WorktreeSnapshot | undefined {
    return this.#snapshots.get(root);
  }

  /** The innermost worktree taken that holds `path`, and the path in it; undefined outside them. */
  locate(path: string): { readonly root: string; readonly key: string; readonly snapshot: WorktreeSnapshot } | undefined {
    if (this.#snapshots.size === 0) return undefined;
    for (let slash = path.lastIndexOf("/"); slash > 0; slash = path.lastIndexOf("/", slash - 1)) {
      const root = path.slice(0, slash);
      const snapshot = this.#snapshots.get(root);
      if (snapshot !== undefined) return { root, key: path.slice(slash + 1), snapshot };
    }
    return undefined;
  }

  /** What the snapshot taken holds of `path`, as the session names it, unless the session saw it gone since. */
  get(path: string): ObservedSource | undefined {
    const source = this.#held(path);
    return source === undefined || this.#gone.has(path) ? undefined : { ...source, path };
  }

  /** Whether the snapshot taken holds `source` at `path` as it is, and the session has not seen it gone. */
  holds(path: string, source: ObservedSource): boolean {
    const held = this.#held(path);
    return held !== undefined && !this.#gone.has(path) && sameObservation(held, source);
  }

  /** Marks `path` gone where the snapshot taken holds it; a path no snapshot holds needs no mark. */
  hide(path: string): void {
    if (this.#held(path) !== undefined) this.#gone.add(path);
  }

  unhide(path: string): void {
    this.#gone.delete(path);
  }

  isHidden(path: string): boolean {
    return this.#gone.has(path);
  }

  /** The paths under the worktree at `root` that the session saw gone. */
  hidden(root: string): string[] {
    return [...this.#gone].filter(path => path.startsWith(`${root}/`));
  }

  take(root: string, snapshot: WorktreeSnapshot): void {
    this.#snapshots.set(root, snapshot);
    // A mark the new snapshot no longer needs goes: the source is gone from it too.
    for (const path of this.hidden(root)) if (this.#held(path) === undefined) this.#gone.delete(path);
  }

  /** Drops the worktrees at `roots`, which took their files with them. */
  forget(roots: ReadonlySet<string>): void {
    for (const root of roots) this.#snapshots.delete(root);
    for (const path of this.#gone) if ([...roots].some(root => path.startsWith(`${root}/`))) this.#gone.delete(path);
  }

  clear(): void {
    this.#snapshots.clear();
    this.#gone.clear();
  }

  #held(path: string): ObservedSource | undefined {
    const located = this.locate(path);
    return located?.snapshot.sources.get(located.key);
  }
}
