/**
 * The operating system's change notices for the walk of a repository's other worktrees (`WorktreeWalks`): which
 * worktrees may hold a change since a walk last read them, so a walk reads only those and keeps what it saw of the rest.
 *
 * The notices come from the engine's file-system watch (`TreeWatch`): one watch of each worktree's whole tree on macOS
 * (FSEvents) and Windows (ReadDirectoryChangesW, holding an oplock so a rename or removal of the worktree is never
 * blocked), and on Linux one inotify watch per folder the last read walked into, so a folder Git ignores is never
 * watched. A worktree the engine cannot watch (a network or WSL path, past Linux's watch budget, Windows with no oplock,
 * or an engine built before the watch) is read on every walk, as before notices; one that could not be watched is tried
 * again after `WATCH_RETRY_MS`.
 *
 * A notice counts unless it names something the last read saw and left out: a path inside a `.git` folder, the times of
 * a `.git` entry or of a folder the read walked (each change among a folder's entries comes as a notice of its own), or
 * a path in or under what the read did not walk or observe (an ignored file or folder, another worktree). A notice the
 * system says it lost, a watch that ended, and every notice that arrives while a read runs count.
 */
import { lstat } from "node:fs/promises";
import type { TreeNotice, TreeWatch } from "./tree-watch.js";

/** What a walk's read of a worktree saw, by paths relative to the worktree (`/`-separated, `""` for the worktree itself). */
export interface WorktreeRead {
  /** Each folder the read walked into. */
  readonly folders: ReadonlySet<string>;
  /** Each file and folder the read found and neither observed nor walked into: ignored, or another worktree. */
  readonly leftOut: ReadonlySet<string>;
}

/** A worktree that could not be watched is tried again after this long, read on every walk meanwhile. */
export const WATCH_RETRY_MS = 600_000;
/**
 * A worktree whose watch ended is watched again no sooner than this after, read whole on every walk meanwhile, unless
 * another folder than the one it watched now stands at its path (`folderAt`): a worktree made again there is watched
 * as the next walk reads it, so a write after that read is a notice. A watch ends when the worktree or a folder above
 * it is renamed, moved or deleted (on Windows, when the path then holds no folder or another, or after a run of renames:
 * the engine's `REOPENS` within `REOPEN_WINDOW`), or on a watch error. Watching the same folder again at once would
 * chase a worktree still being moved, its watch ending again on every walk.
 */
export const REWATCH_MS = 5_000;

/** At most this many paths are kept of the notices of one root queued for one walk, as the engine's watch keeps per take; past it they are lost. */
export const NOTICE_PATHS_CAP = 16_384;

/**
 * Whether a notice of `event` (`change`: a file's text, size or attributes; `rename`: an entry created, deleted or
 * renamed) for `path` (relative to the worktree) names something the last read cared about.
 */
export function noticeCounts(seen: WorktreeRead, event: "change" | "rename", path: string): boolean {
  const parts = path.split("/");
  const git = parts.indexOf(".git");
  if (git >= 0 && underOrTimesOf(parts, git, event)) return false;
  if (inLeftOut(seen, event, parts)) return false;
  return event === "rename" || !seen.folders.has(path);
}

/**
 * Whether a notice of `event` for the path of `parts` names something under the path of `parts` up to `index`, or the
 * times of that path itself (a `change`); a `rename` of that path itself is neither.
 */
function underOrTimesOf(parts: readonly string[], index: number, event: "change" | "rename"): boolean {
  return index < parts.length - 1 || event !== "rename";
}

/**
 * Whether a notice of `event` for the path of `parts` names the times of, or anything under, what the read left out.
 * What the read left out may come or go (a file where an ignored folder was is no longer ignored); its own times and
 * anything under it do not matter.
 */
function inLeftOut(seen: WorktreeRead, event: "change" | "rename", parts: readonly string[]): boolean {
  let prefix = "";
  for (const [index, part] of parts.entries()) {
    prefix = index === 0 ? part : `${prefix}/${part}`;
    if (seen.leftOut.has(prefix) && underOrTimesOf(parts, index, event)) return true;
  }
  return false;
}

