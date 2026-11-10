/**
 * The locks that let one Rerail process at a time do a piece of work on a Rerail home: start its daemon, update it,
 * change its installed rule packs, activate one of its profiles.
 *
 * Each is the OS lock `main-instance.ts` holds (`flock` / `LockFileEx`, through the engine) on a file in the home's
 * `locks/` folder. The OS drops it when its holder ends, however it ends, so a lock is free the moment its holder is
 * gone and no process ever judges another's lock abandoned: two processes that judge one dead holder's lock at the same
 * moment could both take it over, one removing the lock the other has just made in its place.
 *
 * A lock file is never removed while Rerail may take it: a process holding the lock on a removed file and one holding
 * it on the file made in its place would both hold it. (A first install that fails removes the `locks/` it made:
 * `ManagedUpdater.install`.) The holder records itself beside the lock (`<name>.json`: pid, since when) so a process
 * that gave up waiting can say who holds it, and a person can look; the record decides nothing.
 *
 * Without the engine there is no OS lock. `rerail update` is how a person replaces a Rerail whose engine does not
 * load, so the update lock alone asks for a lock that works without it ({@link takeFolderLock}). An update that loads
 * its engine and one that cannot do not see each other's lock; both run from one package, and so agree, unless two
 * Rerail versions update the home at the same moment.
 */
