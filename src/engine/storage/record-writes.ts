/**
 * Record writes to a Rerail database that never wait on the calling thread for another process's write lock.
 *
 * The daemon writes what it records (session events, audits) on the thread that answers every session's hooks, and
 * better-sqlite3 is synchronous: while `rerail reset`, `rerail scrub` or a newer Rerail's migration held the lock, each
 * write slept in SQLite's busy handler up to the connection's 5 s `busy_timeout`, then failed and the record was lost.
 * Here a write runs at once when the lock is free and nothing waits before it, as before; when the lock is held it
 * waits in memory, in order, behind every write made before it, and the writes are made once a worker thread
 * (`waitForWriteLock`) finds the lock given back. One writer per database file in a process, whatever store calls it, so
 * writes commit in the order they were made: an audit never lands before the session row it names.
 *
 * What a waiting write cannot have: a caller that needs its result or its failure. A write made at once throws as
 * before; one made later reports its failure on stderr, as `CodeRuleEnforcer` reports a record it could not make.
 * Reads on other connections see a waiting write once it is made.
 */
import { resolve } from "node:path";
import { openSqliteDatabase, type RerailDatabase } from "./database.js";
import { isBusy } from "./sqlite-wal.js";
import { waitForWriteLock } from "./write-lock-wait.js";

/** One record write: its statements, run on the writer's connection inside one immediate transaction. */
export type RecordWrite = (database: RerailDatabase) => void;

export interface RecordWriter {
  /**
   * Makes `write` now when the write lock is free and no write waits, throwing what it throws; otherwise keeps it, in
   * order, until the lock is given back. `bytes`, about what it holds in memory, bounds what may wait.
   */
  write(write: RecordWrite, bytes?: number): void;
  /** Whether a write waits for the lock: a read on another connection does not see it yet. */
  waiting(): boolean;
  /** Resolves once no write waits. */
  settled(): Promise<void>;
  /**
   * This user's end. The last one makes what waits, letting its connection wait for the lock up to `busy_timeout`
   * once, and reports what it could not make.
   */
  close(): void;
}

/**
 * What may wait while a holder keeps the lock: past either bound a write is dropped and reported, as every write was
 * after 5 s before, so a holder that never finishes cannot grow the daemon's memory without end.
 */
const MAX_WAITING_WRITES = 10_000;
const MAX_WAITING_BYTES = 64 * 1024 * 1024;

/** Every writer of this process by database file, with how many stores use it. */
const writers = new Map<string, FileRecordWriter>();

/** The record writer for the database at `path`, shared by every store of this process that writes there. */
export function openRecordWriter(path: string): RecordWriter {
  const key = resolve(path);
  let writer = writers.get(key);
  if (writer === undefined) {
    writer = new FileRecordWriter(key, () => writers.delete(key));
    writers.set(key, writer);
  }
  return writer.use();
}

class FileRecordWriter {
  readonly #path: string;
  readonly #ended: () => void;
  readonly #waiting: Array<{ write: RecordWrite; bytes: number }> = [];
  readonly #settled: Array<() => void> = [];
  #database: RerailDatabase | undefined;
  #users = 0;
  #waitingBytes = 0;
  #dropped = 0;
  #lockWait: AbortController | undefined;

  constructor(path: string, ended: () => void) {
    this.#path = path;
    this.#ended = ended;
  }

  use(): RecordWriter {
    // Opened with its first user, as each store opens its own: never first at a write, on the thread answering a hook.
    this.#connection();
    this.#users += 1;
    let closed = false;
    return {
      write: (write, bytes = 0) => {
        // As a closed connection answers a statement.
        if (closed) throw new TypeError("The database connection is not open");
        this.#write(write, bytes);
      },
      waiting: () => this.#waiting.length > 0,
      settled: () => this.#whenSettled(),
      close: () => {
        if (closed) return;
        closed = true;
        this.#release();
      },
    };
  }

