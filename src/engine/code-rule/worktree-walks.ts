/**
 * The walk of a repository's other worktrees, shared by every session that watches them. A shell write in another
 * worktree is seen only by walking it, and every session of the repository walked them all for itself: each kept an
 * observation of every file of every other worktree, so with sessions in many worktrees the daemon's memory, and the time
 * it spent walking, grew with the sessions times the worktrees times their files. Here one walk reads each worktree
 * once, for every session watching it, and keeps what it saw as a snapshot; each session takes from a walk only the
 * sources that changed since the snapshot it took last (`CodeRuleBridge`).
 *
 * A walk reads only the worktrees that may have changed since it last read them: each a change notice named
 * (`WorktreeNotices`) or that cannot be watched, and each whose `HEAD`, unmerged entries or nested worktrees moved, which
 * every walk reads. It hands each session what it saw of the others last time. Reading them all stat'ed every file of
 * every worktree on each walk: with 190 worktrees, about 5 s of the daemon's main thread a walk.
 */
import { sep } from "node:path";
import { gitHead, gitModifiedOrUntrackedPaths, gitUnmergedReader } from "../git.js";
import type { ObservedSource, ScanError, Source, SourceScan } from "./code-rule-bridge.js";
import { noticesTaken, WatchedWorktrees, type WorktreeNotices, type WorktreeRead } from "./worktree-notices.js";

/**
 * The least time between walks of the other worktrees, which take time per worktree and nothing waits for; a walk that
 * took longer than this waits as long as it took, so walking never takes more than half the time.
 */
export const WORKTREE_WALK_GAP_MS = 5_000;

/**
 * How long walks trust change notices: the first walk after this reads every worktree, so a change no notice told of
 * (one the system lost without saying so) is found then.
 */
export const WORKTREE_FULL_WALK_MS = 600_000;

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
  /**
   * For each of `checkouts`, the files its Git lists as modified or untracked (`gitModifiedOrUntrackedPaths`), relative
   * to it: listed once for every session that takes the worktree for the first time in this walk, and only then.
   */
  readonly listed: ReadonlyMap<string, readonly string[]>;
  /** Subtrees the walk could not read. */
  readonly failedRoots: ReadonlySet<string>;
  readonly errors: readonly ScanError[];
  /** The unlisted files it found too large to read, with their stamps. */
  readonly unread: ReadonlyMap<string, { readonly stamp: string; readonly readError: string }>;
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
  /** Those of `worktrees` the session takes for the first time, or had a subtree of it could not read. */
  readonly firstSight: ReadonlySet<string>;
  /** When the session opened, on the filesystem's clock. */
  readonly startedAt: number;
  /**
   * Reads a worktree for the walk, asking this session which files it reads and which Git ignores; undefined when the
   * session closed meanwhile, and another watching session reads it instead.
   */
  scan(request: WorktreeScan): Promise<WorktreeScanned | undefined>;
  /** Takes what the walk found. Never rejects: the session records its own failures. */
  take(walk: WorktreeWalk): Promise<void>;
}

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
   * once the last walk has been over for the gap; one asked for while a walk runs runs again after it.
   */
  request(): void;
  /** Whether a walk the session asked for has yet to be taken. */
  pending(): boolean;
  /** Resolves once no walk the session asked for is timed, running, or still being taken. */
  settled(): Promise<void>;
  /** Stops watching: no walk reads through the session or reaches it again. */
  close(): void;
}

interface Watching {
  readonly watcher: WorktreeWatcher;
  /** Asked for a walk that has yet to begin. */
  requested: boolean;
  /** How many walks it asked for, and how many of those asks the walks it took answered. */
  asked: number;
  answered: number;
  /** Taking walks, one after the other; the newest it has yet to take waits in `waiting`, replacing any older. */
  taking: Promise<void> | undefined;
  waiting: { readonly walk: WatcherWalk; readonly found: WorktreeWalk; readonly answers: number } | undefined;
  /** The worktrees it watched as the last walk began. */
  worktrees: ReadonlySet<string>;
  closed: boolean;
}