import { randomUUID } from "node:crypto";
import { readFileSync, rmdirSync, rmSync, unlinkSync } from "node:fs";
import { mkdir, readdir, readFile, rm, rmdir, unlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { heldOnWindows, renameWithRetry } from "../file-replace.js";
import { loadNativeBinding, type NativeBinding } from "./native.js";

/** In the Rerail home: the lock files. */
export const LOCKS_DIR = "locks";

/** How often a process waiting for a lock tries it again. */
const POLL_MS = 50;

/** Who holds a lock, as its holder recorded itself. */
export interface LockHolder {
  readonly pid: number;
  /** ISO-8601 time it took the lock. */
  readonly since: string;
}

/** What the OS lock comes from: the engine, which tests stand in for or make fail to load. */
export type LockEngine = Pick<NativeBinding, "tryLockExclusive">;

export interface LockRequest {
  /** The lock file is `locks/<name>.lock`. */
  readonly name: string;
  /** The error for a lock another process still holds after the wait, given who holds it if that is known. */
  readonly busy: (holder: LockHolder | undefined) => Error;
  /** How long to wait while another process holds the lock; 0, the default, tries once. */
  readonly waitMs?: number | undefined;
  /** Waits between tries. */
  readonly sleep?: ((ms: number) => Promise<void>) | undefined;
  /** Loads the engine; `loadNativeBinding` unless given. */
  readonly engine?: (() => LockEngine) | undefined;
  /** Where the engine does not load, take the folder lock rather than fail: the update lock only. */
  readonly withoutEngine?: "fail" | "folder" | undefined;
}

type Wait = Pick<LockRequest, "waitMs" | "sleep">;

type Taken =
  | { readonly kind: "held"; release(): void }
  | { readonly kind: "busy"; readonly holder: LockHolder | undefined };

/** Runs `action` holding `home`'s lock `request.name`, waiting up to `request.waitMs`; throws `request.busy` past that. */
export async function withHomeLock<T>(home: string, request: LockRequest, action: () => Promise<T>): Promise<T> {
  const locks = join(home, LOCKS_DIR);
  await mkdir(locks, { recursive: true, mode: 0o700 });
  const taken = await take(locks, request);
  if (taken.kind === "busy") throw request.busy(taken.holder);
  try {
    return await action();
  } finally {
    taken.release();
  }
}

/** The OS lock, or the folder lock where the engine does not load and the request allows it. */
async function take(locks: string, request: LockRequest): Promise<Taken> {
  let engine: LockEngine;
  try {
    engine = (request.engine ?? loadNativeBinding)();
  } catch (error) {
    if (request.withoutEngine !== "folder") throw error;
    return await takeFolderLock(join(locks, `${request.name}-without-engine`), request);
  }
  return await takeOsLock(engine, locks, request);
}

function isRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    // EPERM: it runs, as another user.
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

function parseHolder(text: string): LockHolder | undefined {
  try {
    const value: unknown = JSON.parse(text);
    if (typeof value !== "object" || value === null) return undefined;
    const { pid, since } = value as Record<string, unknown>;
    return typeof pid === "number" && typeof since === "string" ? { pid, since } : undefined;
  } catch {
    return undefined;
  }
}

const holderText = (): string => `${JSON.stringify({ pid: process.pid, since: new Date().toISOString() } satisfies LockHolder)}\n`;

async function takeOsLock(engine: LockEngine, locks: string, request: LockRequest): Promise<Taken> {
  const path = join(locks, `${request.name}.lock`);
  const record = join(locks, `${request.name}.json`);
  const deadline = Date.now() + (request.waitMs ?? 0);
  for (;;) {
    const lock = engine.tryLockExclusive(path);
    if (lock !== null) {
      await writeRecord(record);
      return {
        kind: "held",
        release() {
          try {
            // Still the holder here, so the record is this process's own.
            rmSync(record, { force: true });
          } finally {
            lock.release();
          }
        },
      };
    }
    if (Date.now() >= deadline) return { kind: "busy", holder: runningHolder(record) };
    await (request.sleep ?? sleep)(POLL_MS);
  }
}

/**
 * Records this process as the holder, whole: a waiter may read the record at any moment. Never fails the work it
 * names: on Windows the rename may meet a reader for longer than it retries, and a holder that could not record itself
 * holds the lock all the same.
 */
async function writeRecord(record: string): Promise<void> {
  const staged = `${record}.${process.pid}.${randomUUID()}.tmp`;
  try {
    await writeFile(staged, holderText(), { mode: 0o600 });
    await renameWithRetry(staged, record);
  } catch {
    await rm(staged, { force: true }).catch(() => undefined);
  }
}

/** The recorded holder while its process runs: one the OS ended without letting go leaves its record behind. */
function runningHolder(record: string): LockHolder | undefined {
  let text: string;
  try {
    text = readFileSync(record, "utf8");
  } catch {
    return undefined;
  }
  const holder = parseHolder(text);
  return holder !== undefined && isRunning(holder.pid) ? holder : undefined;
}

/**
 * The lock where there is no OS lock: the folder `folder`, holding one file, `owner-<token>.json`, that names its
 * holder. A holder that dies leaves its folder, so a taker clears a folder whose holder's process is gone, and no two
 * takers that find one dead holder can both win:
 *
 * - The folder appears whole, its owner inside, by one rename of a folder made beside it, so a taker never finds a
 *   folder being made. The rename fails onto a folder with an owner in it; on Windows it fails onto any folder.
 * - A taker clears a dead holder's folder by removing that holder's owner file, named by its token, so it never removes
 *   a later holder's; one that finds the file gone tries again. It then removes the folder only if it is empty.
 * - A folder with no owner in it has no holder, and any taker may remove it.
 *
 * A process running on this machine never reads as gone, so a live holder is never cleared. A dead one whose pid the
 * system has given to another process reads as running: its lock stays until a person removes the folder.
 */
async function takeFolderLock(folder: string, wait: Wait): Promise<Taken> {
  const token = randomUUID();
  const owner = `owner-${token}.json`;
  const staged = `${folder}.${token}.tmp`;
  const deadline = Date.now() + (wait.waitMs ?? 0);
  await mkdir(staged, { mode: 0o700 });
  try {
    await writeFile(join(staged, owner), holderText(), { mode: 0o600 });
    for (;;) {
      if (await moveIn(staged, folder)) return { kind: "held", release: () => { letGo(folder, owner); } };
      const found = await folderOwner(folder);
      if (found.kind !== "held" && await clear(folder, found)) continue;
      if (Date.now() >= deadline) return { kind: "busy", holder: found.kind === "empty" ? undefined : found.holder };
      await (wait.sleep ?? sleep)(POLL_MS);
    }
  } finally {
    // Gone already once moved in.
    await rm(staged, { recursive: true, force: true });
  }
}

/** Renames the staged folder to `folder`; false when another holder's folder is there. */
async function moveIn(staged: string, folder: string): Promise<boolean> {
  try {
    await renameWithRetry(staged, folder);
    return true;
  } catch (error) {
    // ENOTEMPTY or EEXIST: a folder with an owner in it. On Windows, any folder there, or one being deleted.
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOTEMPTY" || code === "EEXIST" || heldOnWindows(error)) return false;
    throw error;
  }
}

type FolderOwner =
  | { readonly kind: "empty" }
  | { readonly kind: "dead"; readonly file: string; readonly holder: LockHolder }
  | { readonly kind: "held"; readonly holder: LockHolder | undefined };

const OWNER_FILE = /^owner-[0-9a-f-]+\.json$/u;

/**
 * What holds `folder`: no owner ("empty", also for a folder gone since), an owner whose process is gone ("dead"), or
 * a running owner ("held"). A folder with anything else in it, or an owner file that cannot be read, reads as held by
 * an unknown holder and is left alone.
 */
async function folderOwner(folder: string): Promise<FolderOwner> {
  let entries: string[];
  try {
    entries = await readdir(folder);
  } catch {
    // Gone since the rename failed, or on Windows being deleted: the next rename tells which.
    return { kind: "empty" };
  }
  const [file] = entries;
  if (file === undefined) return { kind: "empty" };
  if (entries.length > 1 || !OWNER_FILE.test(file)) return { kind: "held", holder: undefined };
  const holder = parseHolder(await readFile(join(folder, file), "utf8").catch(() => ""));
  if (holder === undefined || isRunning(holder.pid)) return { kind: "held", holder };
  return { kind: "dead", file, holder };
}

/** Clears a folder with no owner, or one whose owner is gone; true when the next rename may take it. */
async function clear(folder: string, found: Exclude<FolderOwner, { kind: "held" }>): Promise<boolean> {
  try {
    if (found.kind === "dead") await unlink(join(folder, found.file));
    await rmdir(folder);
    return true;
  } catch (error) {
    // Another taker cleared it first and may hold it now (ENOENT, ENOTEMPTY or EEXIST): the next rename tells.
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT" || code === "ENOTEMPTY" || code === "EEXIST") return true;
    // On Windows, another process is deleting it or has it open: wait.
    if (heldOnWindows(error)) return false;
    throw error;
  }
}

/** Removes this holder's owner file, then the folder unless a taker has moved its own in since. */
function letGo(folder: string, owner: string): void {
  try {
    unlinkSync(join(folder, owner));
  } catch {
    // Left in place, it names a process that ends soon, and the next taker after that clears it.
    return;
  }
  try {
    rmdirSync(folder);
  } catch {
    // Another's folder now, or on Windows held open a moment: an empty folder is cleared by the next taker.
  }
}