  #connection(): RerailDatabase {
    if (this.#database === undefined) {
      this.#database = openSqliteDatabase(this.#path);
      // A busy lock answers at once: the write waits in memory instead of on this thread.
      this.#database.$client.pragma("busy_timeout = 0");
    }
    return this.#database;
  }

  #write(write: RecordWrite, bytes: number): void {
    if (this.#waiting.length === 0 && this.#makeNow(write)) return;
    this.#keep(write, bytes);
  }

  /** True once `write` is made; false when the lock was held, which throws nothing. Other failures throw. */
  #makeNow(write: RecordWrite): boolean {
    const database = this.#connection();
    try {
      database.$client.transaction(() => write(database)).immediate();
      return true;
    } catch (error) {
      if (isBusy(error)) return false;
      throw error;
    }
  }

  #keep(write: RecordWrite, bytes: number): void {
    if (this.#waiting.length >= MAX_WAITING_WRITES || this.#waitingBytes + bytes > MAX_WAITING_BYTES) {
      this.#dropped += 1;
      return;
    }
    this.#waiting.push({ write, bytes });
    this.#waitingBytes += bytes;
    this.#awaitLock();
  }

  #awaitLock(): void {
    if (this.#lockWait !== undefined) return;
    const lockWait = new AbortController();
    this.#lockWait = lockWait;
    void waitForWriteLock(this.#path, lockWait.signal).then(
      free => this.#lockFreed(lockWait, free),
      (error: unknown) => this.#lockFreed(lockWait, false, error),
    );
  }

  /**
   * The worker's answer. A worker that failed (it never should) leaves the writes to this thread's own bounded wait,
   * as before this module, rather than start another that could fail as fast.
   */
  #lockFreed(lockWait: AbortController, free: boolean, error?: unknown): void {
    if (this.#lockWait !== lockWait) return;
    this.#lockWait = undefined;
    if (error === undefined) {
      if (free) this.#makeWaiting();
      return;
    }
    process.stderr.write(`rerail: waiting for the database's write lock failed: ${messageOf(error)}\n`);
    this.#makeWaitingOnThisThread();
  }

  /** Makes the waiting writes in order until one meets the lock again, which waits on; reports each that fails. */
  #makeWaiting(): void {
    while (this.#waiting.length > 0) {
      const next = this.#waiting[0]!;
      if (!this.#makeReported(next.write)) {
        this.#awaitLock();
        return;
      }
      this.#waiting.shift();
      this.#waitingBytes -= next.bytes;
    }
    this.#reportDropped();
    for (const settle of this.#settled.splice(0)) settle();
  }

  /** As `#makeNow`, a failure other than the lock reported rather than thrown: its caller has moved on. */
  #makeReported(write: RecordWrite): boolean {
    try {
      return this.#makeNow(write);
    } catch (error) {
      process.stderr.write(`rerail: record write failed: ${messageOf(error)}\n`);
      return true;
    }
  }

  #reportDropped(): void {
    if (this.#dropped === 0) return;
    process.stderr.write(`rerail: ${String(this.#dropped)} records not written: the database's write lock stayed held past what may wait\n`);
    this.#dropped = 0;
  }

  #whenSettled(): Promise<void> {
    if (this.#waiting.length === 0) return Promise.resolve();
    return new Promise(settle => this.#settled.push(settle));
  }

  #release(): void {
    this.#users -= 1;
    if (this.#users > 0) return;
    this.#ended();
    this.#lockWait?.abort();
    this.#lockWait = undefined;
    if (this.#waiting.length > 0) this.#makeWaitingOnThisThread();
    this.#database?.$client.close();
    this.#database = undefined;
  }

  /**
   * One wait for the lock on this thread, up to the 5 s `busy_timeout` every Rerail connection has, then what still
   * waits is reported lost: the last user's close, which has no later turn, and a lock wait that failed.
   */
  #makeWaitingOnThisThread(): void {
    const client = this.#connection().$client;
    client.pragma("busy_timeout = 5000");
    try {
      while (this.#waiting.length > 0 && this.#makeReported(this.#waiting[0]!.write)) this.#waiting.shift();
    } finally {
      client.pragma("busy_timeout = 0");
    }
    this.#dropped += this.#waiting.splice(0).length;
    this.#waitingBytes = 0;
    this.#reportDropped();
    for (const settle of this.#settled.splice(0)) settle();
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
