/**
 * The operating system's change notices for the walk of a repository's other worktrees (`WorktreeWalks`): which
 * worktrees may hold a change since a walk last read them, so a walk reads only those and keeps what it saw of the rest.
 *
 * Windows and macOS watch each worktree with one recursive watch (ReadDirectoryChangesW, FSEvents). Node's recursive
 * watch on Linux keeps a watch and a `stat` of every file, taken synchronously as it starts, so on Linux each folder a
 * read walked into gets a watch of its own instead: inotify reports a change to any entry of a watched folder, a file's
 * text included, and nothing deeper. Elsewhere, and on a Linux filesystem that another machine may write to, nothing is
 * watched and every walk reads the worktree, as it did before notices.
 *
 * A notice counts unless it names something the last read saw and left out: a path inside a `.git` folder, the times of
 * a `.git` entry or of a folder the read walked (each change among a folder's entries comes as a notice of its own), or
 * a path in or under what the read did not walk or observe (an ignored file or folder, another worktree). A notice with
 * no name (Windows: the notice buffer overflowed), one Linux may have dropped (`NoticeQueue`), one that arrives while a
 * read runs, and one naming anything else, short Windows names included, count.
 */
import { type FSWatcher, readFileSync, statfsSync, watch as watchPath } from "node:fs";
import { basename, isAbsolute, join, sep } from "node:path";

/** What a walk's read of a worktree saw, by paths relative to the worktree (`/`-separated, `""` for the worktree itself). */
export interface WorktreeRead {
  /** Each folder the read walked into. */
  readonly folders: ReadonlySet<string>;
  /** Each file and folder the read found and neither observed nor walked into: ignored, or another worktree. */
  readonly leftOut: ReadonlySet<string>;
}

/**
 * Which worktrees a walk must read again (`WorktreeWalks`). A walk asks `changed` of each worktree; one it reads it
 * marks with `reading` as the read begins and `read` as it ends. Between the two every notice counts, and outside them
 * a notice counts unless the last read's `WorktreeRead` says otherwise, whether or not the walks since read it.
 */
export interface WorktreeNotices {
  /**
   * Whether the worktree at `root` (physical) may hold a change since its last read; the first call starts watching it.
   * True until a watch of it was up for a whole read, after a notice that counts, and when it cannot be watched.
   */
  changed(root: string): boolean;
  /** A read of `root` begins: what `changed` said is answered, and from now until `read` every notice counts. */
  reading(root: string): void;
  /** What the read of `root` saw; a watch per folder follows its folders. */
  read(root: string, seen: WorktreeRead): void;
  /** Stops watching each worktree not in `roots`. */
  retain(roots: ReadonlySet<string>): void;
}

type Watch = Pick<FSWatcher, "close" | "on">;
/** `fs.watch`, or a stand-in: the listener gets each notice's kind and the path it names, relative to `path`. */
export type WatchPath = (path: string, options: { readonly recursive: boolean }, listener: (event: string, name: string | null) => void) => Watch;

/** A worktree that could not be watched is tried again after this long, read on every walk meanwhile. */
const WATCH_RETRY_MS = 600_000;
/** The most folder watches open at once in a process, whatever `max_user_watches` allows (about 1.2 KB of memory each). */
const MOST_FOLDER_WATCHES = 131_072;

/**
 * Filesystems whose changes all pass through this machine's kernel, where inotify sees every write: ext2 to ext4, XFS,
 * Btrfs, tmpfs, ZFS, F2FS, overlayfs, bcachefs, ReiserFS, JFS, NILFS and ramfs, by `statfs` type. A network or FUSE
 * filesystem, or a Windows drive under WSL (9p), may be written from elsewhere without a notice here.
 */
const LOCAL_FILESYSTEMS = new Set([
  0xef53, 0x58465342, 0x9123683e, 0x01021994, 0x2fc12fc1, 0xf2f52010, 0x794c7630, 0xca451a4e, 0x52654973, 0x3153464a, 0x3434, 0x858458f6,
]);

/** A number from `/proc/sys/fs/inotify`, or `fallback` where it cannot be read. */
function inotifyLimit(name: string, fallback: number): number {
  try {
    const value = Number.parseInt(readFileSync(`/proc/sys/fs/inotify/${name}`, "utf8"), 10);
    return Number.isFinite(value) && value > 0 ? value : fallback;
  } catch {
    return fallback;
  }
}

