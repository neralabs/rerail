/**
 * The operating system's change notices for the walk of a repository's other worktrees (`WorktreeWalks`): which
 * worktrees may hold a change since a walk last read them, so a walk reads only those and keeps what it saw of the rest.
 * The session folder's own walk takes its notices the same way (`SessionNotices`): which of its paths changed since its
 * last pass, so a command's result reads only those.
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
 * A worktree whose watch ended is watched again no sooner than this after: read whole on every walk meanwhile. A watch
 * ends when the worktree or a folder above it is renamed, moved or deleted (on Windows, when the path then holds no
 * folder or another, or after a run of renames: the engine's `REOPENS` within `REOPEN_WINDOW`), or on a watch error.
 * Watching it again at once would chase a worktree still being moved or replaced, its watch ending again on every walk.
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

/** Whether any path a notice names counts against `seen` (`noticeCounts`). */
function counts(seen: WorktreeRead, notice: TreeNotice): boolean {
  return notice.renamed.some(path => noticeCounts(seen, "rename", path)) || notice.changed.some(path => noticeCounts(seen, "change", path));
}

/**
 * The folders a Linux watch of one root follows (`TreeWatch.watch`), as last given to it, less each one the engine let go
 * of since. The engine lets go of a folder that is deleted or moved away, with every folder under it, and a folder put
 * back at the same path is another, which no watch follows until a `watch` names it again; a notice that lost its paths
 * may have lost such a going, so after one no folder counts as followed. Elsewhere the watch holds the whole tree.
 */
export class FollowedFolders {
  #folders: ReadonlySet<string> = new Set([""]);

  /** From the root folder alone, as a watch opened before a walk names its folders. */
  restart(): void {
    this.#folders = new Set([""]);
  }

