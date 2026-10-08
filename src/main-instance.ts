import { mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve, sep } from "node:path";
import { loadNativeBinding, type NativeExclusiveFileLock } from "./native.js";
import { mainContentStoreDirectory, type RerailBuild } from "./rerail-build.js";

/**
 * At most one main Rerail instance runs on a machine: the main daemon, which alone opens the machine-wide
 * content store. It holds an OS advisory lock on `main-instance.lock` in the store's directory for its whole
 * life; the OS drops the lock however the process ends, so the lock is the only liveness signal and nothing
 * is ever reclaimed by guessing. The holder record sits beside the lock rather than inside it because
 * Windows' `LockFileEx` would also bar other processes from reading a locked file; it only names the holder
 * in a refusal and never decides anything.
 */
export const MAIN_INSTANCE_LOCK_FILE = "main-instance.lock";
export const MAIN_INSTANCE_HOLDER_FILE = "main-instance.json";

export interface MainInstanceHolder {
  pid: number;
  version: string;
  packageRoot: string;
  /** ISO-8601 time the lock was taken. */
  startedAt: string;
}

export type MainRerailBuild = Extract<RerailBuild, { kind: "main" }>;

/** Another process holds the main-instance lock; `holder` is undefined while it has not yet recorded itself. */
export class MainInstanceConflictError extends Error {
  constructor(readonly directory: string, readonly holder: MainInstanceHolder | undefined) {
    super(
      holder === undefined
        ? `Another main Rerail instance holds ${join(directory, MAIN_INSTANCE_LOCK_FILE)}; stop it (rerail daemon stop) before starting this one.`
        : `Another main Rerail instance is running: pid ${holder.pid}, version ${holder.version}, from ${holder.packageRoot}, ` +
          `since ${holder.startedAt}. Stop it (rerail daemon stop) before starting this one.`,
    );
    this.name = "MainInstanceConflictError";
  }
}

export interface MainInstanceLease {
  readonly directory: string;
  readonly holder: MainInstanceHolder;
  /** Gives the lock up before process exit, e.g. once a daemon has closed its stores. */
  release(): void;
}

interface HeldLock {
  directory: string;
  lock: NativeExclusiveFileLock;
}

/** Module state keeps the native handle reachable: a collected handle would close, and so release, the lock. */
let held: HeldLock | undefined;

function isMainInstanceHolder(value: unknown): value is MainInstanceHolder {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return typeof record.pid === "number" && typeof record.version === "string"
    && typeof record.packageRoot === "string" && typeof record.startedAt === "string";
}

function readHolder(directory: string): MainInstanceHolder | undefined {
  try {
    const value: unknown = JSON.parse(readFileSync(join(directory, MAIN_INSTANCE_HOLDER_FILE), "utf8"));
    return isMainInstanceHolder(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Makes this process the machine's main Rerail instance for the rest of its life, or throws
 * `MainInstanceConflictError` at once when another process already is. Never waits.
 */
export function acquireMainInstance(build: MainRerailBuild, directory: string = mainContentStoreDirectory()): MainInstanceLease {
  if (held !== undefined) throw new Error(`This process is already the main Rerail instance (${held.directory})`);
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const lock = loadNativeBinding().tryLockExclusive(join(directory, MAIN_INSTANCE_LOCK_FILE));
  if (lock === null) throw new MainInstanceConflictError(directory, readHolder(directory));
  const holder: MainInstanceHolder = {
    pid: process.pid,
    version: build.version,
    packageRoot: build.packageRoot,
    startedAt: new Date().toISOString(),
  };
  const holderPath = join(directory, MAIN_INSTANCE_HOLDER_FILE);
  const staged = `${holderPath}.${process.pid}`;
  try {
    // A refusal may read the record at any moment; the rename keeps it from ever seeing half of one.
    writeFileSync(staged, `${JSON.stringify(holder)}\n`, { mode: 0o600 });
    renameSync(staged, holderPath);
  } catch (error) {
    rmSync(staged, { force: true });
    lock.release();
    throw error;
  }
  const lease: HeldLock = { directory, lock };
  held = lease;
  return {
    directory,
    holder,
    release() {
      if (held !== lease) return;
      held = undefined;
      // Still the holder here, so no other instance can have written the record yet.
      rmSync(holderPath, { force: true });
      lock.release();
    },
  };
}

/**
 * Refuses to open a content store inside the machine-wide store directory unless this process is the main
 * instance. Dev builds never take the lock, so this also keeps them, and any short-lived main process, off the
 * main store whatever `RERAIL_CONTENT_STORE_PATH` says; they reach it only through the main daemon.
 */
export function assertContentStoreOwner(storePath: string): void {
  const directory = resolve(mainContentStoreDirectory());
  const path = relative(directory, resolve(storePath));
  const insideMainDirectory = path === "" || (path !== ".." && !path.startsWith(`..${sep}`) && !isAbsolute(path));
  if (!insideMainDirectory) return;
  if (held !== undefined && resolve(held.directory) === directory) return;
  throw new Error(
    `The machine-wide Rerail content store ${storePath} belongs to the main Rerail daemon, and process ${process.pid} is not it. ` +
      "Main harness processes evaluate policies through the daemon; dev builds use their checkout's own store.",
  );
}