/**
 * The folder at `root`: its device, file number and birth time (a new folder on Linux may get the file number of one
 * just deleted); `""` when none is there. Read through Node's thread pool, so a path that does not answer (a hung
 * network share) holds no walk of another worktree and no event in the daemon's thread.
 */
export async function folderAt(root: string): Promise<string> {
  try {
    const info = await lstat(root, { bigint: true });
    return info.isDirectory() ? `${info.dev}:${info.ino}:${info.birthtimeNs}` : "";
  } catch {
    return "";
  }
}

/** Whether `now`, the folder at a worktree's path, is another than `before`, the one an ended watch watched (`folderAt`). */
function remade(before: string, now: string): boolean {
  return now !== "" && now !== before;
}

/** Whether any path a notice names counts against `seen` (`noticeCounts`). */
function counts(seen: WorktreeRead, notice: TreeNotice): boolean {
  return notice.renamed.some(path => noticeCounts(seen, "rename", path)) || notice.changed.some(path => noticeCounts(seen, "change", path));
}

interface Watched {
  /** The folders the watch follows (Linux), as last given to `TreeWatch.watch`. */
  folders: ReadonlySet<string>;
  /** What the last finished read saw; undefined while a read runs or before the first, when every notice counts. */
  seen: WorktreeRead | undefined;
  changed: boolean;
  /** The folder at the worktree's path as the watch began (`folderAt`). */
  readonly folder: string;
}

/** A watch that ended: when, and the folder it watched (`folderAt`). */
interface Ended {
  readonly at: number;
  readonly folder: string;
}

/**
 * Which worktrees a walk must read again (`WorktreeWalks`). A walk takes the queued notices (`collect`) and asks
 * `changed` of each worktree; one it reads it marks with `reading` as the read begins and `read` as it ends. Between the
 * two every notice counts, and outside them a notice counts unless the last read's `WorktreeRead` says otherwise.
 */
export class WorktreeNotices {
  readonly #watch: TreeWatch | undefined;
  /** Whether the watch follows the folders a read walked (Linux), rather than each worktree's whole tree. */
  readonly #byFolder: boolean;
  readonly #now: () => number;
  readonly #folderAt: (root: string) => Promise<string>;
  readonly #watched = new Map<string, Watched>();
  /** When each worktree that could not be watched failed. */
  readonly #failed = new Map<string, number>();
  /** The watch of each worktree that last ended. */
  readonly #ended = new Map<string, Ended>();
  /** The watched worktrees a notice counted in since a read of each began, until `noticed` hands them on. */
  readonly #noticed = new Set<string>();
  /** The worktrees the last `retain` kept; undefined before the first. */
  #retained: ReadonlySet<string> | undefined;

  /** `folderAt`: the folder at a worktree's path (`folderAt` by default), which tells a worktree made again there. */
  constructor(options: {
    treeWatch?: TreeWatch | undefined;
    platform?: NodeJS.Platform;
    now?: () => number;
    folderAt?: (root: string) => Promise<string>;
  } = {}) {
    this.#watch = options.treeWatch;
    this.#byFolder = (options.platform ?? process.platform) === "linux";
    this.#now = options.now ?? Date.now;
    this.#folderAt = options.folderAt ?? folderAt;
  }

