import { stat } from "node:fs/promises";
import { fileStamp } from "../file-stamp.js";
import {
  auditGitlinks, auditStaged, auditUnstaged, auditUntracked, batchBlobs, checkoutLocation, deletedFiles, filesOnDisk,
  gitBranchTips, gitCommitsBetween, gitConfigFiles, gitCrlfPaths, gitExcludeFiles, gitHead, gitIndexPath,
  gitModifiedOrUntrackedPaths, gitStartCounts, headMovedOnlyByCommits, legacyRepositoryId, originUrl, readGitBlobIds,
  remoteNames, remoteUrl, scanChanged, scanCommit, scanEmptyTree, scanHead, scanHeadRef, scanHolds, scanMergeBase,
  scanTopLevel, scanTreeEntries, scanUntracked, submoduleGitlinks, submodulePaths, unmergedEntries, worktreeList,
  type BlobLimits, type CrlfRule, type GitAccessCounts, type GitPlace, type GitReadOptions, type GitResult,
} from "./git-process.js";

export type {
  BlobLimits, CrlfRule, GitAccessCounts, GitPlace, GitQuestion, GitReadOptions, GitResult, QuestionStarts,
} from "./git-process.js";
export { gitBash, GitUnavailableError, requireGit, searchGitFrom } from "./find-git.js";

/**
 * The questions the `rerail scan` command asks Git about the checkout it scans, each blocking until Git exits (G22 in
 * the Git access inventory). The ones that return a string throw, naming the command, when Git fails.
 */
export interface ScanGit {
  topLevel(folder: string): string | null;
  commit(root: string, ref: string): string | null;
  holds(root: string, revision: string): boolean;
  treeEntries(root: string, revision: string): string;
  head(root: string): string | null;
  headRef(root: string): string | null;
  emptyTree(root: string): string;
  mergeBase(root: string, base: string): string | null;
  changed(root: string, against: string): string;
  untracked(root: string): string;
}

/**
 * Every question Rerail asks Git about a checkout, in Rerail's words. Each one is answered by starting Git, as it was
 * before this interface, with the same command, arguments, environment and time limit; `counts()` says how many Git
 * processes each question started. The questions that take a {@link GitReadOptions} do not block and end once its
 * signal aborts; the others block the event loop until Git exits. The G numbers are the Git access inventory's.
 */
export interface CheckoutGit {
  /** G1: the top level, common directory and Git directory of the checkout around `folder`, one a line. */
  checkoutLocation(folder: string): GitResult;
  /** G2: the repository's main checkout and linked worktrees, as `git worktree list --porcelain` prints them. */
  worktreeList(checkout: string): GitResult;
  /** G3: the clone id an older Rerail kept in the checkout's local config. */
  legacyRepositoryId(root: string): string | undefined;
  /** G4: `origin`'s address. */
  originUrl(root: string): string | undefined;
  /** G4: the names of the checkout's remotes, one a line. */
  remoteNames(root: string): string | undefined;
  /** G4: the address of the remote `name`. */
  remoteUrl(root: string, name: string): string | undefined;
  /** G5: each path `.gitmodules` declares a submodule at. */
  submodulePaths(root: string): GitResult;
  /** G5: the index entries at `paths`, so a caller keeps the gitlinks. */
  submoduleGitlinks(root: string, paths: readonly string[]): GitResult;
  /** G6: the commit `HEAD` names; undefined outside a repository, or before its first commit. */
  head(cwd: string, read?: GitReadOptions): Promise<string | undefined>;
  /** G8: the file the checkout keeps its index in; undefined when Git cannot say. */
  indexFile(cwd: string, read?: GitReadOptions): Promise<string | undefined>;
  /** G9: the index's unmerged entries by path; undefined when Git cannot say. */
  unmergedEntries(cwd: string, read?: GitReadOptions): Promise<ReadonlyMap<string, string> | undefined>;
  /** G10: whether everything that moved `HEAD` since it named `from` was a commit. */
  movedOnlyByCommits(cwd: string, from: string | undefined, read?: GitReadOptions): Promise<boolean>;
  /** G11: the contents of `revisions` that name a blob no larger than the limit. */
  blobs(cwd: string, revisions: readonly string[], limits: BlobLimits & GitReadOptions): Promise<Map<string, Buffer>>;
  /** G11, G14: the object id of each of `revisions` that names a blob. */
  blobIds(cwd: string, revisions: readonly string[], read?: GitReadOptions): Promise<Map<string, string>>;
  /** G12, G13: the paths a checkout writes with CRLF line breaks, with the rule of each. */
  crlfPaths(cwd: string, paths: readonly string[], options?: { platform?: NodeJS.Platform } & GitReadOptions): Promise<Map<string, CrlfRule>>;
  /** G15: the distinct commits the branches and remote-tracking branches point at. */
  branchTips(cwd: string, read?: GitReadOptions): Promise<string[]>;
  /** G16: the commits reachable from `to` but not from `from`. */
  commitsBetween(cwd: string, from: string, to: string, read?: GitReadOptions): Promise<string[]>;
  /** G17: the files whose text the index may not hold: modified since staged, and untracked but not ignored. */
  modifiedOrUntracked(cwd: string, read?: GitReadOptions): Promise<string[]>;
  /** G18: the exclude files no diff shows. */
  excludeFiles(cwd: string, read?: GitReadOptions): Promise<string[]>;
  /** G19: the configuration files that can name the excludes file. */
  configFiles(cwd: string, place?: GitPlace & GitReadOptions): Promise<string[]>;
  /** G20: every file on disk under Git, each after its status tag; undefined without Git or when Git fails. */
  filesOnDisk(root: string): string[] | undefined;
  /** G20: the tracked files deleted from the working tree. */
  deletedFiles(root: string): string[] | undefined;
  /** G21: the index entries at `folders`; throws when Git fails, as each audit question does. */
  auditGitlinks(root: string, folders: readonly string[]): string[];
  /** G21: the files changed since staged. */
  auditUnstaged(root: string): string[];
  /** G21: the files staged since `HEAD`. */
  auditStaged(root: string): string[];
  /** G21: the untracked files that are not ignored. */
  auditUntracked(root: string): string[];
  /** G22: the scan command's questions. */
  readonly scan: ScanGit;
  /** The Git processes this process started, in all and by question. */
  counts(): GitAccessCounts;
}

