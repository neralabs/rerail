import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
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
 * processes from reading a locked file; the record names the holder in a message and decides nothing.
 */
const LOCK_FILE = "daemon.lock";
const HOLDER_FILE = "daemon.json";

/**
 * How long a daemon keeps trying for a lock another process holds. A start that asks whether a daemon runs
 * ({@link homeDaemonLockHeld}) holds it for an instant, and a daemon restarting itself lets go just before its successor
 * starts; only a running daemon holds it for longer.
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
  /** Gives the lock up before process exit, once the daemon has closed. */
  release(): void;
}

/** Module state keeps the native handle reachable: a collected handle would close, and so release, the lock. */
let held: NativeExclusiveFileLock | undefined;

/** The pid the holder of `home`'s lock recorded, if it recorded one. */
export function homeDaemonHolder(home: string): number | undefined {
  try {
    const value: unknown = JSON.parse(readFileSync(join(home, LOCKS_DIR, HOLDER_FILE), "utf8"));
    const pid = typeof value === "object" && value !== null && "pid" in value ? value.pid : undefined;
    return typeof pid === "number" && Number.isInteger(pid) ? pid : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Makes this process the daemon of `home` for the rest of its life, or throws {@link HomeDaemonConflictError} when
 * another process still holds the home after {@link ACQUIRE_WAIT_MS}. Undefined where the native binding does not
 * load: the lock cannot be tried, and the engine pipe's or socket's own check is then all that keeps a second daemon out.
 */
export async function holdHomeDaemonLock(home: string): Promise<HomeDaemonLease | undefined> {
  if (held !== undefined) throw new Error("This process already holds a Rerail home's daemon lock");
  mkdirSync(join(home, LOCKS_DIR), { recursive: true, mode: 0o700 });
  const path = join(home, LOCKS_DIR, LOCK_FILE);
  let tryLock: () => NativeExclusiveFileLock | null;
  try {
    const native = loadNativeBinding();
    tryLock = () => native.tryLockExclusive(path);
  } catch {
    return undefined;
  }
  const deadline = Date.now() + ACQUIRE_WAIT_MS;
  let lock = tryLock();
  while (lock === null) {
    if (Date.now() >= deadline) throw new HomeDaemonConflictError(home, homeDaemonHolder(home));
    await sleep(ACQUIRE_POLL_MS);
    lock = tryLock();
  }
  recordHolder(home);
  const taken = lock;
  held = taken;
  return {
    release() {
      if (held !== taken) return;
      held = undefined;
      try {
        // Still the holder here, so no other daemon can have written the record yet.
        rmSync(join(home, LOCKS_DIR, HOLDER_FILE), { force: true });
      } finally {
        taken.release();
      }
    },
  };
}

/**
 * Writes this process's pid beside the lock, whole, since a refusal may read the record at any moment. Never fails the
 * daemon: a holder that could not record itself holds the home all the same, and the refusal then names no pid.
 */
function recordHolder(home: string): void {
  const record = join(home, LOCKS_DIR, HOLDER_FILE);
  try {
    rmSync(record, { force: true });
    replaceFileSync(record, `${JSON.stringify({ pid: process.pid })}\n`, { mode: 0o600 });
  } catch {
    // Unrecorded, the holder holds the home all the same.
  }
}

/**
 * Whether a daemon holds `home`, asked by taking its lock and giving it straight back. False where there is no lock
 * file, which is left unmade; undefined when the native binding does not load, for the caller to go on as if none did.
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
