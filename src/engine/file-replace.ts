/**
 * Replacing, removing and exclusively creating files where Windows holds them open.
 *
 * On Windows, renaming a file onto or from one another process holds open fails with EPERM, EACCES or EBUSY: a
 * reader of `daemon.json`, an editor or language server with a source file open, or antivirus and the indexer scanning
 * a file just written. Removing a folder fails the same way while a file in it is held. Each hold lasts a moment, so
 * these helpers retry there; elsewhere the first failure stands, as a rename over a file never fails for that reason.
 * Every rename in `src/` goes through them: `npm run check:owned-builtins` fails on any other. Creating a file or
 * folder only if none is there fails with those codes too, not EEXIST, while another process deletes one of that name:
 * `nameTaken` reads them as the name being taken.
 *
 * To make text the whole of a file, use `replaceFile` or `replaceFileSync`. `renameWithRetry` is for moving a file or
 * folder that already exists, and for a writer that must open its new file itself.
 */
import { randomUUID } from "node:crypto";
import { chmodSync, closeSync, fchmodSync, openSync, renameSync as fsRenameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { chmod, open, rename as fsRename, rm as fsRm, stat } from "node:fs/promises";
import { setTimeout as sleep } from "node:timers/promises";

/**
 * How a folder removal waits out a file Windows holds for a moment, given to `rm` or `rmSync`: 10 more tries, each
 * 100 ms later than the one before, about 5.5 s in all. A folder a running process keeps open stays held, and the
 * removal then fails. Every removal that retries uses these: `removeScratch`, the updater's staging cleanup and
 * `rerail install --remove`; the tests' `removeFolder` repeats them, held equal by `test/engine/file-replace.test.ts`.
 */
export const FOLDER_REMOVAL_RETRIES = { maxRetries: 10, retryDelay: 100 } as const;

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
 * Whether `error` is Windows refusing a file or folder another process holds open or is deleting (EPERM, EACCES or
 * EBUSY), which it is free of a moment later. Never on another platform, where those codes mean this process may not.
 */
export function heldOnWindows(error: unknown, platform: NodeJS.Platform = process.platform): boolean {
  return platform === "win32" && HELD.has(codeOf(error));
}

/**
 * Whether `error`, from creating a file or folder only if none is there (`wx`, `O_EXCL`, or a `mkdir` taken as a lock),
 * says another process has the name. EEXIST says so everywhere. On Windows a file or folder another process is deleting
 * keeps its name until the delete ends: creating, reading or removing it in that moment fails with EPERM, EACCES or
 * EBUSY, and looking at it (`stat`) still succeeds. The name is free again a moment later.
 */
export function nameTaken(error: unknown, platform: NodeJS.Platform = process.platform): boolean {
  return codeOf(error) === "EEXIST" || heldOnWindows(error, platform);
}

/**
 * What a rename Windows refused for the whole retry throws. The bare `EPERM: operation not permitted, rename` never
 * says that another program had the file open, which is what the person must close; the code stays for callers that
 * branch on it, and the last refusal is the cause.
 */
function heldTooLong(error: unknown, from: string, to: string, retryMs: number): Error {
  const code = codeOf(error);
  const message = `${String(code)}: Windows refused for ${retryMs / 1_000} s to rename ${from} to ${to}, as another`
    + " program held one of them open. Close the program that has it open, such as an editor or a virus scan, and try again.";
  return Object.assign(new Error(message, { cause: error }), { code });
}

/**
 * Renames `from` over `to`, retried on Windows for {@link RENAME_RETRY_MS} ms while either file is held, then thrown
 * as an error that says another program held it. A read-only `to` is made writable first there, since Windows refuses
 * to replace a read-only file, which mode bits elsewhere never stop; the file moved in keeps its own mode. A folder at
 * `to` fails at once with the refusal itself: Windows never renames over one, so waiting would only delay the error.
 */
export async function renameWithRetry(from: string, to: string, options: RenameOptions = {}): Promise<void> {
  const rename = options.rename ?? fsRename;
  const retryMs = options.retryMs ?? RENAME_RETRY_MS;
  const deadline = Date.now() + retryMs;
  for (let attempt = 1; ; attempt += 1) {
    try {
      await rename(from, to);
      return;
    } catch (error) {
      if (!heldOnWindows(error, options.platform)) throw error;
      if (attempt === 1 && await makeWritable(to) === "folder") throw error;
      if (Date.now() >= deadline) throw heldTooLong(error, from, to, retryMs);
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
  const retryMs = options.retryMs ?? RENAME_RETRY_MS;
  const deadline = Date.now() + retryMs;
  for (let attempt = 1; ; attempt += 1) {
    try {
      rename(from, to);
      return;
    } catch (error) {
      if (!heldOnWindows(error, options.platform)) throw error;
      if (attempt === 1 && makeWritableSync(to) === "folder") throw error;
      if (Date.now() >= deadline) throw heldTooLong(error, from, to, retryMs);
      Atomics.wait(PAUSE, 0, 0, RENAME_RETRY_INTERVAL_MS);
    }
  }
}

export interface ReplaceFileOptions extends RenameOptions {
  /** The file's permission bits, set exactly: the umask does not narrow them. Windows keeps only the read-only bit. */
  mode: number;
}

export interface ReplaceFileSyncOptions extends RenameSyncOptions {
  /** As {@link ReplaceFileOptions.mode}. */
  mode: number;
}

/**
 * The new file written beside `path` first. The uninstall knows `<file>.<pid>.<uuid>.tmp` in the Rerail home as a
 * write cut short. The name is new, so creating it exclusively refuses a link someone planted there.
 */
const sidecarOf = (path: string): string => `${path}.${process.pid}.${randomUUID()}.tmp`;

/**
 * Makes `text` the whole of the file at `path`: a reader sees the old file or the new one, never part of either, and
 * the old file is never removed first. The text goes to a new file beside `path`, which {@link renameWithRetry} then
 * moves over it, so a file another program holds on Windows is retried and the give-up says what held it. When any
 * step fails the new file is removed, the old one stays, and the error is the step's. The folder must exist.
 */
export async function replaceFile(path: string, text: string, options: ReplaceFileOptions): Promise<void> {
  const { mode, ...renameOptions } = options;
  const sidecar = sidecarOf(path);
  try {
    const handle = await open(sidecar, "wx", mode);
    try {
      await handle.writeFile(text, "utf8");
      await handle.chmod(mode);
    } finally {
      await handle.close();
    }
    await renameWithRetry(sidecar, path, renameOptions);
  } catch (error) {
    // The failure the caller must see is the write's or the rename's, not a removal's.
    await fsRm(sidecar, { force: true }).catch(() => undefined);
    throw error;
  }
}

/** {@link replaceFile} for synchronous code. */
export function replaceFileSync(path: string, text: string, options: ReplaceFileSyncOptions): void {
  const { mode, ...renameOptions } = options;
  const sidecar = sidecarOf(path);
  try {
    const descriptor = openSync(sidecar, "wx", mode);
    try {
      writeFileSync(descriptor, text, "utf8");
      fchmodSync(descriptor, mode);
    } finally {
      closeSync(descriptor);
    }
    renameWithRetrySync(sidecar, path, renameOptions);
  } catch (error) {
    removeQuietlySync(sidecar);
    throw error;
  }
}

function removeQuietlySync(path: string): void {
  try {
    rmSync(path, { force: true });
  } catch {
    // The failure the caller must see is the write's or the rename's, not a removal's.
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
    await rm(path, { recursive: true, force: true, ...FOLDER_REMOVAL_RETRIES });
  } catch {
    // Left for the system's temp cleanup.
  }
}
