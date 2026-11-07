/**
 * One change at a time to a Rerail home's installed packs. Set-up's install (snapshot, install, check, put back) and
 * `rerail packs add|remove` each hold this lock, so two sessions' set-ups, or a set-up and a `rerail packs add`, never
 * run npm in the one prefix together, and a put-back never undoes another process's install.
 */
import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { installedPacksPrefix } from "../engine/catalog/rule-catalog.js";

/** How long to wait for another change to the packs to end: several npm installs, each fetching from the registry. */
export const PACKS_LOCK_WAIT_MS = 5 * 60_000;
/** A lock older than this is left from a process that never released it, even if its pid now names another process. */
const STALE_LOCK_MS = 30 * 60_000;
/** A lock file with no owner yet is being written; one still empty after this was left by a process that died. */
const UNWRITTEN_LOCK_MS = 10_000;
const POLL_MS = 250;

interface LockOwner {
  readonly pid: number;
  readonly token: string;
  readonly at: number;
}

export interface PacksLockOptions {
  /** How long to wait for another holder: `PACKS_LOCK_WAIT_MS` unless given. */
  readonly waitMs?: number;
  /** Waits between tries. */
  readonly sleep?: (ms: number) => Promise<void>;
  /** Whether process `pid` is running. */
  readonly running?: (pid: number) => boolean;
  readonly now?: () => number;
}

function processRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    // EPERM: it runs, as another user.
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

function readOwner(lock: string): LockOwner | undefined {
  try {
    const owner = JSON.parse(readFileSync(lock, "utf8")) as Partial<LockOwner>;
    return typeof owner.pid === "number" && typeof owner.token === "string" && typeof owner.at === "number" ? owner as LockOwner : undefined;
  } catch {
    return undefined;
  }
}

/** Creates the lock file only if there is none (`wx`); false when another holds it. */
function tryTake(lock: string, owner: LockOwner): boolean {
  try {
    writeFileSync(lock, JSON.stringify(owner), { flag: "wx" });
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EEXIST") return false;
    throw error;
  }
}

/** Whether the lock as it is now was left by a holder that is gone. */
function abandoned(lock: string, owner: LockOwner | undefined, running: (pid: number) => boolean, now: number): boolean {
  if (owner !== undefined) return !running(owner.pid) || now - owner.at > STALE_LOCK_MS;
  try {
    return now - statSync(lock).mtimeMs > UNWRITTEN_LOCK_MS;
  } catch {
    // Released between the try and this look.
    return false;
  }
}

/**
 * Runs `action` holding the lock on `home`'s packs (`packs/.lock`), waiting while another process holds it. A lock
 * whose process is gone is taken over. Throws, naming the holder, when it is still held after the wait.
 */
export async function withPacksLock<T>(home: string, action: () => Promise<T>, options: PacksLockOptions = {}): Promise<T> {
  const prefix = installedPacksPrefix(home);
  mkdirSync(prefix, { recursive: true });
  const lock = join(prefix, ".lock");
  const waitMs = options.waitMs ?? PACKS_LOCK_WAIT_MS;
  const sleep = options.sleep ?? (ms => new Promise<void>(resolve => setTimeout(resolve, ms)));
  const running = options.running ?? processRunning;
  const now = options.now ?? Date.now;
  const token = randomUUID();
  for (let waited = 0; !tryTake(lock, { pid: process.pid, token, at: now() }); waited += POLL_MS) {
    const owner = readOwner(lock);
    if (abandoned(lock, owner, running, now())) {
      // Only the lock as it was read: another process may have taken it over meanwhile.
      if (readOwner(lock)?.token === owner?.token) rmSync(lock, { force: true });
      continue;
    }
    if (waited >= waitMs) {
      throw new Error(`Another Rerail process (${owner === undefined ? "starting" : `pid ${owner.pid}`}) is changing the rule packs in ${prefix}; try again once it is done.`);
    }
    await sleep(POLL_MS);
  }
  try {
    return await action();
  } finally {
    if (readOwner(lock)?.token === token) rmSync(lock, { force: true });
  }
}