/** What the last read of a worktree found, kept to hand to sessions while nothing tells of a change there. */
interface Kept {
  readonly findings: Omit<WorktreeFindings, "texts" | "listed">;
  /** Whether the read left something to try again: a folder it could not list, or a file it could not read. */
  readonly retry: boolean;
  /** The other worktrees and sessions' checkouts inside it that the read left out (`leftOut`), one per line. */
  readonly skipped: string;
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

/** The folders of `skip` inside the worktree at `root`, one per line. */
function skippedIn(root: string, skip: ReadonlySet<string>): string {
  return [...skip].filter(folder => folder.startsWith(`${root}${sep}`)).sort().join("\n");
}

function sameEntries(left: ReadonlyMap<string, string> | undefined, right: ReadonlyMap<string, string> | undefined): boolean {
  if (left === right) return true;
  if (left === undefined || right === undefined || left.size !== right.size) return false;
  for (const [path, entries] of left) if (right.get(path) !== entries) return false;
  return true;
}

/** Whether a read must be tried again: a folder it could not list, or a file whose read failed (an error with no stamp). */
function retried(scan: SourceScan): boolean {
  if (scan.failedRoots.size > 0) return true;
  for (const source of scan.sources.values()) if (source.readError !== undefined && source.stamp === undefined) return true;
  return false;
}

/** What Git lists as modified or untracked in each of `checkouts`, one Git process each. */
async function listChanged(checkouts: ReadonlySet<string>): Promise<Map<string, readonly string[]>> {
  const listed = new Map<string, readonly string[]>();
  for (const checkout of checkouts) listed.set(checkout, await gitModifiedOrUntrackedPaths(checkout));
  return listed;
}

/** `WorktreeFindings.listed` of the worktree at `root`: listed when one of `watchers` takes it for the first time. */
async function listedFor(root: string, watchers: readonly WatcherWalk[], checkouts: ReadonlySet<string>): Promise<Map<string, readonly string[]>> {
  return watchers.some(watcher => watcher.firstSight.has(root)) ? await listChanged(checkouts) : new Map<string, readonly string[]>();
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
  /** Every session watching, in the order they started: a worktree is read through the first that watches it. */
  readonly #watching: Watching[] = [];
  readonly #kept = new Map<string, Kept>();
  /** Worktrees a walk began to read and has not kept findings of: the next walk reads them whatever the notices say. */
  readonly #owed = new Set<string>();
  readonly #readUnmerged = gitUnmergedReader();
  /** The walk running or timed; undefined while none is due. */
  #walk: Promise<void> | undefined;
  #nextWalkAt = 0;
  #fullWalkAt = Number.NEGATIVE_INFINITY;

  constructor(options: { gapMs?: number; fullWalkMs?: number; notices?: WorktreeNotices } = {}) {
    this.#gapMs = options.gapMs ?? WORKTREE_WALK_GAP_MS;
    this.#fullWalkMs = options.fullWalkMs ?? WORKTREE_FULL_WALK_MS;
    this.#notices = options.notices ?? new WatchedWorktrees();
  }

  watch(watcher: WorktreeWatcher): WorktreeWatch {
    const watching: Watching = {
      watcher, requested: false, asked: 0, answered: 0, taking: undefined, waiting: undefined, worktrees: new Set(), closed: false,
    };
    this.#watching.push(watching);
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

  #schedule(): void {
    if (this.#walk !== undefined || !this.#watching.some(watching => watching.requested)) return;
    const delay = Math.max(0, this.#nextWalkAt - Date.now());
    this.#walk = this.#walkAfter(delay).finally(() => {
      this.#walk = undefined;
      this.#schedule();
    });
  }

  async #walkAfter(delay: number): Promise<void> {
    if (delay > 0) await new Promise(wake => { setTimeout(wake, delay).unref?.(); });
    const startedAt = Date.now();
    try {
      await this.#walkOnce();
    } finally {
      this.#nextWalkAt = Date.now() + Math.max(this.#gapMs, Date.now() - startedAt);
    }
  }

