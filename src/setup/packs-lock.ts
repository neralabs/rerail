/**
 * One change at a time to a Rerail home's installed packs. Set-up's install (snapshot, install, check, put back) and
 * `rerail packs add|remove` each hold this lock, so two sessions' set-ups, or a set-up and a `rerail packs add`, never
 * run npm in the one prefix together, and a put-back never undoes another process's install.
 */
import { mkdirSync } from "node:fs";
import { installedPacksPrefix } from "../engine/catalog/rule-catalog.js";
import { withHomeLock } from "../engine/native/file-lock.js";

/** How long to wait for another change to the packs to end: several npm installs, each fetching from the registry. */
export const PACKS_LOCK_WAIT_MS = 5 * 60_000;

export interface PacksLockOptions {
  /** How long to wait for another holder: `PACKS_LOCK_WAIT_MS` unless given. */
  readonly waitMs?: number;
  /** Waits between tries. */
  readonly sleep?: (ms: number) => Promise<void>;
}

/**
 * Runs `action` holding the lock on `home`'s packs (`locks/packs.lock`, `withHomeLock`), waiting while another process
 * holds it. Throws, naming the holder, when it is still held after the wait.
 */
export async function withPacksLock<T>(home: string, action: () => Promise<T>, options: PacksLockOptions = {}): Promise<T> {
  const prefix = installedPacksPrefix(home);
  mkdirSync(prefix, { recursive: true });
  return await withHomeLock(home, {
    name: "packs",
    waitMs: options.waitMs ?? PACKS_LOCK_WAIT_MS,
    sleep: options.sleep,
    busy: holder => new Error(
      `Another Rerail process (${holder === undefined ? "starting" : `pid ${holder.pid}`}) is changing the rule packs in ${prefix}; try again once it is done.`,
    ),
  }, action);
}