/** This process's one {@link CheckoutGit}: the daemon's for every runtime, session and walk in it. */
export const checkoutGit: CheckoutGit = {
  checkoutLocation,
  worktreeList,
  legacyRepositoryId,
  originUrl,
  remoteNames,
  remoteUrl,
  submodulePaths,
  submoduleGitlinks,
  head: gitHead,
  indexFile: (cwd, read = {}) => gitIndexPath(cwd, read.signal),
  unmergedEntries: (cwd, read = {}) => unmergedEntries(cwd, read.signal),
  movedOnlyByCommits: headMovedOnlyByCommits,
  blobs: batchBlobs,
  blobIds: readGitBlobIds,
  crlfPaths: gitCrlfPaths,
  branchTips: gitBranchTips,
  commitsBetween: gitCommitsBetween,
  modifiedOrUntracked: gitModifiedOrUntrackedPaths,
  excludeFiles: gitExcludeFiles,
  configFiles: gitConfigFiles,
  filesOnDisk,
  deletedFiles,
  auditGitlinks,
  auditUnstaged,
  auditStaged,
  auditUntracked,
  scan: {
    topLevel: scanTopLevel,
    commit: scanCommit,
    holds: scanHolds,
    treeEntries: scanTreeEntries,
    head: scanHead,
    headRef: scanHeadRef,
    emptyTree: scanEmptyTree,
    mergeBase: scanMergeBase,
    changed: scanChanged,
    untracked: scanUntracked,
  },
  counts: gitStartCounts,
};

/**
 * The Git reads one owner makes without waiting for each, such as a session's source walks and ignore rules, or a
 * headless scan's read of the rules a ref holds: a handle onto a {@link CheckoutGit}, the process's own unless a test
 * passes another. Each method is one read, and each starts its Git under the signal {@link stop} aborts. So
 * {@link stop} kills the reads still running and settles once each Git has exited, and the owner can end with no Git
 * left running in its checkouts; an owner with nothing to stop, such as the headless scan, never calls it.
 * Once stopped, a read starts no Git and answers as without Git. It also keeps what {@link unmerged} last read of each
 * folder, held while that folder's index file is unchanged.
 */
export class GitReads {
  readonly #git: CheckoutGit;
  readonly #stopping = new AbortController();
  readonly #running = new Set<Promise<unknown>>();
  readonly #unmerged: UnmergedReader;

  constructor(git: CheckoutGit = checkoutGit) {
    this.#git = git;
    this.#unmerged = gitUnmergedReader(git);
  }

  /** The commit `HEAD` names in `cwd`; undefined outside a repository, or before its first commit. */
  head(cwd: string): Promise<string | undefined> {
    return this.#run(signal => this.#git.head(cwd, { signal }));
  }

  /** Whether everything that moved `HEAD` in `cwd` since it named `from` was a commit (`headMovedOnlyByCommits`). */
  movedOnlyByCommits(cwd: string, from: string | undefined): Promise<boolean> {
    return this.#run(signal => this.#git.movedOnlyByCommits(cwd, from, { signal }));
  }

  /** The contents of `revisions` in `cwd` by one `git cat-file --batch` (`batchBlobs`). */
  blobs(cwd: string, revisions: readonly string[], limits: BlobLimits): Promise<Map<string, Buffer>> {
    return this.#run(signal => this.#git.blobs(cwd, revisions, { ...limits, signal }));
  }

