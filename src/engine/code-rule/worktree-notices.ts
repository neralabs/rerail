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
 *
 * The folder where Git registers a repository's linked worktrees (`worktrees` in its common Git folder) is watched too
 * (`watchRegistry`): `git worktree add`, `remove`, `move`, `repair` and `prune` change an entry there, so a notice of one
 * tells the walks that Git's list may name a worktree it did not, such as one removed and added again at its path.
 */
import { type BigIntStats, readdirSync } from "node:fs";
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
 * another folder than the one it watched now stands at its path (`folderAt`), or its file system cannot tell
 * (`UNKNOWN_FOLDER`): a worktree made again there is watched as the next walk reads it, so a write after that read is a
 * notice. A watch ends when the worktree or a folder above it is
 * renamed, moved or deleted (on Windows, when the path then holds no folder or another, or after a run of renames: the
 * engine's `REOPENS` within `REOPEN_WINDOW`), or on a watch error. Watching the same folder again at once would chase a
 * worktree still being moved, its watch ending again on every walk.
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
 * Whether `now`, the folder at a worktree's path, is another than `before`, the one an ended watch watched (`folderAt`):
 * a folder whose file system cannot tell it from that one (`UNKNOWN_FOLDER`) is taken for another.
 */
function remade(before: string, now: string): boolean {
  return now !== "" && (now !== before || now === UNKNOWN_FOLDER);
}

/** Whether any path a notice names counts against `seen` (`noticeCounts`). */
function counts(seen: WorktreeRead, notice: TreeNotice): boolean {
  return notice.renamed.some(path => noticeCounts(seen, "rename", path)) || notice.changed.some(path => noticeCounts(seen, "change", path));
}

/** The files of a worktree's entry in Git's registry that say where it is (`gitdir`) and whether Git is still making it (`locked`). */
const REGISTRY_FILES = new Set(["gitdir", "locked"]);

/**
 * Whether a notice of `event` for `path` (relative to the folder where Git registers the worktrees) may change what Git
 * lists: an entry came or went, or its `gitdir` or `locked` file did or was written. Git writes the rest of an entry
 * (`HEAD`, `index`, `logs`) as it works in that worktree, which moves nothing it lists.
 */
export function registryNoticeCounts(event: "change" | "rename", path: string): boolean {
  const parts = path.split("/");
  if (parts.length === 1) return event === "rename";
  return parts.length === 2 && REGISTRY_FILES.has(parts[1]!);
}

/** Whether a notice of a registry folder may change what Git lists (`registryNoticeCounts`); one that lost its paths or ended may. */
function registryCounts(notice: TreeNotice): boolean {
  if (notice.ended || notice.lost) return true;
  return notice.renamed.some(path => registryNoticeCounts("rename", path)) || notice.changed.some(path => registryNoticeCounts("change", path));
}

/**
 * What `folderAt` gives for a folder on a file system that records no birth time (ext3, for one): there a folder made
 * again may take the file number of one just removed, so it is never taken for the folder that stood there before.
 */
export const UNKNOWN_FOLDER = "?";

/**
 * The folder at `root`: its device, file number and birth time (a folder made on Linux may take the file number of one
 * just removed); `""` when none is there. One made within a tick of the file system's clock (4 ms at Linux's 250 Hz)
 * after the folder removed there was made takes its birth time too, and reads as that folder. Read through Node's
 * thread pool, so a path that does not answer (a hung network share) holds no walk of another worktree and no event in
 * the daemon's thread. Rejects when the path could not be read (EACCES, EIO), which says nothing of what stands there.
 */
export async function folderAt(root: string): Promise<string> {
  try {
    return folderIdentity(await lstat(root, { bigint: true }));
  } catch (error) {
    if (nothingAt(error)) return "";
    throw error;
  }
}

/** Whether a read of a path failed because nothing stands there: ENOENT, or ENOTDIR where a part of it is a file. */
function nothingAt(error: unknown): boolean {
  const code = error instanceof Error && "code" in error ? error.code : undefined;
  return code === "ENOENT" || code === "ENOTDIR";
}

/** Whether `read` (`folderAt`) found no folder at its path; false when it rejected, which says nothing of what stands there. */
function foundNothing(read: Promise<string>): Promise<boolean> {
  return read.then(folder => folder === "", () => false);
}

/** `folderAt` of the entry `info` describes: `""` for no folder, `UNKNOWN_FOLDER` for one with no birth time. */
export function folderIdentity(info: Pick<BigIntStats, "dev" | "ino" | "birthtimeNs" | "isDirectory">): string {
  if (!info.isDirectory()) return "";
  return info.birthtimeNs === 0n ? UNKNOWN_FOLDER : `${info.dev}:${info.ino}:${info.birthtimeNs}`;
}

/** The folders a watch of the registry `folder` follows on Linux: its own, and each worktree's entry, where Git writes `gitdir` and `locked`. */
function registryFolders(folder: string): string[] {
  try {
    return ["", ...readdirSync(folder, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort()];
  } catch {
    return [""];
  }
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
  /** The folders where Git registers the worktrees that are watched (`watchRegistry`), each with the folders it follows on Linux. */
  readonly #registries = new Map<string, readonly string[]>();
  /**
   * The watched worktrees a notice counted in since a read of each began, and the registry folders a notice counted in
   * (`registryNoticeCounts`), until `noticed` hands them on.
   */
  readonly #noticed = new Set<string>();
  /** The worktrees the last `retain` kept; undefined before the first. */
  #retained: ReadonlySet<string> | undefined;

  /**
   * `folderAt`: the folder at a worktree's path (`folderAt` by default), which tells a worktree made again there; it
   * rejects for a path it could not read.
   */
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

  /** Whether any worktree, or a folder where Git registers them, is watched now, so a notice may come. */
  watchingAny(): boolean {
    return this.#watched.size > 0 || this.#registries.size > 0;
  }

  /**
   * Watches `folder` (physical), where Git registers a repository's linked worktrees, unless it is watched; on Linux with
   * each worktree's entry, so Git's writing of an entry's `gitdir` and `locked` is a notice too. Once a notice there may
   * change what Git lists (`registryNoticeCounts`), `noticed` hands on `folder`. A folder that is not there, or that the
   * watch refuses, is tried again at the next call. It is watched spare (`TreeWatch.watch`): on Linux it takes only room
   * the watch budget has left, so every worktree fits as it would with no registry watched, and its watch ends once a
   * worktree needs that room.
   */
  watchRegistry(folder: string): void {
    if (this.#watch === undefined || this.#registries.has(folder)) return;
    this.#followRegistry(folder);
  }

  /**
   * Takes the worktrees a notice counted in since the last call, and since a read of each began: each may hold a
   * change no read has seen, which a notice taken as the read began, or while it ran, would not. With them, each
   * registry folder (`watchRegistry`) a notice counted in since the last call: Git's list may name another worktree.
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

  /**
   * Stops watching each worktree, and each registry folder (`watchRegistry`), not in `roots`, releasing every handle the
   * watch held on it.
   */
  retain(roots: ReadonlySet<string>): void {
    this.#retained = roots;
    this.#unwatchOthers(this.#watched, roots);
    this.#unwatchOthers(this.#registries, roots);
    forgetOthers(this.#failed, roots);
    forgetOthers(this.#ended, roots);
    forgetOthers(this.#noticed, roots);
  }

  /** Stops watching each root of `watched` not in `roots`. */
  #unwatchOthers(watched: Map<string, unknown>, roots: ReadonlySet<string>): void {
    for (const root of watched.keys()) {
      if (roots.has(root)) continue;
      watched.delete(root);
      this.#watch?.unwatch(root);
    }
  }

  /**
   * Starts watching `root`, its root folder alone until a read names its folders; undefined when it cannot be watched
   * now, or when a `retain` let go of it while its folder was read.
   */
  async #open(root: string): Promise<Watched | undefined> {
    if (this.#watch === undefined || this.#waits(root)) return undefined;
    const read = this.#folderAt(root);
    // A path that could not be read tells no folder from another, as one with no folder.
    const folder = await read.catch(() => "");
    if (this.#waitsFor(root, folder) || this.#letGo(root)) return undefined;
    this.#failed.delete(root);
    this.#ended.delete(root);
    // Up before the read: a change made while it runs is a notice.
    if (!this.#watch.watch(root, [""])) {
      await this.#refused(root, await foundNothing(read));
      return undefined;
    }
    const watched: Watched = { folders: new Set([""]), seen: undefined, changed: true, folder };
    this.#watched.set(root, watched);
    return watched;
  }

  /**
   * The engine refused to watch `root`: it is tried again after `WATCH_RETRY_MS`, unless a read of its path found no
   * folder there, the read before the watch (`nothingBefore`) or one after the refusal: the folder went after the walk
   * listed or read it, or came only after the read before the watch, and the walk Git's registry asks for once the
   * worktree is made again watches it. A read that rejected says nothing of what stands there: with the other finding a
   * folder or rejecting too, the watch failed. A `retain` that let go of `root` meanwhile forgot it, so nothing is
   * recorded.
   */
  async #refused(root: string, nothingBefore: boolean): Promise<void> {
    const gone = nothingBefore || (await foundNothing(this.#folderAt(root)));
    if (!gone && !this.#letGo(root)) this.#failed.set(root, this.#now());
  }

  /** Watches the registry `folder`, spare, with the folders it follows now (`registryFolders` on Linux); one the watch refuses is let go. */
  #followRegistry(folder: string): void {
    const folders = this.#byFolder ? registryFolders(folder) : [""];
    if (this.#watch!.watch(folder, folders, true)) this.#registries.set(folder, folders);
    else this.#registries.delete(folder);
  }

  /**
   * A notice of a registry folder: once its watch ended, it is watched again at the next `watchRegistry`; on Linux an
   * entry that came or went, or a notice that lost its paths, has the watch follow the entries there now.
   */
  #registryNotice(notice: TreeNotice): void {
    if (notice.ended) this.#registries.delete(notice.root);
    else if (this.#byFolder && (notice.lost || notice.renamed.some(path => !path.includes("/")))) this.#followRegistry(notice.root);
    if (registryCounts(notice)) this.#noticed.add(notice.root);
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
   * every walk until the retry (`#refused`), unless its folder went after the read.
   */
  #follow(root: string, watched: Watched, folders: ReadonlySet<string>): void {
    const added = [...folders].some(folder => !watched.folders.has(folder));
    if (!added && folders.size === watched.folders.size) return;
    if (!this.#watch!.watch(root, [...folders])) {
      this.#watched.delete(root);
      this.#watch!.unwatch(root);
      // The walk's read found its folder there: only a read after the refusal may find none.
      void this.#refused(root, false);
      return;
    }
    watched.folders = folders;
    if (added) watched.changed = true;
  }

  #notice(notice: TreeNotice): void {
    if (this.#registries.has(notice.root)) {
      this.#registryNotice(notice);
      return;
    }
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

/** What one user of a `SharedTreeWatch` asked of a root: the folders it follows, and whether it watches the root spare. */
interface Asked {
  readonly folders: readonly string[];
  readonly spare: boolean;
}

/**
 * One engine `TreeWatch` shared by the walks of every repository of an engine service (`RepositoryWorktreeWalks`):
 * each walk watches through a user of its own and takes only the notices of the roots it watches. A root several users
 * watch is watched once, with every folder any of them follows, spare while each of them watches it spare, until the
 * last lets go of it.
 */
export class SharedTreeWatch {
  readonly #watch: TreeWatch;
  /** By root, what each user watching it asked. */
  readonly #roots = new Map<string, Map<object, Asked>>();
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
      watch: (root, folders, spare) => this.#watchFor(user, root, { folders, spare: spare === true }),
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

  #watchFor(user: object, root: string, asked: Asked): boolean {
    if (this.#closed) return false;
    const users = this.#roots.get(root) ?? new Map<object, Asked>();
    users.set(user, asked);
    this.#roots.set(root, users);
    if (this.#engineWatch(root, users)) {
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
  #release(root: string, users: ReadonlyMap<object, Asked>): void {
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
  #watchLeft(root: string, users: ReadonlyMap<object, Asked>): boolean {
    if (this.#engineWatch(root, users)) return true;
    this.#roots.delete(root);
    this.#queue(users.keys(), lostNotice(root, true));
    return false;
  }

  /** Watches `root` with every folder its `users` follow, spare while each of them watches it spare. */
  #engineWatch(root: string, users: ReadonlyMap<object, Asked>): boolean {
    return this.#watch.watch(root, unionOf(users), [...users.values()].every(asked => asked.spare));
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
function unionOf(users: ReadonlyMap<object, Asked>): string[] {
  return [...new Set([...users.values()].flatMap(asked => asked.folders))];
}

/** A notice that `root`'s queued notices were lost, and with `ended` that its watch ended. */
function lostNotice(root: string, ended: boolean): TreeNotice {
  return { root, changed: [], renamed: [], lost: true, ended };
}
