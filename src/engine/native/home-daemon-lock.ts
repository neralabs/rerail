import { existsSync, mkdirSync, readFileSync, rmSync, statSync, type BigIntStats } from "node:fs";
import { join } from "node:path";
import { replaceFileSync } from "../file-replace.js";
import { LOCKS_DIR } from "./file-lock.js";
import { loadNativeBinding, type NativeExclusiveFileLock } from "./native.js";

/**
 * At most one Rerail daemon runs per Rerail home, whatever the build. The daemon holds an OS lock on
 * `locks/daemon.lock` in its home for its whole life, taken before it opens the database or binds a socket or a port;
 * the OS drops it however the process ends, so a held lock means a daemon runs in the home whether or not `daemon.json`
 * names it. The holder records its pid beside the lock (`locks/daemon.json`), since Windows' `LockFileEx` keeps other
 * processes from reading a locked file. The record names the holder in a message, and tells a start or a stop whether
 * the lock is worth asking, which loads the engine; the lock alone says whether a daemon holds the home.
 */
const LOCK_FILE = "daemon.lock";
const HOLDER_FILE = "daemon.json";

/**
 * How long a daemon waits for a lock another process holds before it takes that holder for a running daemon. A start
 * that asks whether a daemon runs ({@link homeDaemonLockHeld}) holds it for an instant, and a daemon restarting itself
 * lets go as it exits, just after it launches the next one: the OS hands the lock to the waiting daemon at that release,
 * well inside this bound. Only a running daemon holds it for longer, and nothing but time tells one from a holder that is
 * about to let go.
 */
const ACQUIRE_WAIT_MS = 1_000;

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

/** The pid the holder of `home`'s lock recorded, if it recorded one. */
export function homeDaemonHolder(home: string): number | undefined {
  try {
    // JSON null throws here too, and is no record.
    const { pid } = JSON.parse(readFileSync(join(home, LOCKS_DIR, HOLDER_FILE), "utf8")) as { pid?: unknown };
    return typeof pid === "number" && Number.isInteger(pid) ? pid : undefined;
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
 * another process still holds the home after {@link ACQUIRE_WAIT_MS}; when the wait itself failed, its error. Undefined
 * where the first try cannot be made (the native binding does not load, the lock file does not open, or the try fails):
 * the engine pipe's or socket's own check is then all that keeps a second daemon out, as on a build without the lock.
 */
export async function holdHomeDaemonLock(home: string): Promise<HomeDaemonLease | undefined> {
  if (held !== undefined) throw new Error("This process already holds a Rerail home's daemon lock");
  mkdirSync(join(home, LOCKS_DIR), { recursive: true, mode: 0o700 });
  const path = join(home, LOCKS_DIR, LOCK_FILE);
  let waiting: Promise<NativeExclusiveFileLock | null>;
  try {
    // The engine's first try is made before this returns, and a free lock is already taken. One open serves the whole
    // wait: a lock file another program opens meanwhile (Windows) cannot end it.
    waiting = loadNativeBinding().lockExclusive(path, ACQUIRE_WAIT_MS);
  } catch {
    return undefined;
  }
  const taken = await waiting;
  if (taken === null) throw new HomeDaemonConflictError(home, homeDaemonHolder(home));
  recordHolder(home);
  held = taken;
  // The file this process locked, read by path just after: only a home moved in that instant could give another.
  const locked = fileAt(path);
  return {
    holdsHome() {
      if (locked === undefined || locked === "none") return true;
      const now = fileAt(path);
      return now === undefined || (now !== "none" && now.dev === locked.dev && now.ino === locked.ino);
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
 * Writes this process's pid beside the lock, whole, since a refusal may read the record at any moment: the replace is
 * atomic, so a reader sees the record a holder killed outright left, or this one, never none. Never fails the daemon:
 * a holder that could not record itself holds the home all the same, and removes the record an earlier holder left, so
 * the refusal names no pid rather than one that runs no daemon.
 */
function recordHolder(home: string): void {
  const record = join(home, LOCKS_DIR, HOLDER_FILE);
  try {
    replaceFileSync(record, `${JSON.stringify({ pid: process.pid })}\n`, { mode: 0o600 });
  } catch {
    try {
      rmSync(record, { force: true });
    } catch {
      // Unrecorded, the holder holds the home all the same.
    }
  }
}

/**
 * Whether a daemon holds `home`, asked by taking its lock and giving it straight back. False where there is no lock
 * file, which is left unmade; undefined when the lock cannot be asked (the native binding does not load, or the lock
 * file does not open), for the caller to go on as if none did.
 */
export function homeDaemonLockHeld(home: string): boolean | undefined {
  const path = join(home, LOCKS_DIR, LOCK_FILE);
  if (!existsSync(path)) return false;
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
