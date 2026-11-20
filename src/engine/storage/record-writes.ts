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
 * A read of what this process recorded makes the waiting writes first (`flushRecordWrites`), as does a write on another
 * connection that must not commit ahead of them (`requireRecordWritesMade`). A thread waits for a waiting write no
 * longer than `BUSY_TIMEOUT_MS` from when the write was asked, as the write itself waited before, so the reads, the
 * owner claim and the last close of one turn never wait that bound more than once between them.
 */
import { resolve } from "node:path";
import Database from "better-sqlite3";
import { BUSY_TIMEOUT_MS, openSqliteDatabase, type RerailDatabase } from "./database.js";
import { isBusy } from "./sqlite-wal.js";
import { busyPauseMs, waitForWriteLock } from "./write-lock-wait.js";

const { SqliteError } = Database;

/** One record write: its statements, run on the writer's connection inside one immediate transaction. */
export type RecordWrite = (database: RerailDatabase) => void;

export interface RecordWriter {
  /**
   * Makes `write` now when the write lock is free and no write waits, throwing what it throws; otherwise keeps it, in
   * order, until the lock is given back. `bytes`, about what it holds in memory, bounds what may wait.
   */
  write(write: RecordWrite, bytes?: number): void;
  /**
   * `write`, unless no write waits and `unchanged`, a read of what is on disk, says it would change nothing: a write of
   * no effect takes no write lock. `write` checks again itself, since it may be made later.
   */
  writeUnless(unchanged: () => boolean, write: RecordWrite, bytes?: number): void;
  /**
   * Makes `write` now and answers true when the write lock is free and no write waits, throwing what it throws;
   * answers false, keeping nothing, when another connection holds the lock or a write waits: for a caller that has
   * somewhere else to take the write meanwhile (a hook hands a steer's receipt to the daemon).
   */
  writeNow(write: RecordWrite): boolean;
  /**
   * `writeNow` tried again at the pauses of SQLite's busy handler, between this thread's turns, never holding it, as a
   * statement waits for the lock: true once made; false, keeping nothing, once the pauses reach `BUSY_TIMEOUT_MS` or
   * `signal` aborts first.
   */
  writeWhenFree(write: RecordWrite, signal: AbortSignal): Promise<boolean>;
  /** Resolves once no write waits. */
  settled(): Promise<void>;
  /**
   * This user's end. The last one makes what waits, letting its connection wait for the lock while the oldest waiting
   * write is younger than `BUSY_TIMEOUT_MS`, and reports what it could not make.
   */
  close(): void;
}

/**
 * What may wait while a holder keeps the lock: past either bound a write is dropped and reported, as every write was
 * after 5 s before, so a holder that never finishes cannot grow the daemon's memory without end.
 */
const MAX_WAITING_WRITES = 10_000;
const MAX_WAITING_BYTES = 64 * 1024 * 1024;

/** A write that waits for the lock: until when a thread may wait for it (`BUSY_TIMEOUT_MS` from when it was asked). */
interface WaitingWrite {
  readonly write: RecordWrite;
  readonly bytes: number;
  readonly deadline: number;
}

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

/**
 * Makes this process's waiting record writes to the database at `path` now, on this thread, the connection waiting for
 * the lock while the oldest of them is younger than `BUSY_TIMEOUT_MS`. For a read of what this process recorded, which
 * a holder of a moment would otherwise hide: what it cannot make keeps waiting for the lock, and the read sees what is
 * on disk, as it did once such a write had failed.
 */
export function flushRecordWrites(path: string): void {
  writers.get(resolve(path))?.flush();
}

/**
 * `flushRecordWrites` for a write on another connection that must not commit ahead of this process's waiting record
 * writes, such as the session owner claim: throws SQLite's `SQLITE_BUSY`, as a statement facing the held lock does,
 * while some still wait.
 */
export function requireRecordWritesMade(path: string): void {
  if (writers.get(resolve(path))?.flush() === false) throw new SqliteError("database is locked", "SQLITE_BUSY");
}

class FileRecordWriter {
  readonly #path: string;
  readonly #ended: () => void;
  readonly #waiting: WaitingWrite[] = [];
  readonly #settled: Array<() => void> = [];
  #database: RerailDatabase | undefined;
  #users = 0;
  #waitingBytes = 0;
  #dropped = 0;
  /** A write was dropped behind the waiting writes: later ones are dropped too until those are made (`#keep`). */
  #refusing = false;
  #lockWait: AbortController | undefined;
  #transaction: { begin: Database.Statement; commit: Database.Statement; rollback: Database.Statement } | undefined;

  constructor(path: string, ended: () => void) {
    this.#path = path;
    this.#ended = ended;
  }

