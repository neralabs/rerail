import { stat } from "node:fs/promises";
import { fileStamp } from "../file-stamp.js";
import {
  batchBlobs, gitBranchTips, gitCommitsBetween, gitConfigFiles, gitCrlfPaths, gitExcludeFiles, gitHead, gitIndexPath,
  gitModifiedOrUntrackedPaths, headMovedOnlyByCommits, readGitBlobIds, unmergedEntries,
  type BlobLimits, type CrlfRule, type GitPlace, type GitReadOptions,
} from "./git-process.js";

/**
 * The Git reads one owner makes without waiting for each, such as a session's source walks and ignore rules, or a
 * headless scan's read of the rules a ref holds: each method is one read, and each starts its Git under the signal
 * {@link stop} aborts. So {@link stop} kills the reads still running and settles once each Git has exited, and the
 * owner can end with no Git left running in its checkouts; an owner with nothing to stop, such as the headless scan,
 * never calls it.
 * Once stopped, a read starts no Git and answers as without Git. It also keeps what {@link unmerged} last read of each
 * folder, held while that folder's index file is unchanged.
 */
export class GitReads {
  readonly #stopping = new AbortController();
  readonly #running = new Set<Promise<unknown>>();
  readonly #unmerged = gitUnmergedReader();

  /** The commit `HEAD` names in `cwd`; undefined outside a repository, or before its first commit. */
  head(cwd: string): Promise<string | undefined> {
    return this.#run(signal => gitHead(cwd, { signal }));
  }

  /** Whether everything that moved `HEAD` in `cwd` since it named `from` was a commit ({@link headMovedOnlyByCommits}). */
  movedOnlyByCommits(cwd: string, from: string | undefined): Promise<boolean> {
    return this.#run(signal => headMovedOnlyByCommits(cwd, from, { signal }));
  }

  /** The contents of `revisions` in `cwd` by one `git cat-file --batch` ({@link batchBlobs}). */
  blobs(cwd: string, revisions: readonly string[], limits: BlobLimits): Promise<Map<string, Buffer>> {
    return this.#run(signal => batchBlobs(cwd, revisions, { ...limits, signal }));
  }

  /** The object id of each of `revisions` that names a blob, no blob read ({@link readGitBlobIds}). */
  blobIds(cwd: string, revisions: readonly string[]): Promise<Map<string, string>> {
    return this.#run(signal => readGitBlobIds(cwd, revisions, { signal }));
  }

  /** The paths among `paths` a checkout of `cwd` writes with CRLF line breaks, with the rule of each ({@link gitCrlfPaths}). */
  crlfPaths(cwd: string, paths: readonly string[], options: { platform?: NodeJS.Platform } = {}): Promise<Map<string, CrlfRule>> {
    return this.#run(signal => gitCrlfPaths(cwd, paths, { ...options, signal }));
  }

  /** The distinct commits `cwd`'s branches and remote-tracking branches point at ({@link gitBranchTips}). */
  branchTips(cwd: string): Promise<string[]> {
    return this.#run(signal => gitBranchTips(cwd, { signal }));
  }

  /** The commits reachable from `to` but not from `from` ({@link gitCommitsBetween}). */
  commitsBetween(cwd: string, from: string, to: string): Promise<string[]> {
    return this.#run(signal => gitCommitsBetween(cwd, from, to, { signal }));
  }

  /** The files of `cwd`'s checkout whose text its index may not hold ({@link gitModifiedOrUntrackedPaths}). */
  modifiedOrUntracked(cwd: string): Promise<string[]> {
    return this.#run(signal => gitModifiedOrUntrackedPaths(cwd, { signal }));
  }

  /** The exclude files of `cwd`'s checkout that no diff shows ({@link gitExcludeFiles}). */
  excludeFiles(cwd: string): Promise<string[]> {
    return this.#run(signal => gitExcludeFiles(cwd, { signal }));
  }

  /** The configuration files that can name `cwd`'s excludes file ({@link gitConfigFiles}). */
  configFiles(cwd: string, place: GitPlace = {}): Promise<string[]> {
    return this.#run(signal => gitConfigFiles(cwd, { ...place, signal }));
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

/**
 * Reads the index's unmerged entries under a folder ({@link unmergedEntries}), for a caller that asks every pass: it
 * starts a Git process only when the folder's index file changed since the last read, and otherwise costs one `stat`
 * (plus one Git process the first time, to find the file). Git replaces that file with a new one on every write, so a
 * write changes its inode even inside one clock tick. A folder whose file cannot be found or stamped is read every time,
 * so this can cost more, never answer with stale entries. When Git cannot list them (it failed, timed out or was
 * stopped), the answer is undefined and nothing is kept, so the next read asks Git again.
 */
function gitUnmergedReader(): (cwd: string, options?: GitReadOptions) => Promise<ReadonlyMap<string, string> | undefined> {
  const indexes = new Map<string, string>();
  const held = new Map<string, { readonly stamp: string; readonly entries: ReadonlyMap<string, string> }>();
  /** The stamp of `cwd`'s index file, found once and remembered; undefined when it cannot be found or stamped. */
  const indexStamp = async (cwd: string, signal: AbortSignal | undefined): Promise<string | undefined> => {
    const index = indexes.get(cwd) ?? await gitIndexPath(cwd, signal);
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
    const entries = await unmergedEntries(cwd, options.signal);
    if (entries !== undefined && stamp !== undefined) held.set(cwd, { stamp, entries });
    return entries;
  };
}
