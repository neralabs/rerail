/**
 * Replacing and removing files where Windows holds them open.
 *
 * On Windows, renaming a file onto or from one another process holds open fails with EPERM, EACCES or EBUSY: a
 * reader of `daemon.json`, an editor or language server with a source file open, or antivirus and the indexer scanning
 * a file just written. Removing a folder fails the same way while a file in it is held. Each hold lasts a moment, so
 * these helpers retry there; elsewhere the first failure stands, as a rename over a file never fails for that reason.
 */
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
 * stop; the file moved in keeps its own mode.
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
      if (attempt === 1) await makeWritable(to);
      await sleep(RENAME_RETRY_INTERVAL_MS);
    }
  }
}

async function makeWritable(path: string): Promise<void> {
  try {
    const { mode } = await stat(path);
    if ((mode & 0o200) === 0) await chmod(path, mode | 0o200);
  } catch {
    // Missing or unreadable: the next rename attempt reports what is wrong.
  }
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