/**
 * Linux's one inotify queue per thread, through which libuv reads every folder watch, and the watches it can take.
 * The kernel holds `max_queued_events` notices; past that it drops them and queues an overflow notice, which libuv drops
 * too. libuv drains the whole queue in one go, calling the listeners before the loop next reaches its check phase. So a
 * drain that brings at least half that many notices between two check phases may have followed a drop, and every
 * worktree is read again. Half, not all: notices for watches closed meanwhile are queued but reach no listener.
 */
export class NoticeQueue {
  readonly #options: { readonly overflowBurst?: number; readonly mostWatches?: number };
  /** Read from the system on first use, so a process that watches no folder never reads them. */
  #limits: { readonly overflowBurst: number; readonly mostWatches: number } | undefined;
  readonly #lost = new Set<() => void>();
  #burst = 0;
  #watches = 0;

  constructor(options: { overflowBurst?: number; mostWatches?: number } = {}) {
    this.#options = options;
  }

  /** Counts a notice; at the overflow burst, tells every worktree it may have lost some. */
  counted(): void {
    this.#burst += 1;
    if (this.#burst === 1) setImmediate(() => { this.#burst = 0; });
    if (this.#burst === this.#limitsRead().overflowBurst) for (const lost of this.#lost) lost();
  }

  /** Takes `count` more watches if the process may hold them. */
  take(count: number): boolean {
    if (this.#watches + count > this.#limitsRead().mostWatches) return false;
    this.#watches += count;
    return true;
  }

  #limitsRead(): { readonly overflowBurst: number; readonly mostWatches: number } {
    this.#limits ??= {
      overflowBurst: this.#options.overflowBurst ?? Math.floor(inotifyLimit("max_queued_events", 16_384) / 2),
      mostWatches: this.#options.mostWatches ?? Math.min(Math.floor(inotifyLimit("max_user_watches", 8_192) / 2), MOST_FOLDER_WATCHES),
    };
    return this.#limits;
  }

  give(count: number): void {
    this.#watches -= count;
  }

  /** Calls `lost` whenever notices may have been dropped, until the returned function is called. */
  onLost(lost: () => void): () => void {
    this.#lost.add(lost);
    return () => this.#lost.delete(lost);
  }
}

const linuxQueue = new NoticeQueue();

/** Resolves once the loop has passed through a whole poll phase, where it takes every notice the system had queued. */
export async function noticesTaken(): Promise<void> {
  for (let check = 0; check < 2; check += 1) await new Promise(resolve => { setImmediate(resolve); });
}

function missing(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "ENOENT";
}

/** Whether a notice of `event` for `path` (relative to the worktree) names something the last read cared about. */
export function noticeCounts(seen: WorktreeRead, event: string, path: string): boolean {
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

interface Watched {
  /** By folder relative to the worktree; `""` alone for a recursive watch. */
  readonly watches: Map<string, Watch>;
  /** What the last finished read saw; undefined while a read runs or before the first, when every notice counts. */
  seen: WorktreeRead | undefined;
  changed: boolean;
  /** Folders whose watch may watch nothing now (the folder itself moved or went), watched again at the next read. */
  readonly stale: Set<string>;
  /**
   * Marks the worktree changed and stops watching it, when one of its watches fails or (Windows) names the worktree
   * itself, gone: one function for all of a worktree's watches, which on Linux number one per folder.
   */
  readonly stop: () => void;
}

/** `WorktreeNotices` from `fs.watch`, one recursive watch per worktree or one watch per folder (see the module). */
export class WatchedWorktrees implements WorktreeNotices {
  readonly #watch: WatchPath;
  readonly #recursive: boolean;
  readonly #platform: NodeJS.Platform;
  readonly #queue: NoticeQueue;
  readonly #watched = new Map<string, Watched>();
  /** When each worktree that could not be watched failed. */
  readonly #failed = new Map<string, number>();
  #stopLost: (() => void) | undefined;

  constructor(options: { platform?: NodeJS.Platform; watch?: WatchPath; queue?: NoticeQueue } = {}) {
    this.#platform = options.platform ?? process.platform;
    this.#recursive = this.#platform === "win32" || this.#platform === "darwin";
    this.#watch = options.watch ?? ((path, { recursive }, listener) => watchPath(path, { recursive, persistent: false }, listener));
    this.#queue = options.queue ?? linuxQueue;
  }

  changed(root: string): boolean {
    return (this.#watched.get(root) ?? this.#open(root))?.changed ?? true;
  }

  reading(root: string): void {
    const watched = this.#watched.get(root);
    if (watched === undefined) return;
    watched.changed = false;
    watched.seen = undefined;
  }

  read(root: string, seen: WorktreeRead): void {
    const watched = this.#watched.get(root);
    if (watched === undefined) return;
    watched.seen = seen;
    if (!this.#recursive) this.#followFolders(root, watched, seen.folders);
  }

  retain(roots: ReadonlySet<string>): void {
    for (const root of this.#watched.keys()) if (!roots.has(root)) this.#close(root);
    for (const root of this.#failed.keys()) if (!roots.has(root)) this.#failed.delete(root);
  }

  /** Starts watching `root`; undefined when it cannot be watched now. */
  #open(root: string): Watched | undefined {
    const failedAt = this.#failed.get(root);
    if (failedAt !== undefined && Date.now() - failedAt < WATCH_RETRY_MS) return undefined;
    this.#failed.delete(root);
    if (!this.#watchable(root)) return this.#fail(root);
    const watched: Watched = {
      watches: new Map(), seen: undefined, changed: true, stale: new Set(),
      stop: () => {
        watched.changed = true;
        if (this.#watched.get(root) === watched) this.#close(root);
      },
    };
    this.#watched.set(root, watched);
    if (!this.#recursive) {
      this.#stopLost ??= this.#queue.onLost(() => { for (const each of this.#watched.values()) each.changed = true; });
      return watched;
    }
    // Up before the read: a change made while it runs is a notice.
    try {
      watched.watches.set("", this.#watchFolder(root, "", watched));
      return watched;
    } catch {
      this.#close(root);
      return this.#fail(root);
    }
  }

  /** Whether notices can tell every change in `root` on this system. */
  #watchable(root: string): boolean {
    if (this.#recursive) return true;
    if (this.#platform !== "linux") return false;
    try {
      return LOCAL_FILESYSTEMS.has(Number(statfsSync(root).type));
    } catch {
      return false;
    }
  }

  #fail(root: string): undefined {
    this.#failed.set(root, Date.now());
    return undefined;
  }

  /**
   * Watches each folder of `folders` not watched yet, and stops watching each the read no longer walked. A folder
   * watched now was not while the read ran, so the next walk reads the worktree again. A worktree whose folders the
   * process cannot all watch is read on every walk instead, until the retry.
   */
  #followFolders(root: string, watched: Watched, folders: ReadonlySet<string>): void {
    for (const [folder, watch] of watched.watches) {
      if (folders.has(folder) && !watched.stale.has(folder)) continue;
      watch.close();
      watched.watches.delete(folder);
      this.#queue.give(1);
    }
    watched.stale.clear();
    const added = [...folders].filter(folder => !watched.watches.has(folder));
    if (added.length === 0) return;
    if (!this.#queue.take(added.length)) {
      this.#close(root);
      this.#fail(root);
      return;
    }
    watched.changed = true;
    for (const [index, folder] of added.entries()) {
      try {
        watched.watches.set(folder, this.#watchFolder(root, folder, watched));
      } catch (error) {
        // A folder removed since the read is the next read's to find gone; any other failure gives up the worktree.
        if (missing(error)) {
          this.#queue.give(1);
          continue;
        }
        this.#queue.give(added.length - index);
        this.#close(root);
        this.#fail(root);
        return;
      }
    }
  }

  #watchFolder(root: string, folder: string, watched: Watched): Watch {
    const absolute = folder === "" ? root : join(root, ...folder.split("/"));
    const watch = this.#watch(absolute, { recursive: this.#recursive }, (event, name) => this.#notice(root, watched, folder, event, name));
    watch.on("error", watched.stop);
    return watch;
  }

  #notice(root: string, watched: Watched, folder: string, event: string, name: string | null): void {
    if (!this.#recursive) this.#queue.counted();
    if (name === null) {
      watched.changed = true;
      return;
    }
    // Windows names the watched folder itself, again and again until the watch closes, once it is removed.
    if (this.#recursive && isAbsolute(name)) {
      watched.stop();
      return;
    }
    // inotify names a watched folder itself when it is moved or removed; a child of the same name is watched again too.
    if (!this.#recursive && event === "rename" && name === basename(folder === "" ? root : folder)) watched.stale.add(folder);
    const relative = this.#recursive ? name.split(sep).join("/") : folder === "" ? name : `${folder}/${name}`;
    if (watched.seen === undefined || noticeCounts(watched.seen, event, relative)) watched.changed = true;
  }

  #close(root: string): void {
    const watched = this.#watched.get(root);
    if (watched === undefined) return;
    this.#watched.delete(root);
    for (const watch of watched.watches.values()) watch.close();
    if (!this.#recursive) this.#queue.give(watched.watches.size);
    watched.watches.clear();
    if (this.#watched.size === 0) {
      this.#stopLost?.();
      this.#stopLost = undefined;
    }
  }
}