  /** The object id of each of `revisions` that names a blob, no blob read (`readGitBlobIds`). */
  blobIds(cwd: string, revisions: readonly string[]): Promise<Map<string, string>> {
    return this.#run(signal => this.#git.blobIds(cwd, revisions, { signal }));
  }

  /** The paths among `paths` a checkout of `cwd` writes with CRLF line breaks, with the rule of each (`gitCrlfPaths`). */
  crlfPaths(cwd: string, paths: readonly string[], options: { platform?: NodeJS.Platform } = {}): Promise<Map<string, CrlfRule>> {
    return this.#run(signal => this.#git.crlfPaths(cwd, paths, { ...options, signal }));
  }

  /** The distinct commits `cwd`'s branches and remote-tracking branches point at (`gitBranchTips`). */
  branchTips(cwd: string): Promise<string[]> {
    return this.#run(signal => this.#git.branchTips(cwd, { signal }));
  }

  /** The commits reachable from `to` but not from `from` (`gitCommitsBetween`). */
  commitsBetween(cwd: string, from: string, to: string): Promise<string[]> {
    return this.#run(signal => this.#git.commitsBetween(cwd, from, to, { signal }));
  }

  /** The files of `cwd`'s checkout whose text its index may not hold (`gitModifiedOrUntrackedPaths`). */
  modifiedOrUntracked(cwd: string): Promise<string[]> {
    return this.#run(signal => this.#git.modifiedOrUntracked(cwd, { signal }));
  }

  /** The exclude files of `cwd`'s checkout that no diff shows (`gitExcludeFiles`). */
  excludeFiles(cwd: string): Promise<string[]> {
    return this.#run(signal => this.#git.excludeFiles(cwd, { signal }));
  }

  /** The configuration files that can name `cwd`'s excludes file (`gitConfigFiles`). */
  configFiles(cwd: string, place: GitPlace = {}): Promise<string[]> {
    return this.#run(signal => this.#git.configFiles(cwd, { ...place, signal }));
  }

  /**
   * The index's unmerged entries under `cwd` ({@link gitUnmergedReader}): a Git process runs only when the folder's
   * index file changed since the last read; undefined when Git cannot say.
   */
  unmerged(cwd: string): Promise<ReadonlyMap<string, string> | undefined> {
    return this.#run(signal => this.#unmerged(cwd, { signal }));
  }

  /** Kills every read still running and settles once the Git each started has exited. */
  async stop(): Promise<void> {
    this.#stopping.abort();
    await Promise.allSettled([...this.#running]);
  }

  /** Runs `read` with the signal {@link stop} aborts, and keeps it among the reads `stop` waits for until it settles. */
  #run<T>(read: (signal: AbortSignal) => Promise<T>): Promise<T> {
    const running = read(this.#stopping.signal);
    this.#running.add(running);
    const forget = (): void => {
      this.#running.delete(running);
    };
    running.then(forget, forget);
    return running;
  }
}

type UnmergedReader = (cwd: string, options?: GitReadOptions) => Promise<ReadonlyMap<string, string> | undefined>;

/**
 * Reads the index's unmerged entries under a folder (`unmergedEntries`), for a caller that asks every pass: it
 * starts a Git process only when the folder's index file changed since the last read, and otherwise costs one `stat`
 * (plus one Git process the first time, to find the file). Git replaces that file with a new one on every write, so a
 * write changes its inode even inside one clock tick. A folder whose file cannot be found or stamped is read every time,
 * so this can cost more, never answer with stale entries. When Git cannot list them (it failed, timed out or was
 * stopped), the answer is undefined and nothing is kept, so the next read asks Git again.
 */
function gitUnmergedReader(git: CheckoutGit): UnmergedReader {
  const indexes = new Map<string, string>();
  const held = new Map<string, { readonly stamp: string; readonly entries: ReadonlyMap<string, string> }>();
  /** The stamp of `cwd`'s index file, found once and remembered; undefined when it cannot be found or stamped. */
  const indexStamp = async (cwd: string, signal: AbortSignal | undefined): Promise<string | undefined> => {
    const index = indexes.get(cwd) ?? await git.indexFile(cwd, { signal });
    if (index === undefined) return undefined;
    indexes.set(cwd, index);
    try {
      return fileStamp(await stat(index));
    } catch {
      return undefined;
    }
  };
  return async (cwd, options = {}) => {
    // Stamped before the read: an index written between the two reads as changed next time.
    const stamp = await indexStamp(cwd, options.signal);
    const last = held.get(cwd);
    if (last !== undefined && last.stamp === stamp) return last.entries;
    held.delete(cwd);
    const entries = await git.unmergedEntries(cwd, { signal: options.signal });
    if (entries !== undefined && stamp !== undefined) held.set(cwd, { stamp, entries });
    return entries;
  };
}
