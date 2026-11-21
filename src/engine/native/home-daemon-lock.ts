import { closeSync, existsSync, mkdirSync, openSync, readFileSync, readSync, rmSync, statSync, type BigIntStats } from "node:fs";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { replaceFileSync } from "../file-replace.js";
import { LOCKS_DIR } from "./file-lock.js";
import { loadNativeBinding, type NativeExclusiveFileLock } from "./native.js";

/**
 * At most one Rerail daemon runs per Rerail home, whatever the build. The daemon holds an OS lock on
 * `locks/daemon.lock` in its home for its whole life, taken before it opens the database or binds a socket or a port;
 * the OS drops it however the process ends, so a held lock means a daemon runs in the home whether or not `daemon.json`
 * names it. The holder records its pid beside the lock (`locks/daemon.json`), since Windows' `LockFileEx` keeps other
 * processes from reading a locked file. The record names the holder in a message, and tells a start or a stop whether
 * the lock is worth asking, which loads the engine on macOS and Linux; the lock alone says whether a daemon holds the
 * home.
 */
const LOCK_FILE = "daemon.lock";
const HOLDER_FILE = "daemon.json";

/**
 * How long a daemon keeps trying for a lock another process holds. A start that asks whether a daemon runs
 * ({@link homeDaemonLockHeld}) holds it for an instant, and a daemon restarting itself lets go as it exits, just after it
 * launches the next one; only a running daemon holds it for longer.
 */
const ACQUIRE_WAIT_MS = 1_000;
const ACQUIRE_POLL_MS = 50;

/** Another process is the daemon of the home; `pid` is undefined while it has not recorded itself. */
export class HomeDaemonConflictError extends Error {
  override readonly name = "HomeDaemonConflictError";

  constructor(readonly home: string, readonly pid: number | undefined) {
    super(`Another Rerail daemon${pid === undefined ? "" : ` (pid ${pid})`} runs in ${home}; this one stops so the home keeps one.`);
  }
}

export interface HomeDaemonLease {
  /**
   * Whether the lock file this process locked is still the one at its path: false once the home was moved or removed,
   * whether or not a home was made again there. The OS lock goes with the file, so another daemon can then hold the
   * home at that path, and this one holds none. Never throws: where the file system cannot tell (an error other than a
   * missing file, or no file id), the home counts as held, as before the check.
   */
  holdsHome(): boolean;
  /**
   * Records that the daemon's start is over: it has written daemon.json and answers. Until then its record beside the
   * lock says it is starting, so a start that finds it waits for it ({@link homeDaemonStarting}).
   */
  started(): void;
  /** Gives the lock up before process exit, once the daemon has closed. */
  release(): void;
}

/**
 * Which file is at `path`, as the file system tells files apart: "none" when no file is there, its path gone with its
 * folder; undefined when it cannot tell, as for a file Windows has marked for deletion and a file system with no file
 * ids.
 */
function fileAt(path: string): Pick<BigIntStats, "dev" | "ino"> | "none" | undefined {
  try {
    // Bigint: Windows' 64-bit file ids do not fit a number.
    const found = statSync(path, { bigint: true });
    return found.ino === 0n ? undefined : { dev: found.dev, ino: found.ino };
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    return code === "ENOENT" || code === "ENOTDIR" ? "none" : undefined;
  }
}

/** Module state keeps the native handle reachable: a collected handle would close, and so release, the lock. */
let held: NativeExclusiveFileLock | undefined;

/** What the holder of `home`'s lock recorded: its pid, and `starting` until its start is over. */
function holderRecord(home: string): { pid: number; starting: boolean } | undefined {
  try {
    // JSON null throws here too, and is no record.
    const { pid, starting } = JSON.parse(readFileSync(join(home, LOCKS_DIR, HOLDER_FILE), "utf8")) as { pid?: unknown; starting?: unknown };
    return typeof pid === "number" && Number.isInteger(pid) ? { pid, starting: starting === true } : undefined;
  } catch {
    return undefined;
  }
}

/** The pid the holder of `home`'s lock recorded, if it recorded one. */
export function homeDaemonHolder(home: string): number | undefined {
  return holderRecord(home)?.pid;
}

/**
 * The pid the holder of `home`'s lock recorded while its start is not over ({@link HomeDaemonLease.started}), or
 * undefined. The record alone proves nothing: a holder killed in its start leaves it, so a reader asks whether that pid
 * runs and the lock is held.
 */
export function homeDaemonStarting(home: string): number | undefined {
  const record = holderRecord(home);
  return record?.starting === true ? record.pid : undefined;
}

/** Where the holder of `home`'s lock records its pid. */
export function homeDaemonHolderFile(home: string): string {
  return join(home, LOCKS_DIR, HOLDER_FILE);
}

/** When the holder's record in `home` was last written, in epoch ms, or undefined when there is none to read. */
export function homeDaemonHolderWrittenAt(home: string): number | undefined {
  try {
    return statSync(homeDaemonHolderFile(home)).mtimeMs;
  } catch {
    return undefined;
  }
}

/**
 * Removes the holder's record in `home` when it names `pid`, a daemon that ended without its exit handler, which removes
 * the record on every other exit. Windows ends a process that way at a kill, and soon gives its pid to another process.
 */
export function forgetHomeDaemonHolder(home: string, pid: number): void {
  if (homeDaemonHolder(home) === pid) rmSync(join(home, LOCKS_DIR, HOLDER_FILE), { force: true });
}

