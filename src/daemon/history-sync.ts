/**
 * The daemon's sync of the history in the background. The store's commits do not wait for the disk
 * (`synchronous = NORMAL`), so a commit sits in the write-ahead log in the operating system's cache until a checkpoint
 * or the system's own writeback puts it on the disk; Linux writes back data 30 s old, and a checkpoint comes after
 * about a thousand pages of log. While commits wait, this syncs the log about every {@link HISTORY_SYNC_PERIOD_MS}, so a
 * power cut or an operating system crash takes about that much history, more while the disk stalls a sync.
 */
import { existsSync } from "node:fs";
import Database from "better-sqlite3";
import { syncHistoryLog } from "../engine/storage/log-sync.js";

/**
 * How long after one look ends the next one comes. The maintainers chose 5 s: it bounds the history a power cut can
 * take, against a sync of the log that often while sessions run; no fact of the store or the disk sets it.
 */
export const HISTORY_SYNC_PERIOD_MS = 5_000;

export interface HistorySync {
  /** Stops the looks; a sync under way is awaited, then the connection that watches for commits is closed. */
  stop(): Promise<void>;
}

export interface HistorySyncOptions {
  readonly periodMs?: number;
  /** Puts the log on the disk: `syncHistoryLog` unless a test passes its own. */
  readonly sync?: (databasePath: string) => Promise<void>;
  readonly log?: (line: string) => void;
}

/**
 * Whether any connection committed since the last call: `PRAGMA data_version` of a read-only connection of its own,
 * which moves with every commit another connection makes, read from SQLite's shared memory without touching the disk.
 * Undefined while the database does not exist; a read that fails counts as a commit, so the log is synced anyway.
 */
class CommitWatch {
  readonly #path: string;
  #connection: Database.Database | undefined;
  #seen: number | undefined;

  constructor(path: string) {
    this.#path = path;
    try {
      this.#seen = this.#version();
    } catch {
      // The first look then counts as a commit.
    }
  }

  committed(): boolean | undefined {
    let version: number | undefined;
    try {
      version = this.#version();
    } catch {
      return true;
    }
    if (version === undefined) return undefined;
    const moved = version !== this.#seen;
    this.#seen = version;
    return moved;
  }

  close(): void {
    this.#connection?.close();
    this.#connection = undefined;
  }

  #version(): number | undefined {
    if (this.#connection === undefined) {
      if (!existsSync(this.#path)) return undefined;
      this.#connection = new Database(this.#path, { readonly: true, fileMustExist: true });
      // A look never waits for a lock: one it cannot take counts as a commit.
      this.#connection.pragma("busy_timeout = 0");
    }
    return this.#connection.pragma("data_version", { simple: true }) as number;
  }
}

/**
 * Syncs the log of the database at `databasePath` every `periodMs` while commits wait, one sync at a time, off the
 * daemon's thread (`syncHistoryLog`): the next look is due `periodMs` after the last one ended, so a stalled disk
 * piles nothing up, and no hook or commit waits for a sync. A look that finds no commit since the last sync does
 * nothing, so an idle daemon does no disk work. A failed sync is said once in the daemon's log until one succeeds
 * again, and the next look tries again, commits or not. It never throws and never holds the daemon open.
 */
export function startHistorySync(databasePath: string, options: HistorySyncOptions = {}): HistorySync {
  const periodMs = options.periodMs ?? HISTORY_SYNC_PERIOD_MS;
  const sync = options.sync ?? syncHistoryLog;
  const log = options.log ?? (line => process.stderr.write(line));
  const watch = new CommitWatch(databasePath);
  let failing = false;
  let stopped = false;
  let timer: NodeJS.Timeout | undefined;
  let looking: Promise<void> = Promise.resolve();
  const look = async (): Promise<void> => {
    // After a failure the commits it left are still unsynced, so the next look tries again with no new commit.
    if (watch.committed() !== true && !failing) return;
    try {
      await sync(databasePath);
      if (failing) log("rerail daemon: syncing the history to the disk works again\n");
      failing = false;
    } catch (error) {
      if (!failing) log(`rerail daemon: syncing the history to the disk failed, trying again every ${periodMs / 1000} s: ${error instanceof Error ? error.message : String(error)}\n`);
      failing = true;
    }
  };
  const next = (): void => {
    if (stopped) return;
    timer = setTimeout(() => {
      looking = look().catch((error: unknown) => log(`rerail daemon: syncing the history: ${error instanceof Error ? error.message : String(error)}\n`)).then(next);
    }, periodMs);
    timer.unref();
  };
  next();
  return {
    async stop() {
      stopped = true;
      clearTimeout(timer);
      await looking;
      watch.close();
    },
  };
}