  use(): RecordWriter {
    // Opened with its first user, as each store opens its own: never first at a write, on the thread answering a hook.
    this.#connection();
    this.#users += 1;
    let closed = false;
    const open = (): void => {
      // As a closed connection answers a statement.
      if (closed) throw new TypeError("The database connection is not open");
    };
    return {
      write: (write, bytes = 0) => { open(); this.#write(write, bytes); },
      writeUnless: (unchanged, write, bytes = 0) => {
        open();
        if (this.#waiting.length === 0 && unchanged()) return;
        this.#write(write, bytes);
      },
      writeNow: write => { open(); return this.#writeNow(write); },
      writeWhenFree: async (write, signal) => await triedUntilFree(() => { open(); return this.#writeNow(write); }, signal),
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
      const database = openSqliteDatabase(this.#path);
      // A busy lock answers at once: the write waits in memory instead of on this thread.
      database.$client.pragma("busy_timeout = 0");
      const statement = (text: string): Database.Statement => database.$client.prepare(text);
      this.#transaction = { begin: statement("BEGIN IMMEDIATE"), commit: statement("COMMIT"), rollback: statement("ROLLBACK") };
      this.#database = database;
    }
    return this.#database;
  }

  #write(write: RecordWrite, bytes: number): void {
    if (this.#writeNow(write)) return;
    this.#keep(write, bytes);
  }

  /** Made now only behind no waiting write, so writes commit in the order they were asked for. */
  #writeNow(write: RecordWrite): boolean {
    return this.#waiting.length === 0 && this.#makeNow(write);
  }

  /**
   * True once `write` is made in one immediate transaction; false when the lock was held, before any statement ran.
   * Other failures roll it back and throw. The transaction's statements are prepared once per connection.
   */
  #makeNow(write: RecordWrite): boolean {
    const database = this.#connection();
    const transaction = this.#transaction!;
    try {
      transaction.begin.run();
    } catch (error) {
      if (isBusy(error)) return false;
      throw error;
    }
    try {
      write(database);
      transaction.commit.run();
    } catch (error) {
      if (database.$client.inTransaction) transaction.rollback.run();
      throw error;
    }
    return true;
  }

  /**
   * Keeps `write` until the lock is given back, or drops it past what may wait. Once one is dropped behind waiting writes,
   * every later one is dropped too until those are made (`#settle`): a write made without one asked before it, such as
   * the removal of a steer receipt's kept file after the receipt's record, would lose what the dropped one held.
   */
  #keep(write: RecordWrite, bytes: number): void {
    if (this.#refusing || this.#waiting.length >= MAX_WAITING_WRITES || this.#waitingBytes + bytes > MAX_WAITING_BYTES) {
      this.#dropped += 1;
      this.#refusing = this.#waiting.length > 0;
      return;
    }
    this.#waiting.push({ write, bytes, deadline: performance.now() + BUSY_TIMEOUT_MS });
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
      if (!this.#makeReported(this.#waiting[0]!.write)) {
        this.#awaitLock();
        return;
      }
      this.#shiftWaiting();
    }
    this.#settle();
  }

  /** `flushRecordWrites`: the waiting writes made on this thread; once none waits, the lock wait has nothing to do. */
  flush(): boolean {
    if (this.#waiting.length === 0) return true;
    this.#makeOnThisThread();
    if (this.#waiting.length > 0) return false;
    this.#lockWait?.abort();
    this.#lockWait = undefined;
    this.#settle();
    return true;
  }

  #shiftWaiting(): void {
    this.#waitingBytes -= this.#waiting.shift()!.bytes;
  }

  #settle(): void {
    this.#refusing = false;
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
    this.#transaction = undefined;
  }

  /**
   * One wait for the lock on this thread, then what still waits is reported lost: the last user's close, which has no
   * later turn, and a lock wait that failed.
   */
  #makeWaitingOnThisThread(): void {
    this.#makeOnThisThread();
    this.#dropped += this.#waiting.splice(0).length;
    this.#waitingBytes = 0;
    this.#settle();
  }

  /**
   * Makes the waiting writes in order, the connection waiting for the lock until the oldest one's deadline: a write
   * asked `BUSY_TIMEOUT_MS` ago or more gets one try, so this thread never waits that bound twice for the same writes.
   */
  #makeOnThisThread(): void {
    const client = this.#connection().$client;
    const waitMs = Math.max(0, Math.ceil(this.#waiting[0]!.deadline - performance.now()));
    client.pragma(`busy_timeout = ${String(waitMs)}`);
    try {
      while (this.#waiting.length > 0 && this.#makeReported(this.#waiting[0]!.write)) this.#shiftWaiting();
    } finally {
      client.pragma("busy_timeout = 0");
    }
  }
}

/**
 * `attempt` (true once it took the write lock and did its work) now and then again at the times SQLite's busy handler
 * tries, as a statement on a connection whose `busy_timeout` is `BUSY_TIMEOUT_MS` does: after each of its pauses, which
 * sum to that bound, the last one cut to fit. Each pause is a timer, so the thread answers other work between tries,
 * and each ends at its try's time from the first: a timer that fires late, as Node's do by about a millisecond, and the
 * work the thread did meanwhile shorten the next pause instead of pushing every later try back.
 */
async function triedUntilFree(attempt: () => boolean, signal: AbortSignal): Promise<boolean> {
  const startedAt = performance.now();
  for (let tries = 0, due = 0; !signal.aborted; tries += 1) {
    if (attempt()) return true;
    if (due >= BUSY_TIMEOUT_MS) return false;
    due = Math.min(due + busyPauseMs(tries), BUSY_TIMEOUT_MS);
    if (!await pauseFor(Math.max(0, startedAt + due - performance.now()), signal)) return false;
  }
  return false;
}

/** True once `ms` passed; false at once when `signal` aborts first. */
function pauseFor(ms: number, signal: AbortSignal): Promise<boolean> {
  return new Promise(resolve => {
    const aborted = (): void => {
      clearTimeout(timer);
      resolve(false);
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", aborted);
      resolve(true);
    }, ms);
    signal.addEventListener("abort", aborted, { once: true });
  });
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