  /**
   * Has the engine follow `walked`, the folders a walk walked into, through `watch` (false when it refuses), unless it
   * follows them already: "added" when one was not followed (a write in it before now was told by nothing), "followed"
   * when only folders went, "refused" when the engine refused them, undefined when nothing changed.
   */
  follow(walked: ReadonlySet<string>, watch: (folders: readonly string[]) => boolean): "added" | "followed" | "refused" | undefined {
    const added = [...walked].some(folder => !this.#folders.has(folder));
    if (!added && walked.size === this.#folders.size) return undefined;
    if (!watch([...walked])) return "refused";
    this.#folders = walked;
    return added ? "added" : "followed";
  }

  /** Lets go of what the engine let go of by `notice`: every folder after lost notices, else each that came or went. */
  noticed(notice: TreeNotice): void {
    if (notice.lost) this.#folders = new Set();
    else this.#replaced(notice.renamed);
  }

  /** Drops each followed folder that came or went (`renamed`), and every folder under it. */
  #replaced(renamed: readonly string[]): void {
    const gone = renamed.filter(path => this.#folders.has(path));
    if (gone.length === 0) return;
    this.#folders = new Set([...this.#folders].filter(folder => !gone.some(path => folder === path || folder.startsWith(`${path}/`))));
  }
}

interface Watched {
  /** The folders the watch follows (Linux). */
  readonly followed: FollowedFolders;
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
  /** The watched worktrees a notice counted in since a read of each began, until `noticed` hands them on. */
  readonly #noticed = new Set<string>();

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
    for (const root of this.#watched.keys()) {
      if (roots.has(root)) continue;
      this.#watched.delete(root);
      this.#watch?.unwatch(root);
    }
    forgetOthers(this.#failed, roots);
    forgetOthers(this.#ended, roots);
    forgetOthers(this.#noticed, roots);
  }

  /** Starts watching `root`, its root folder alone until a read names its folders; undefined when it cannot be watched now. */
  #open(root: string): Watched | undefined {
    if (this.#watch === undefined || this.#tooSoon(root)) return undefined;
    this.#failed.delete(root);
    this.#ended.delete(root);
    // Up before the read: a change made while it runs is a notice.
    if (!this.#watch.watch(root, [""])) {
      this.#failed.set(root, this.#now());
      return undefined;
    }
    const watched: Watched = { followed: new FollowedFolders(), seen: undefined, changed: true };
    this.#watched.set(root, watched);
    return watched;
  }

  /** Whether `root` could not be watched, or its watch ended, too lately to ask for it again now. */
  #tooSoon(root: string): boolean {
    const failedAt = this.#failed.get(root);
    if (failedAt !== undefined && this.#now() - failedAt < WATCH_RETRY_MS) return true;
    const endedAt = this.#ended.get(root);
    return endedAt !== undefined && this.#now() - endedAt < REWATCH_MS;
  }

  /**
   * Watches the folders the read walked into, in place of those of the last. A folder watched now was not while the
   * read ran, so the next walk reads the worktree again. One the watch cannot follow (past the watch budget) is read on
   * every walk until the retry.
   */
  #follow(root: string, watched: Watched, folders: ReadonlySet<string>): void {
    const followed = watched.followed.follow(folders, list => this.#watch!.watch(root, list));
    if (followed === "added") watched.changed = true;
    if (followed !== "refused") return;
    this.#watched.delete(root);
    this.#watch!.unwatch(root);
    this.#failed.set(root, this.#now());
  }

  #notice(notice: TreeNotice): void {
    const watched = this.#watched.get(notice.root);
    if (watched === undefined) return;
    // The watch let go (the worktree or a folder above it moved or went, a watch error): read it whole, and watch it
    // again as the next walk asks.
    if (notice.ended) {
      this.#watched.delete(notice.root);
      this.#ended.set(notice.root, this.#now());
    }
    // The next read's folders are watched again where the engine let go of them; it keeps the watches it holds.
    if (this.#byFolder) watched.followed.noticed(notice);
    if (marksChanged(watched, notice)) {
      watched.changed = true;
      this.#noticed.add(notice.root);
    }
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

/**
 * How a pass of noticed paths looks at a path: by its stamp alone ("stamp": a file with another name no notice named,
 * spelled as a walk found it), or by its stamp and then its spelling ("stamp-and-spelling": a path a notice named, which
 * may spell a file otherwise than the folder holds it).
 */
export type NoticedRead = "stamp" | "stamp-and-spelling";

/**
 * What a pass of the session folder reads: every file, or only `paths`, each with how to look at it: those change notices
 * named since the last pass, and the files with another name a walk found. `linked` takes the files with another name
 * the pass finds (`LinkedFiles`).
 */
export type SessionReads =
  | { readonly whole: true; readonly linked: LinkedFiles }
  | { readonly whole: false; readonly paths: ReadonlyMap<string, NoticedRead>; readonly linked: LinkedFiles };

/** What a pass of the session folder found, as it ended (`SessionNotices.ended`). */
export interface SessionFound {
  /** The files with another name (a hard link) it found: its reads' `linked`. */
  readonly linked: LinkedFiles;
  /** How many files it looked at. */
  readonly files: number;
}

/**
 * The files with another name (a hard link) one pass of the session folder finds, as it finds them (`SessionReads`),
 * each noted while they stay under half of what the last whole walk looked at. Once they reach that bound, a pass of
 * noticed paths reads the whole folder anyway (`SessionNotices`), so no more are noted and only that fact is kept: in a
 * checkout whose files a copy made by links shares, a whole walk notes no more than half its files, and keeps none.
 */
export class LinkedFiles {
  readonly #looks: number;
  /** The files noted; undefined once they reached the bound. */
  #paths: string[] | undefined = [];

  /** `looks`: how many files and folders the last whole walk looked at; infinite before the first, which notes every one. */
  constructor(looks: number) {
    this.#looks = looks;
  }

  /** Notes `path`, a file with another name, unless the files noted reached the bound. */
  note(path: string): void {
    if (this.#paths === undefined) return;
    if (2 * (this.#paths.length + 1) >= this.#looks) this.#paths = undefined;
    else this.#paths.push(path);
  }

  /** Forgets the files noted: a pass of noticed paths that falls back to a whole walk notes them all again. */
  restart(): void {
    this.#paths = [];
  }

  /** The files noted, none once they reached the bound. */
  get paths(): readonly string[] {
    return this.#paths ?? [];
  }

  /**
   * The files noted, for a whole walk that looked at `looks` files and folders; undefined once they reached the bound,
   * or reach half of `looks`, which the first whole walk only knows as it ends.
   */
  keep(looks: number): Set<string> | undefined {
    return this.#paths === undefined || 2 * this.#paths.length >= looks ? undefined : new Set(this.#paths);
  }
}

/**
 * The change notices of one session folder (`CodeRuleBridge`), taken at the start of each pass of its walk: which
 * paths a shell command's result must read again, so the result reads those, in place of every file of the folder.
 *
 * The folder is watched from the session's first walk on, through the engine's watch the walks of the other worktrees
 * take their notices from (`TreeWatch`): on Linux folder by folder, as the last whole walk walked them, elsewhere the
 * whole tree. A pass reads the whole folder whenever the notices cannot vouch for what they leave out: until a whole
 * walk finished with the watch up, when the folder cannot be watched (a network or WSL path, past Linux's watch budget),
 * after the watch ended or lost notices, after Linux's watch took up folders the last walk found (a write in one before
 * its watch is told by nothing), after a pass that did not finish, and when a notice names what changes which files a
 * walk reads: a `.gitignore`, a `.git` entry, a folder that came or went, or a path in a folder no walk walked. On
 * Linux a folder that came or went is followed again from the next whole walk on (`FollowedFolders`), which the pass
 * after reads whole too. A file a pass found with another name (a hard link) can be written through a name outside the
 * folder, which no notice of the folder tells of: a pass of noticed paths reads each such file, too. A pass that would
 * read half as many paths as the last whole walk looked at, or more (a checkout whose files a copy made by links shares,
 * a command that wrote much of the folder), reads the whole folder instead (`#noticed`); a whole walk stops noting the
 * files with another name once they reach that bound (`LinkedFiles`). A watch the engine
 * refused is not asked for again until a whole walk walks fewer folders than it refused, or the watch it had ended:
 * each ask costs the engine a look at the folder's file system, and on Linux each watch it took up before the refusal.
 *
 * What no notice tells of, the next whole pass reads: a write through a hard link made to a name outside the folder
 * after the last whole walk, a write through a memory map (on Linux only one its program still holds at the result: the
 * engine tells of one let go of), and on Windows a write that keeps the size by a program that still holds the file open
 * (Windows tells of it as the program closes the file). A pass reads whole once the last whole walk began more than
 * `fullWalkMs` ago, so such a write waits for the first result past that, or for the turn's end if it comes first.
 */
export class SessionNotices {
  readonly #root: string;
  /** The engine's watch the notices come through, once the session's first walk of the other worktrees gave one. */
  #watch: TreeWatch | undefined;
  /** Whether the watch follows the folders a walk walked (Linux), rather than the whole tree. */
  readonly #byFolder: boolean;
  /** Whether the watch holds the folder now. */
  #watching = false;
  /** The folders the watch follows (Linux). */
  readonly #followed = new FollowedFolders();
  /** What the last whole walk saw, which notices are weighed against; undefined before the first and while one runs. */
  #seen: WorktreeRead | undefined;
  /** Whether the next pass reads the whole folder. */
  #whole = true;
  /** The paths notices named since the last pass began. */
  #paths = new Set<string>();
  /**
   * The files with another name a pass found since the last whole one (`ended`), read again at every pass; undefined
   * when the last whole walk found them at the bound or past it, which is all it kept of them (`LinkedFiles`).
   */
  #linked: Set<string> | undefined = new Set<string>();
  /**
   * How many files and folders the last whole walk looked at, which bounds what a pass of noticed paths reads; infinite
   * before the first.
   */
  #looks = Number.POSITIVE_INFINITY;
  /** How many folders the engine last refused to watch: none is asked for again until a whole walk walks fewer. */
  #refused: number | undefined;
  /** The longest a write no notice tells of waits for a whole walk, from when the last one began. */
  readonly #fullWalkMs: number;
  readonly #now: () => number;
  /** When the pass that began last began. */
  #began = Number.NEGATIVE_INFINITY;
  /** When the last whole walk that finished began. */
  #walkedAt = Number.NEGATIVE_INFINITY;

  /**
   * `root`: the session folder (physical). `fullWalkMs`: how long after the last whole walk began a pass reads whole
   * again, whatever the notices say. `now`: the clock that is read by, `Date.now` (looked up at each read) by default.
   */
  constructor(root: string, options: { fullWalkMs: number; platform?: NodeJS.Platform; now?: () => number }) {
    this.#root = root;
    this.#byFolder = (options.platform ?? process.platform) === "linux";
    this.#fullWalkMs = options.fullWalkMs;
    this.#now = options.now ?? ((): number => Date.now());
  }

  /**
   * The engine's watch to take the notices through, none when the engine has none; the first one given stays. The
   * folder is watched from the next whole pass on, which reads it whole once the watch is up.
   */
  use(watch: TreeWatch | undefined): void {
    this.#watch ??= watch;
  }

  /**
   * What the pass that begins now reads: the whole folder when `#readsWhole` says so or the paths to read reach the bound
   * (`#noticed`), else the paths notices named since the last pass and the linked files. A whole pass watches the
   * folder again first when its watch is down, so a write made while it runs is a notice. Until `ended`, the next pass
   * reads whole: one that stops midway leaves unread what it took.
   */
  begin(whole: boolean): SessionReads {
    this.#collect();
    const began = this.#now();
    const paths = this.#readsWhole(whole, began) ? undefined : this.#noticed();
    this.#began = began;
    this.#paths = new Set();
    this.#whole = true;
    // A pass of noticed paths reads fewer paths than the bound, so it notes them all.
    const linked = new LinkedFiles(this.#looks);
    if (paths !== undefined) return { whole: false, paths, linked };
    this.#seen = undefined;
    this.#open();
    return { whole: true, linked };
  }

  /**
   * Whether the pass that begins at `now` reads the whole folder, whatever the notices named: when `whole` asks for it,
   * when the notices cannot vouch for the rest, and once the last whole walk began more than `fullWalkMs` ago. That last
   * is a clock, not a fact, because no notice and no count can tell of the writes it bounds (see the class).
   */
  #readsWhole(whole: boolean, now: number): boolean {
    return whole || this.#whole || this.#seen === undefined || now - this.#walkedAt > this.#fullWalkMs;
  }

  /**
   * The paths notices named and the linked files, each with how to look at it; none, so the whole folder, once they
   * reach half of what the last whole walk looked at. A pass looks at a named path twice (its stamp, then its spelling)
   * and at a linked one once, each by a call of its own, where the walk looks at each file and folder once and takes a
   * folder's names from one listing: under the bound a pass looks at no more paths than the walk would, and at half as
   * many for linked files.
   */
  #noticed(): ReadonlyMap<string, NoticedRead> | undefined {
    const [named, linked] = [this.#paths, this.#linked];
    // The last whole walk found the linked files at the bound, and kept only that.
    if (linked === undefined) return undefined;
    // Counted before any map is built: past the bound the pass reads whole, however many files are linked.
    const linkedOnly = linked.size - [...named].filter(path => linked.has(path)).length;
    if (2 * (named.size + linkedOnly) >= this.#looks) return undefined;
    const paths = new Map<string, NoticedRead>();
    for (const path of linked) paths.set(path, "stamp");
    for (const path of named) paths.set(path, "stamp-and-spelling");
    return paths;
  }

  /**
   * Paths the pass that began last left to the pass after (a file a shell command of the session may have been part
   * way through writing as it read it), whose notices that pass took: the next pass reads them as it reads the paths
   * notices named since.
   */
  readAgain(paths: Iterable<string>): void {
    for (const path of paths) this.#paths.add(path);
  }

  /**
   * The pass that began last finished: a whole one with what it `seen`, which notices are weighed against from now on
   * (on Linux the watch follows its folders) and the period runs from as it began, one that read only noticed paths with
   * nothing. `found`: the files with another name it found, and how many files it looked at.
   */
  ended(seen: WorktreeRead | undefined, found: SessionFound): void {
    if (seen === undefined) {
      this.#endedNoticed(found.linked);
      return;
    }
    this.#walkedAt = this.#began;
    this.#looks = found.files + seen.folders.size;
    this.#linked = found.linked.keep(this.#looks);
    if (this.#refused !== undefined && seen.folders.size < this.#refused) this.#refused = undefined;
    this.#seen = seen;
    this.#whole = this.#follow(seen.folders);
  }

  /**
   * A pass of noticed paths finished: the files with another name it found join those kept, and the next pass reads by
   * the notices again. Such a pass runs only while the linked files are kept (`#noticed`).
   */
  #endedNoticed(linked: LinkedFiles): void {
    for (const path of linked.paths) this.#linked?.add(path);
    this.#whole = false;
  }

  /** Lets go of the folder and of the watch. */
  close(): void {
    this.#watching = false;
    this.#watch?.close();
  }

  #open(): void {
    if (this.#watch === undefined || this.#watching || this.#refused !== undefined) return;
    this.#followed.restart();
    this.#watching = this.#watchFolders([""]);
  }

  /** Watches the folder through `folders`; a refusal stands until a whole walk walks fewer folders (`ended`). */
  #watchFolders(folders: readonly string[]): boolean {
    const watching = this.#watch!.watch(this.#root, folders);
    if (!watching) this.#refused = folders.length;
    return watching;
  }

  /**
   * On Linux, watches the folders the whole walk walked into, in place of those before; true when the watch is down, one
   * is new, or the watch refused them: a write made in it before its watch was told by nothing, so the next pass reads
   * whole.
   */
  #follow(folders: ReadonlySet<string>): boolean {
    if (!this.#watching) return true;
    if (!this.#byFolder) return false;
    const followed = this.#followed.follow(folders, list => this.#watchFolders(list));
    if (followed === "refused") this.#watching = false;
    return followed === "added" || followed === "refused";
  }

  #collect(): void {
    if (!this.#watching) return;
    for (const notice of this.#watch!.take()) if (notice.root === this.#root) this.#notice(notice);
  }

  #notice(notice: TreeNotice): void {
    // A folder the engine let go of is watched again by the next whole pass, which walks it.
    if (this.#byFolder) this.#followed.noticed(notice);
    // A watch that ended is asked for again at the next whole pass, whatever was refused before.
    if (notice.ended) [this.#watching, this.#refused] = [false, undefined];
    if (notice.ended || notice.lost || this.#seen === undefined) this.#whole = true;
    else this.#named(this.#seen, notice);
  }

  /** Takes each path `notice` names, unless the last whole walk left it out (`noticeCounts`). */
  #named(seen: WorktreeRead, notice: TreeNotice): void {
    for (const path of notice.renamed) this.#take(seen, "rename", path);
    for (const path of notice.changed) this.#take(seen, "change", path);
  }

  #take(seen: WorktreeRead, event: "change" | "rename", path: string): void {
    if (!noticeCounts(seen, event, path)) return;
    if (readsWhole(seen, event, path)) this.#whole = true;
    else this.#paths.add(path);
  }
}

/**
 * Whether a notice of `event` for `path` changes which files a walk reads: a `.gitignore` or a `.git` entry, a folder
 * the walk walked that came or went, or a path in a folder it did not walk.
 */
function readsWhole(seen: WorktreeRead, event: "change" | "rename", path: string): boolean {
  const slash = path.lastIndexOf("/");
  const name = path.slice(slash + 1);
  if (name === ".gitignore" || name === ".git") return true;
  if (event === "rename" && seen.folders.has(path)) return true;
  return !seen.folders.has(slash < 0 ? "" : path.slice(0, slash));
}