  async #settled(watching: Watching): Promise<void> {
    while (!watching.closed && (watching.answered < watching.asked || watching.taking !== undefined)) {
      await this.#walk?.catch(() => undefined);
      await watching.taking;
    }
  }

  #unwatch(watching: Watching): void {
    if (watching.closed) return;
    watching.closed = true;
    watching.requested = false;
    watching.waiting = undefined;
    this.#watching.splice(this.#watching.indexOf(watching), 1);
    this.#forget(this.#watching.map(other => other.worktrees));
  }

  /** Drops what was kept of each worktree no session watches, and stops watching it. */
  #forget(watched: ReadonlyArray<ReadonlySet<string>>): void {
    const roots = new Set(watched.flatMap(worktrees => [...worktrees]));
    for (const root of this.#kept.keys()) if (!roots.has(root)) this.#kept.delete(root);
    for (const root of this.#owed) if (!roots.has(root)) this.#owed.delete(root);
    this.#notices.retain(roots);
  }

  async #walkOnce(): Promise<void> {
    const starts = this.#start();
    this.#forget([...starts.values()].map(walk => walk.worktrees));
    const walking = this.#walkers(starts);
    if (walking.size === 0) return;
    // The asks each session had made as the walk began are what this walk answers.
    const answers = new Map([...walking.keys()].map(watching => [watching, watching.asked]));
    const found = await this.#read(walking);
    for (const [watching, walk] of walking) this.#deliver(watching, { walk, found, answers: answers.get(watching)! });
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
      starts.set(watching, walk);
    }
    return starts;
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
   * a read found is kept only once the whole walk has read, and only of a worktree some session still watches: one
   * whose sessions all closed meanwhile is not held for nobody.
   */
  async #read(walking: ReadonlyMap<Watching, WatcherWalk>): Promise<WorktreeWalk> {
    const roots = new Set([...walking.values()].flatMap(walk => [...walk.worktrees]));
    try {
      // Read before the walk, so a move or conflict the walk only partly saw is still one to the next.
      const checkouts = await this.#readCheckouts(roots);
      const observedAt = await [...walking.keys()][0]!.watcher.clock();
      const skip = leftOut(roots, walking.values());
      // Every notice the system queued before the walk began counts in it.
      await noticesTaken();
      const full = this.#fullWalkDue();
      const found = new Map<string, WorktreeFindings>();
      const read = new Map<string, Kept>();
      for (const root of roots) {
        const state = { ...checkouts.get(root)!, observedAt, skipped: skippedIn(root, skip) };
        // Asked first, of every worktree: it starts the watch before a first read, and a notice from now on counts.
        const noticed = this.#notices.changed(root);
        const kept = this.#kept.get(root);
        if (kept !== undefined && !noticed && !full && !this.#mustRead(root, kept, state)) {
          found.set(root, await this.#keptFindings(root, kept, walking));
          continue;
        }
        this.#owed.add(root);
        const result = await this.#readWorktree(root, skip, walking, state);
        if (result === undefined) continue;
        found.set(root, result.findings);
        const { texts: _texts, listed: _listed, ...findings } = result.findings;
        read.set(root, { findings, retry: retried(result.scan), skipped: state.skipped });
      }
      for (const [root, kept] of read) {
        if (!this.#watched(root)) continue;
        this.#kept.set(root, kept);
        this.#owed.delete(root);
      }
      return { found, observedAt };
    } catch (error) {
      return { error: errorMessage(error) };
    }
  }

  /** Whether this walk is the one that reads every worktree again (`WORKTREE_FULL_WALK_MS`). */
  #fullWalkDue(): boolean {
    const now = Date.now();
    if (now - this.#fullWalkAt < this.#fullWalkMs) return false;
    this.#fullWalkAt = now;
    return true;
  }

  /**
   * Whether `root` is read though no notice named it: its last read did not finish or left something to try again, or
   * what no notice tells of moved: its `HEAD`, its unmerged entries, or the worktrees and sessions' checkouts inside it.
   */
  #mustRead(root: string, kept: Kept, state: Pick<WorktreeFindings, "head" | "unmerged"> & { readonly skipped: string }): boolean {
    if (this.#owed.has(root) || kept.retry || kept.skipped !== state.skipped) return true;
    return kept.findings.head !== state.head || !sameEntries(kept.findings.unmerged, state.unmerged);
  }

  /** What a walk hands on of a worktree it did not read: what was kept, no text, and Git's listing for a session's first sight. */
  async #keptFindings(root: string, kept: Kept, walking: ReadonlyMap<Watching, WatcherWalk>): Promise<WorktreeFindings> {
    const watchers = [...walking.values()].filter(walk => walk.worktrees.has(root));
    return { ...kept.findings, texts: new Map(), listed: await listedFor(root, watchers, kept.findings.checkouts) };
  }

  /** Whether an open session watched `root` as the last walk began. */
  #watched(root: string): boolean {
    return this.#watching.some(watching => watching.worktrees.has(root));
  }

  async #readCheckouts(roots: ReadonlySet<string>): Promise<Map<string, Pick<WorktreeFindings, "head" | "unmerged">>> {
    const checkouts = new Map<string, Pick<WorktreeFindings, "head" | "unmerged">>();
    for (const root of roots) {
      const head = await gitHead(root);
      checkouts.set(root, { head, unmerged: head === undefined ? undefined : await this.#readUnmerged(root) });
    }
    return checkouts;
  }

  /** One worktree, read through the first session watching it that stays open while it reads. */
  async #readWorktree(
    root: string,
    skip: ReadonlySet<string>,
    walking: ReadonlyMap<Watching, WatcherWalk>,
    read: Pick<WorktreeFindings, "head" | "unmerged"> & { readonly observedAt: number },
  ): Promise<{ readonly findings: WorktreeFindings; readonly scan: WorktreeScanned } | undefined> {
    const watchers = [...walking.values()].filter(walk => walk.worktrees.has(root));
    const previous = this.#kept.get(root)?.findings.snapshot;
    const request: WorktreeScan = {
      root,
      previous: previous?.sources ?? new Map(),
      captureChanges: previous !== undefined,
      startedAt: Math.min(...watchers.map(walk => walk.startedAt)),
      observedAt: read.observedAt,
      skip,
    };
    for (const walk of watchers) {
      const scan = await walk.scan(request);
      if (scan === undefined) continue;
      this.#notices.read(root, scan.seen);
      const { snapshot, texts } = nextSnapshot(previous, scan);
      const listed = await listedFor(root, watchers, scan.checkouts);
      const findings = {
        snapshot, texts, head: read.head, unmerged: read.unmerged, checkouts: scan.checkouts, listed,
        failedRoots: scan.failedRoots, errors: scan.errors, unread: scan.unread,
      };
      return { findings, scan };
    }
    return undefined;
  }

  #deliver(watching: Watching, delivery: NonNullable<Watching["waiting"]>): void {
    if (watching.closed) return;
    // A session still taking an earlier walk takes only the newest after it: what changed since its snapshot is all there.
    watching.waiting = delivery;
    watching.taking ??= this.#takeAll(watching);
  }

  async #takeAll(watching: Watching): Promise<void> {
    try {
      for (let next = watching.waiting; next !== undefined && !watching.closed; next = watching.waiting) {
        watching.waiting = undefined;
        await next.walk.take(next.found).catch(() => undefined);
        watching.answered = Math.max(watching.answered, next.answers);
      }
    } finally {
      watching.taking = undefined;
    }
  }
}

/**
 * One `WorktreeWalks` per repository, for the sessions of an engine service's runtimes or of one runtime. Sessions of
 * one repository share its walk; sessions of two repositories never share one, so a slow walk of one repository's
 * worktrees never holds back what another repository's sessions record.
 */
export class RepositoryWorktreeWalks {
  readonly #options: { gapMs?: number };
  /** Kept for the service's life: an idle walk holds no snapshot, and there is one per repository sessions opened in. */
  readonly #walks = new Map<string, WorktreeWalks>();

  constructor(options: { gapMs?: number } = {}) {
    this.#options = options;
  }

  /** The walk of the repository `repository` names, the same for every checkout of it (its clone id). */
  of(repository: string): WorktreeWalks {
    let walks = this.#walks.get(repository);
    if (walks === undefined) {
      walks = new WorktreeWalks(this.#options);
      this.#walks.set(repository, walks);
    }
    return walks;
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