/**
 * Makes this process the daemon of `home` for the rest of its life, or throws {@link HomeDaemonConflictError} when
 * another process still holds the home after {@link ACQUIRE_WAIT_MS}; when the last try of that wait threw instead of
 * finding the lock held, its error ({@link waitForLock}). Undefined where the first try cannot be made (the native
 * binding does not load, or the lock file does not open): the engine pipe's or socket's own check is then all that keeps
 * a second daemon out, as on a build without the lock.
 */
export async function holdHomeDaemonLock(home: string): Promise<HomeDaemonLease | undefined> {
  if (held !== undefined) throw new Error("This process already holds a Rerail home's daemon lock");
  mkdirSync(join(home, LOCKS_DIR), { recursive: true, mode: 0o700 });
  const path = join(home, LOCKS_DIR, LOCK_FILE);
  let tryLock: () => NativeExclusiveFileLock | null;
  let lock: NativeExclusiveFileLock | null;
  try {
    const native = loadNativeBinding();
    tryLock = () => native.tryLockExclusive(path);
    lock = tryLock();
  } catch {
    return undefined;
  }
  const taken = lock ?? await waitForLock(home, tryLock);
  recordHolder(home, true);
  held = taken;
  // The file this process locked, read by path just after: only a home moved in that instant could give another.
  const locked = fileAt(path);
  return {
    holdsHome() {
      if (locked === undefined || locked === "none") return true;
      const now = fileAt(path);
      return now === undefined || (now !== "none" && now.dev === locked.dev && now.ino === locked.ino);
    },
    started() {
      if (held === taken && homeDaemonHolder(home) === process.pid) recordHolder(home, false);
    },
    release() {
      if (held !== taken) return;
      held = undefined;
      try {
        // Only a record naming this process: once the home was moved and made again at its path, the record there is
        // another daemon's.
        forgetHomeDaemonHolder(home, process.pid);
      } catch {
        // A record that cannot be removed (Windows: another program has it open) names a pid that soon runs no daemon.
      } finally {
        taken.release();
      }
    },
  };
}

/**
 * Tries for the lock another process held at the first try until {@link ACQUIRE_WAIT_MS} runs out. A try that throws,
 * as one can on Windows while another program has the lock file open, does not end the wait; when the last try threw,
 * its error is what the wait ends with, since nothing then says another daemon holds the home.
 */
async function waitForLock(home: string, tryLock: () => NativeExclusiveFileLock | null): Promise<NativeExclusiveFileLock> {
  const deadline = Date.now() + ACQUIRE_WAIT_MS;
  let failure: { readonly error: unknown } | undefined;
  for (;;) {
    if (Date.now() >= deadline) throw failure === undefined ? new HomeDaemonConflictError(home, homeDaemonHolder(home)) : failure.error;
    await sleep(ACQUIRE_POLL_MS);
    try {
      const lock = tryLock();
      if (lock !== null) return lock;
      failure = undefined;
    } catch (error) {
      failure = { error };
    }
  }
}

/**
 * Writes this process's pid beside the lock, whole, with `starting` while its start is not over, since a refusal or a
 * start may read the record at any moment: the replace is atomic, so a reader sees the record a holder killed outright
 * left, or this one, never none. Never fails the daemon: a holder that could not record itself holds the home all the
 * same, and removes the record an earlier holder left, so the refusal names no pid rather than one that runs no daemon.
 */
function recordHolder(home: string, starting: boolean): void {
  const record = join(home, LOCKS_DIR, HOLDER_FILE);
  try {
    replaceFileSync(record, `${JSON.stringify(starting ? { pid: process.pid, starting } : { pid: process.pid })}\n`, { mode: 0o600 });
  } catch {
    try {
      rmSync(record, { force: true });
    } catch {
      // Unrecorded, the holder holds the home all the same.
    }
  }
}

/**
 * Whether a daemon holds `home`; undefined when the lock cannot be asked (the native binding does not load, or the lock
 * file does not open) or there is no lock file, for the caller to go on as if none did. A missing lock file is left
 * unmade, and it proves nothing: a file deleted under the daemon that locked it leaves that daemon holding its lock, on
 * a file no path reaches. On Windows the lock file is read ({@link lockedAgainstReading}), which loads no engine: a hook,
 * and the uninstall's stop before it removes `versions/`, ask it, and Windows keeps a loaded DLL from being deleted.
 * Elsewhere the lock is taken and given straight back.
 */
export function homeDaemonLockHeld(home: string): boolean | undefined {
  const path = join(home, LOCKS_DIR, LOCK_FILE);
  if (!existsSync(path)) return undefined;
  return process.platform === "win32" ? lockedAgainstReading(path) : lockTaken(path);
}

/**
 * Windows: whether another handle holds the lock on `path`. `LockFileEx` keeps every other handle from reading the range
 * it locks, past the end of the empty file included, so a read fails with EBUSY while a daemon holds it. A file that
 * does not open, as one another program opened without sharing it, or a read that fails otherwise, proves nothing.
 */
function lockedAgainstReading(path: string): boolean | undefined {
  let fd: number;
  try {
    fd = openSync(path, "r");
  } catch {
    return undefined;
  }
  try {
    readSync(fd, Buffer.alloc(1), 0, 1, 0);
    return false;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EBUSY" ? true : undefined;
  } finally {
    closeSync(fd);
  }
}

/** Whether another handle holds the lock on `path`, asked by taking it and giving it straight back; undefined if it cannot be. */
function lockTaken(path: string): boolean | undefined {
  let lock: NativeExclusiveFileLock | null;
  try {
    lock = loadNativeBinding().tryLockExclusive(path);
  } catch {
    return undefined;
  }
  if (lock === null) return true;
  lock.release();
  return false;
}
