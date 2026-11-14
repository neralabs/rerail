import { randomUUID } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { LOCKS_DIR } from "../native/file-lock.js";
import { loadNativeBinding, type NativeExclusiveFileLock } from "../native/native.js";

/**
 * Whether the runtime that wrote a session's owner row still runs, whatever process holds the pid the row names. A
 * daemon that died leaves its rows naming a pid the system can give the next daemon (a container, a restart) or any
 * other process (Windows reuses pids soon), so the pid alone cannot tell. Each engine service therefore holds, for its
 * whole life, the OS lock (`flock` / `LockFileEx`, through the engine, as `main-instance.ts` holds its own) on
 * `locks/owners/<id>.lock` beside its database, under an id of its own, and the owner rows its runtimes write carry
 * that id at the head of their token (`ownerToken`). The OS drops the lock however the process ends, so a later process
 * that can take it knows the rows' writer is gone. Another process takes an id's lock only to see whether it is free,
 * and removes the file only while holding it.
 */
export interface OwnerLock {
  /** Undefined when no lock is held: the rows are then written without one. */
  readonly id: string | undefined;
  /** Removes the lock file while still holding it, then lets the lock go: at the service's close, once its sessions ended. */
  release(): void;
}

/** What a service holds where it could take no lock. */
const NO_LOCK: OwnerLock = { id: undefined, release: () => undefined };

const OWNERS_DIR = "owners";
const LOCK_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const LOCK_FILE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.lock$/u;

function ownersFolder(databasePath: string): string {
  return join(dirname(databasePath), LOCKS_DIR, OWNERS_DIR);
}

/** The owner token of a claim made under the lock `lockId`: the id, a dot and a fresh id; a fresh id alone without one. */
export function ownerToken(lockId: string | undefined): string {
  return lockId === undefined ? randomUUID() : `${lockId}.${randomUUID()}`;
}

/** The lock the owner row with `token` was written under; undefined for a row of a runtime that held none. */
export function ownerLockOf(token: string): string | undefined {
  const id = token.split(".")[0] ?? "";
  return token.includes(".") && LOCK_ID.test(id) ? id : undefined;
}

/**
 * Takes a lock of a fresh id beside `databasePath` for the rest of this process's life, or until `release`. None
 * where the engine does not load or the lock cannot be taken, or where another start removed the file before this
 * process locked it: the service's runtimes then write rows without one, which a later start judges by their pid.
 */
export function holdOwnerLock(databasePath: string): OwnerLock {
  const id = randomUUID();
  const path = join(ownersFolder(databasePath), `${id}.lock`);
  let lock: NativeExclusiveFileLock | null;
  try {
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    lock = loadNativeBinding().tryLockExclusive(path);
  } catch {
    return NO_LOCK;
  }
  if (lock !== null && existsSync(path)) return { id, release: () => removeHeld(path, lock) };
  lock?.release();
  return NO_LOCK;
}

function removeHeld(path: string, lock: NativeExclusiveFileLock): void {
  try {
    rmSync(path, { force: true });
  } catch {
    // Left in place, its lock is free once let go, and the next start's `forgetFreedOwnerLocks` removes it.
  } finally {
    lock.release();
  }
}

/** The lock of `path` when no process holds it; null while one does or when it cannot be tried. */
function takeIfFree(path: string): NativeExclusiveFileLock | null {
  try {
    return loadNativeBinding().tryLockExclusive(path);
  } catch {
    return null;
  }
}

/**
 * Whether the runtime that held the lock `lockId` beside `databasePath` has ended: its lock file is there and its lock
 * can be taken. A lock still held, a file not there (one this database's processes never made) or a lock that cannot
 * be tried says it may still run.
 */
export function ownerLockFreed(databasePath: string, lockId: string): boolean {
  const path = join(ownersFolder(databasePath), `${lockId}.lock`);
  const lock = existsSync(path) ? takeIfFree(path) : null;
  lock?.release();
  return lock !== null;
}

/**
 * Removes the lock file of every runtime beside `databasePath` that has ended, each while holding its lock: those whose
 * sessions the start has just ended, and those of a daemon that ended with none open. A held one is left.
 */
export function forgetFreedOwnerLocks(databasePath: string): void {
  const folder = ownersFolder(databasePath);
  let names: string[];
  try {
    names = readdirSync(folder);
  } catch {
    return;
  }
  for (const name of names.filter(entry => LOCK_FILE.test(entry))) {
    const path = join(folder, name);
    const lock = takeIfFree(path);
    if (lock !== null) removeHeld(path, lock);
  }
}