  /** Takes every notice the watch queued since the last take, and weighs each against its worktree's last read. */
  collect(): void {
    if (this.#watch === undefined) return;
    for (const notice of this.#watch.take()) this.#notice(notice);
  }

  /**
   * Whether the worktree at `root` (physical) may hold a change since its last read; the first call starts watching it,
   * before its first read. True until a watch of it was up for a whole read, after a notice that counts, and while it
   * cannot be watched. A worktree not watched yet has its folder read first (`folderAt`), which the walk awaits.
   */
  async changed(root: string): Promise<boolean> {
    return (this.#watched.get(root) ?? (await this.#open(root)))?.changed ?? true;
  }

  /** Whether `root` is watched now, so a notice would tell of a change in it. */
  watching(root: string): boolean {
    return this.#watched.has(root);
  }

  /** Whether any worktree is watched now, so a notice may come. */
  watchingAny(): boolean {
    return this.#watched.size > 0;
  }

  /**
   * Takes the worktrees a notice counted in since the last call, and since a read of each began: each may hold a
   * change no read has seen, which a notice taken as the read began, or while it ran, would not.
   */
  noticed(): Set<string> {
    const noticed = new Set(this.#noticed);
    this.#noticed.clear();
    return noticed;
  }

  /** A read of `root` begins: what `changed` said is answered, and from now until `read` every notice counts. */
  reading(root: string): void {
    this.collect();
    const watched = this.#watched.get(root);
    // Every notice taken so far is the read's to answer.
    this.#noticed.delete(root);
    if (watched === undefined) return;
    watched.changed = false;
    watched.seen = undefined;
  }

  /** What the read of `root` saw; on Linux the watch follows its folders. */
  read(root: string, seen: WorktreeRead): void {
    // What the system queued while the read ran counts against no read.
    this.collect();
    const watched = this.#watched.get(root);
    if (watched === undefined) return;
    watched.seen = seen;
    if (this.#byFolder) this.#follow(root, watched, seen.folders);
  }

  /** Stops watching each worktree not in `roots`, releasing every handle the watch held on it. */
  retain(roots: ReadonlySet<string>): void {
    this.#retained = roots;
    for (const root of this.#watched.keys()) {
      if (roots.has(root)) continue;
      this.#watched.delete(root);
      this.#watch?.unwatch(root);
    }
    forgetOthers(this.#failed, roots);
    forgetOthers(this.#ended, roots);
    forgetOthers(this.#noticed, roots);
  }

  /**
   * Starts watching `root`, its root folder alone until a read names its folders; undefined when it cannot be watched
   * now, or when a `retain` let go of it while its folder was read.
   */
  async #open(root: string): Promise<Watched | undefined> {
    if (this.#watch === undefined || this.#waits(root)) return undefined;
    const folder = await this.#folderAt(root);
    if (this.#waitsFor(root, folder) || this.#letGo(root)) return undefined;
    this.#failed.delete(root);
    this.#ended.delete(root);
    // Up before the read: a change made while it runs is a notice.
    if (!this.#watch.watch(root, [""])) {
      this.#failed.set(root, this.#now());
      return undefined;
    }
    const watched: Watched = { folders: new Set([""]), seen: undefined, changed: true, folder };
    this.#watched.set(root, watched);
    return watched;
  }

  /**
   * Whether `root` is not to be watched yet, before its folder is read: it could not be watched within
   * `WATCH_RETRY_MS`, or its watch ended within `REWATCH_MS` and no folder was read as that watch began, so none can be
   * told from it.
   */
  #waits(root: string): boolean {
    const failedAt = this.#failed.get(root);
    if (failedAt !== undefined && this.#now() - failedAt < WATCH_RETRY_MS) return true;
    return this.#endedLately(root)?.folder === "";
  }

  /**
   * Whether `root`, with `folder` at its path now, is not to be watched yet: its watch ended within `REWATCH_MS` and no
   * other folder stands there (`remade`). One made again there is watched at once: no notice tells of a write in it
   * until it is watched, and no walk may come for one.
   */
  #waitsFor(root: string, folder: string): boolean {
    const ended = this.#endedLately(root);
    return ended !== undefined && !remade(ended.folder, folder);
  }

  /** Whether the last `retain` let go of `root`: no session watches it any more. */
  #letGo(root: string): boolean {
    return this.#retained !== undefined && !this.#retained.has(root);
  }

  /** The watch of `root` that ended within `REWATCH_MS`; undefined when none did. */
  #endedLately(root: string): Ended | undefined {
    const ended = this.#ended.get(root);
    return ended !== undefined && this.#now() - ended.at < REWATCH_MS ? ended : undefined;
  }

  /**
   * Watches the folders the read walked into, in place of those of the last. A folder watched now was not while the
   * read ran, so the next walk reads the worktree again. One the watch cannot follow (past the watch budget) is read on
   * every walk until the retry.
   */
  #follow(root: string, watched: Watched, folders: ReadonlySet<string>): void {
    const added = [...folders].some(folder => !watched.folders.has(folder));
    if (!added && folders.size === watched.folders.size) return;
    if (!this.#watch!.watch(root, [...folders])) {
      this.#watched.delete(root);
      this.#watch!.unwatch(root);
      this.#failed.set(root, this.#now());
      return;
    }
    watched.folders = folders;
    if (added) watched.changed = true;
  }

  #notice(notice: TreeNotice): void {
    const watched = this.#watched.get(notice.root);
    if (watched === undefined) return;
    // The watch let go (the worktree or a folder above it moved or went, a watch error): read it whole, and watch it
    // again as the next walk asks.
    if (notice.ended) {
      this.#watched.delete(notice.root);
      this.#ended.set(notice.root, { at: this.#now(), folder: watched.folder });
    }
    if (this.#byFolder) this.#unfollow(watched, notice);
    if (marksChanged(watched, notice)) {
      watched.changed = true;
      this.#noticed.add(notice.root);
    }
  }

  /**
   * A notice that lost its paths may have lost a folder that went and came back: the next read's folders are all
   * watched again, and the engine keeps the watches it holds. Otherwise only the folders that came or went are (`#replaced`).
   */
  #unfollow(watched: Watched, notice: TreeNotice): void {
    if (notice.lost) watched.folders = new Set();
    else this.#replaced(watched, notice.renamed);
  }

  /**
   * Drops from the folders watched each one that came or went, and every folder under it: a folder put back at the
   * same path is another folder, which the watch of the old one never tells of, so the next read's folders watch it.
   */
  #replaced(watched: Watched, renamed: readonly string[]): void {
    const gone = renamed.filter(path => watched.folders.has(path));
    if (gone.length === 0) return;
    watched.folders = new Set([...watched.folders].filter(folder => !gone.some(path => folder === path || folder.startsWith(`${path}/`))));
  }
}

/**
 * Whether a notice marks its worktree changed: its watch ended, it lost its paths, no finished read stands (one runs, or
 * none ran yet), or a path it names counts against the last read (`counts`).
 */
function marksChanged(watched: Watched, notice: TreeNotice): boolean {
  return notice.ended || notice.lost || watched.seen === undefined || counts(watched.seen, notice);
}

/** Forgets from `kept` each worktree not in `roots`. */
function forgetOthers(kept: Map<string, unknown> | Set<string>, roots: ReadonlySet<string>): void {
  for (const root of kept.keys()) if (!roots.has(root)) kept.delete(root);
}

/** `queued` and `notice`, of one root, as one notice: past `NOTICE_PATHS_CAP` paths, or with either lost, one that lost its paths. */
function mergedNotice(queued: TreeNotice | undefined, notice: TreeNotice): TreeNotice {
  if (queued === undefined) return notice;
  const ended = queued.ended || notice.ended;
  const changed = [...new Set([...queued.changed, ...notice.changed])];
  const renamed = [...new Set([...queued.renamed, ...notice.renamed])];
  if (queued.lost || notice.lost || changed.length + renamed.length > NOTICE_PATHS_CAP) {
    return { root: notice.root, changed: [], renamed: [], lost: true, ended };
  }
  return { root: notice.root, changed, renamed, lost: false, ended };
}

/**
 * One engine `TreeWatch` shared by the walks of every repository of an engine service (`RepositoryWorktreeWalks`):
 * each walk watches through a user of its own and takes only the notices of the roots it watches. A root several users
 * watch is watched once, with every folder any of them follows, until the last lets go of it.
 */
export class SharedTreeWatch {
  readonly #watch: TreeWatch;
  /** By root, the folders each user watches it with. */
  readonly #roots = new Map<string, Map<object, readonly string[]>>();
  /**
   * By user, the notices taken from the watch for it and not yet taken by it, one per root as the engine's take hands
   * them: a walk that takes them rarely holds no more than the engine would. A user that watches nothing and has taken
   * every notice queued for it has none.
   */
  readonly #queues = new Map<object, Map<string, TreeNotice>>();
  #closed = false;

  constructor(watch: TreeWatch) {
    this.#watch = watch;
  }

  /** A `TreeWatch` for one walk; closing it lets go of every root it watches. */
  user(): TreeWatch {
    const user = {};
    return {
      watch: (root, folders) => this.#watchFor(user, root, folders),
      unwatch: root => this.#unwatchFor(user, root),
      take: () => this.#takeFor(user),
      close: () => this.#closeFor(user),
    };
  }

  /** Lets go of every root and of the engine's thread. */
  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#roots.clear();
    this.#queues.clear();
    this.#watch.close();
  }

  #watchFor(user: object, root: string, folders: readonly string[]): boolean {
    if (this.#closed) return false;
    const users = this.#roots.get(root) ?? new Map<object, readonly string[]>();
    users.set(user, folders);
    this.#roots.set(root, users);
    if (this.#watch.watch(root, unionOf(users))) {
      if (!this.#queues.has(user)) this.#queues.set(user, new Map());
      return true;
    }
    // A user refused a root does not hold it: it reads that root on every walk, and no watch of it stays up for it. The
    // engine let go of the root and of the notices it queued, so the users left on it watch it again from nothing.
    users.delete(user);
    if (users.size === 0) this.#roots.delete(root);
    else if (this.#watchLeft(root, users)) this.#queue(users.keys(), lostNotice(root, false));
    return false;
  }

  #unwatchFor(user: object, root: string): void {
    // What was queued of a root its walk lets go of is no longer its to take: the walk stopped weighing the root.
    this.#queues.get(user)?.delete(root);
    const users = this.#roots.get(root);
    if (users?.delete(user)) this.#release(root, users);
    this.#forgetIdle(user);
  }

  /** Watches `root` with the folders of the `users` left on it, or lets go of it when none is. */
  #release(root: string, users: ReadonlyMap<object, readonly string[]>): void {
    if (users.size > 0) {
      this.#watchLeft(root, users);
      return;
    }
    this.#roots.delete(root);
    this.#watch.unwatch(root);
  }

  /**
   * Watches `root` with the folders of the `users` left on it; true when the engine holds it. When it refuses, it let go
   * of the root and of its notices: no user holds the root any more, and each is told its watch ended.
   */
  #watchLeft(root: string, users: ReadonlyMap<object, readonly string[]>): boolean {
    if (this.#watch.watch(root, unionOf(users))) return true;
    this.#roots.delete(root);
    this.#queue(users.keys(), lostNotice(root, true));
    return false;
  }

  /** Queues `notice` for each of `users` still watching anything, merged with what it has not taken of that root. */
  #queue(users: Iterable<object>, notice: TreeNotice): void {
    for (const user of users) {
      const queue = this.#queues.get(user);
      queue?.set(notice.root, mergedNotice(queue.get(notice.root), notice));
    }
  }

  /** Drops `user`'s queue once it watches nothing and has taken every notice queued for it, an ended watch's included. */
  #forgetIdle(user: object): void {
    if (this.#queues.get(user)?.size !== 0) return;
    if (![...this.#roots.values()].some(users => users.has(user))) this.#queues.delete(user);
  }

  #takeFor(user: object): TreeNotice[] {
    if (this.#closed) return [];
    for (const notice of this.#watch.take()) {
      const users = [...this.#roots.get(notice.root)?.keys() ?? []];
      // An ended watch watches nothing: each user watching the root watches it again as its walk asks.
      if (notice.ended) this.#roots.delete(notice.root);
      this.#queue(users, notice);
    }
    const queue = this.#queues.get(user);
    if (queue === undefined) return [];
    const notices = [...queue.values()];
    queue.clear();
    this.#forgetIdle(user);
    return notices;
  }

  #closeFor(user: object): void {
    for (const [root, users] of [...this.#roots]) if (users.has(user)) this.#unwatchFor(user, root);
    this.#queues.delete(user);
  }
}

/** Every folder any user watches a root with. */
function unionOf(users: ReadonlyMap<object, readonly string[]>): string[] {
  return [...new Set([...users.values()].flat())];
}

/** A notice that `root`'s queued notices were lost, and with `ended` that its watch ended. */
function lostNotice(root: string, ended: boolean): TreeNotice {
  return { root, changed: [], renamed: [], lost: true, ended };
}
