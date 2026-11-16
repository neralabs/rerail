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
 * A worktree whose watch ended is watched again no sooner than this after: read whole on every walk meanwhile. On
 * Windows a change among a worktree's top entries ends its watch, and watching it again at once could end it again on
 * every walk.
 */
export const REWATCH_MS = 5_000;

/**
 * Whether a notice of `event` (`change`: a file's text, size or attributes; `rename`: an entry created, deleted or
 * renamed) for `path` (relative to the worktree) names something the last read cared about.
 */
export function noticeCounts(seen: WorktreeRead, event: "change" | "rename", path: string): boolean {
  const parts = path.split("/");
  const git = parts.indexOf(".git");
  if (git >= 0 && (git < parts.length - 1 || event !== "rename")) return false;
  // What the read left out may come or go (a file where an ignored folder was is no longer ignored); its own times and
  // anything under it do not matter.
  let prefix = "";
  for (const [index, part] of parts.entries()) {
    prefix = index === 0 ? part : `${prefix}/${part}`;
    if (seen.leftOut.has(prefix) && (index < parts.length - 1 || event !== "rename")) return false;
  }
  return event === "rename" || !seen.folders.has(path);
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
  readonly #watched = new Map<string, Watched>();
  /** When each worktree that could not be watched failed. */
  readonly #failed = new Map<string, number>();
  /** When the watch of each worktree last ended. */
  readonly #ended = new Map<string, number>();

  constructor(options: { treeWatch?: TreeWatch | undefined; platform?: NodeJS.Platform; now?: () => number } = {}) {
    this.#watch = options.treeWatch;
    this.#byFolder = (options.platform ?? process.platform) === "linux";
    this.#now = options.now ?? Date.now;
  }

  /** Takes every notice the watch queued since the last take, and weighs each against its worktree's last read. */
  collect(): void {
    if (this.#watch === undefined) return;
    for (const notice of this.#watch.take()) this.#notice(notice);
  }

  /**
   * Whether the worktree at `root` (physical) may hold a change since its last read; the first call starts watching it,
   * before its first read. True until a watch of it was up for a whole read, after a notice that counts, and while it
   * cannot be watched.
   */
  changed(root: string): boolean {
    return (this.#watched.get(root) ?? this.#open(root))?.changed ?? true;
  }

  /** Whether `root` is watched now, so a notice would tell of a change in it. */
  watching(root: string): boolean {
    return this.#watched.has(root);
  }

  /** A read of `root` begins: what `changed` said is answered, and from now until `read` every notice counts. */
  reading(root: string): void {
    this.collect();
    const watched = this.#watched.get(root);
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
    for (const root of this.#watched.keys()) {
      if (roots.has(root)) continue;
      this.#watched.delete(root);
      this.#watch?.unwatch(root);
    }
    for (const root of this.#failed.keys()) if (!roots.has(root)) this.#failed.delete(root);
    for (const root of this.#ended.keys()) if (!roots.has(root)) this.#ended.delete(root);
  }

  /** Starts watching `root`, its root folder alone until a read names its folders; undefined when it cannot be watched now. */
  #open(root: string): Watched | undefined {
    if (this.#watch === undefined) return undefined;
    const failedAt = this.#failed.get(root);
    if (failedAt !== undefined && this.#now() - failedAt < WATCH_RETRY_MS) return undefined;
    const endedAt = this.#ended.get(root);
    if (endedAt !== undefined && this.#now() - endedAt < REWATCH_MS) return undefined;
    this.#failed.delete(root);
    this.#ended.delete(root);
    // Up before the read: a change made while it runs is a notice.
    if (!this.#watch.watch(root, [""])) {
      this.#failed.set(root, this.#now());
      return undefined;
    }
    const watched: Watched = { folders: new Set([""]), seen: undefined, changed: true };
    this.#watched.set(root, watched);
    return watched;
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
    // The watch let go (the worktree or a folder above it moved or went, a Windows oplock broke): read it whole, and
    // watch it again as the next walk asks.
    if (notice.ended) {
      this.#watched.delete(notice.root);
      this.#ended.set(notice.root, this.#now());
    }
    if (notice.ended || notice.lost || watched.seen === undefined || counts(watched.seen, notice)) watched.changed = true;
  }
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
  /** By user, the notices taken from the watch for it and not yet taken by it. */
  readonly #queues = new Map<object, TreeNotice[]>();
  #closed = false;

  constructor(watch: TreeWatch) {
    this.#watch = watch;
  }

  /** A `TreeWatch` for one walk; closing it lets go of every root it watches. */
  user(): TreeWatch {
    const user = {};
    this.#queues.set(user, []);
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
    return this.#watch.watch(root, unionOf(users));
  }

  #unwatchFor(user: object, root: string): void {
    const users = this.#roots.get(root);
    if (users === undefined || !users.delete(user)) return;
    if (users.size > 0) {
      this.#watch.watch(root, unionOf(users));
      return;
    }
    this.#roots.delete(root);
    this.#watch.unwatch(root);
  }

  #takeFor(user: object): TreeNotice[] {
    if (this.#closed) return [];
    for (const notice of this.#watch.take()) {
      const users = [...this.#roots.get(notice.root)?.keys() ?? []];
      // An ended watch watches nothing: each user watching the root watches it again as its walk asks.
      if (notice.ended) this.#roots.delete(notice.root);
      for (const other of users) this.#queues.get(other)?.push(notice);
    }
    const queue = this.#queues.get(user) ?? [];
    this.#queues.set(user, []);
    return queue;
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
