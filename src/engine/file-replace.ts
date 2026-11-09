/**
 * Replacing and removing files where Windows holds them open.
 *
 * On Windows, renaming a file onto or from one another process holds open fails with EPERM, EACCES or EBUSY: a
 * reader of `daemon.json`, an editor or language server with a source file open, or antivirus and the indexer scanning
 * a file just written. Removing a folder fails the same way while a file in it is held. Each hold lasts a moment, so
 * these helpers retry there; elsewhere the first failure stands, as a rename over a file never fails for that reason.
 * Every rename in `src/` goes through them: `npm run check:owned-builtins` fails on any other.
 */
import { chmodSync, renameSync as fsRenameSync, statSync } from "node:fs";
import { chmod, rename as fsRename, rm as fsRm, stat } from "node:fs/promises";
import { setTimeout as sleep } from "node:timers/promises";

/** How long a rename is retried on Windows. */
const RENAME_RETRY_MS = 2_000;
const RENAME_RETRY_INTERVAL_MS = 50;
const HELD: ReadonlySet<unknown> = new Set(["EPERM", "EACCES", "EBUSY"]);

export interface RenameOptions {
  platform?: NodeJS.Platform;
  rename?: (from: string, to: string) => Promise<void>;
  retryMs?: number;
}

const codeOf = (error: unknown): unknown => (error instanceof Error && "code" in error ? error.code : undefined);

/**
 * Renames `from` over `to`, retried on Windows for {@link RENAME_RETRY_MS} ms while either file is held. A read-only
 * `to` is made writable first there, since Windows refuses to replace a read-only file, which mode bits elsewhere never
 * stop; the file moved in keeps its own mode. A folder at `to` fails at once: Windows never renames over one, so
 * waiting would only delay the error.
 */
export async function renameWithRetry(from: string, to: string, options: RenameOptions = {}): Promise<void> {
  const rename = options.rename ?? fsRename;
  const windows = (options.platform ?? process.platform) === "win32";
  const deadline = Date.now() + (options.retryMs ?? RENAME_RETRY_MS);
  for (let attempt = 1; ; attempt += 1) {
    try {
      await rename(from, to);
      return;
    } catch (error) {
      if (!windows || !HELD.has(codeOf(error)) || Date.now() >= deadline) throw error;
      if (attempt === 1 && await makeWritable(to) === "folder") throw error;
      await sleep(RENAME_RETRY_INTERVAL_MS);
    }
  }
}

export interface RenameSyncOptions extends Omit<RenameOptions, "rename"> {
  rename?: (from: string, to: string) => void;
}

/** Blocks this thread for `ms` milliseconds; Node allows it on the main thread. */
const PAUSE = new Int32Array(new SharedArrayBuffer(4));

/** {@link renameWithRetry} for synchronous code, which waits between attempts without yielding. */
export function renameWithRetrySync(from: string, to: string, options: RenameSyncOptions = {}): void {
  const rename = options.rename ?? fsRenameSync;
  const windows = (options.platform ?? process.platform) === "win32";
  const deadline = Date.now() + (options.retryMs ?? RENAME_RETRY_MS);
  for (let attempt = 1; ; attempt += 1) {
    try {
      rename(from, to);
      return;
    } catch (error) {
      if (!windows || !HELD.has(codeOf(error)) || Date.now() >= deadline) throw error;
      if (attempt === 1 && makeWritableSync(to) === "folder") throw error;
      Atomics.wait(PAUSE, 0, 0, RENAME_RETRY_INTERVAL_MS);
    }
  }
}

/** What a rename target is once a read-only file there was made writable: a folder, or anything a retry can replace. */
type RenameTarget = "folder" | "replaceable";

function makeWritableSync(path: string): RenameTarget {
  try {
    const info = statSync(path);
    if (info.isDirectory()) return "folder";
    if ((info.mode & 0o200) === 0) chmodSync(path, info.mode | 0o200);
  } catch {
    // Missing or unreadable: the next rename attempt reports what is wrong.
  }
  return "replaceable";
}

async function makeWritable(path: string): Promise<RenameTarget> {
  try {
    const info = await stat(path);
    if (info.isDirectory()) return "folder";
    if ((info.mode & 0o200) === 0) await chmod(path, info.mode | 0o200);
  } catch {
    // Missing or unreadable: the next rename attempt reports what is wrong.
  }
  return "replaceable";
}

/**
 * Removes a temporary folder Rerail made, retrying while Windows holds a file in it, and never fails: a leftover folder
 * in the temp directory must not replace the result, or the error, of the work that used it.
 */
export async function removeScratch(path: string, rm: typeof fsRm = fsRm): Promise<void> {
  try {
    await rm(path, { recursive: true, force: true, maxRetries: 10 });
  } catch {
    // Left for the system's temp cleanup.
  }
}
