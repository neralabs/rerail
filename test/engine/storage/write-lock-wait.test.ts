/**
 * The wait for another process's write lock, off the calling thread. Its stop ends the thread at once, whatever the
 * holder does, so a daemon stopping with records waiting is not kept by it. Its start failing, as it does from a hook
 * bundle that left the worker's file behind, gives the record writer that asked for it a rejection it handles, never a
 * throw into the store write that met the lock.
 */
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { removeFolder } from "../../support/platform/remove-folder.js";

const roots: string[] = [];

afterEach(async () => {
  vi.doUnmock("node:worker_threads");
  vi.resetModules();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

async function databaseFile(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-write-lock-wait-"));
  roots.push(root);
  const path = join(root, "rerail.sqlite");
  const database = new Database(path);
  // As every Rerail database is: a writer in WAL mode leaves readers be and holds only the write lock.
  database.pragma("journal_mode = WAL");
  database.exec("CREATE TABLE held (at REAL)");
  database.close();
  return path;
}

/**
 * A connection of its own that holds the write lock of the database at `path`. It syncs nothing as it commits, so it
 * gives the lock back as soon as its pages are in the log, however busy the disk.
 */
function holding(path: string): Database.Database {
  const holder = new Database(path);
  holder.pragma("synchronous = OFF");
  holder.exec("BEGIN IMMEDIATE");
  return holder;
}

/** When the first of SQLite's tries from `since` at or after `at` comes. */
function nextSqliteTry(since: number, at: number): number {
  let next = 0;
  while (since + sqliteTryAtMs(next) < at) next += 1;
  return since + sqliteTryAtMs(next);
}

/** Now, in ms, on the clock the wait's `since` is read on. */
function now(): number {
  return performance.timeOrigin + performance.now();
}

/** The pauses of SQLite's default busy handler (`sqliteDefaultBusyCallback`), in ms; the last one repeats. */
const SQLITE_BUSY_PAUSES_MS = [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100];

/** When SQLite's busy handler makes try `tries` (0, the first, at once), in ms from the first. */
function sqliteTryAtMs(tries: number): number {
  const pauses = Array.from({ length: tries }, (_, index) => SQLITE_BUSY_PAUSES_MS[index] ?? 100);
  return pauses.reduce((sum, pause) => sum + pause, 0);
}

/**
 * Gives `holder`'s lock back once `now` reaches `at`, having written a row first when `writes`, as a holder that
 * changes the database does (`rerail reset`, a migration); resolves to when it gave it back.
 */
async function releaseAt(holder: Database.Database, at: number, writes: boolean): Promise<number> {
  await new Promise(resolve => setTimeout(resolve, Math.max(0, at - now())));
  if (writes) holder.prepare("INSERT INTO held (at) VALUES (?)").run(at);
  holder.exec("COMMIT");
  return now();
}

describe("waiting for the write lock", () => {
  it("ends its thread at once when stopped while another connection keeps the lock", async () => {
    const { waitForWriteLock } = await import("../../../src/engine/storage/write-lock-wait.js");
    const held = await databaseFile();
    const holder = new Database(held);
    holder.exec("BEGIN IMMEDIATE");
    try {
      const stop = new AbortController();
      const waiting = waitForWriteLock(held, stop.signal);
      // Three whole waits of other threads, one after another and each started after the first, have it trying the held
      // lock by then.
      for (let wait = 0; wait < 3; wait += 1) await expect(waitForWriteLock(await databaseFile())).resolves.toBe(true);

      const stopped = process.hrtime.bigint();
      stop.abort();
      await expect(waiting).resolves.toBe(false);
      // A thread inside SQLite's busy wait could end only when that wait did, up to 5 s on.
      expect(Number(process.hrtime.bigint() - stopped) / 1e6).toBeLessThan(1_000);
    } finally {
      holder.close();
    }
  }, 30_000);

  // A holder that changed the database adds its pages to the write-ahead log as it commits, a moment before it gives the
  // lock back: the wait tries then, not at the next of SQLite's 100 ms pauses.
  it("answers within moments of a release whose commit changed the database, between two of SQLite's tries", async () => {
    const { waitForWriteLock } = await import("../../../src/engine/storage/write-lock-wait.js");
    const path = await databaseFile();
    const holder = holding(path);
    try {
      const since = now();
      const answered = waitForWriteLock(path, undefined, since).then(now);
      // 5 ms after one of the tries SQLite's busy handler makes from `since`, 95 ms before the next.
      const released = await releaseAt(holder, since + sqliteTryAtMs(15) + 5, true);
      // Well before that next try, which the busy handler's times alone would wait for.
      expect(await answered).toBeLessThan(nextSqliteTry(since, released) - 20);
    } finally {
      holder.close();
    }
  }, 30_000);

  // As the caller's own statement would have, waiting in SQLite's busy handler from the moment it met the lock: a wait
  // whose tries counted from its own start, which comes milliseconds later, would find a release that much later.
  it("tries no later than SQLite's busy handler would, counted from when its caller met the lock, not from its own start", async () => {
    const { waitForWriteLock } = await import("../../../src/engine/storage/write-lock-wait.js");
    for (let round = 0; round < 3; round += 1) {
      // How long a wait takes to start and answer: tries counted from its start would fall at most that much after the
      // call.
      const free = now();
      await waitForWriteLock(await databaseFile());
      const start = now() - free;
      const path = await databaseFile();
      const holder = holding(path);
      try {
        const calledAt = now();
        // Just after the first try past the busy handler's short pauses counted from the wait's start (that start comes
        // no later than `start`), 95 ms before the next; 10 ms before one of the handler's tries counted from a caller
        // that met the lock half a second before the call.
        const release = calledAt + start + sqliteTryAtMs(11) + 5;
        const since = release + 10 - sqliteTryAtMs(18);
        const answered = waitForWriteLock(path, undefined, since).then(now);
        const released = await releaseAt(holder, release, false);
        // Up to 35 ms for the answer's way back and the timers' steps, which are about 16 ms on Windows.
        expect(await answered).toBeLessThanOrEqual(nextSqliteTry(since, released) + 35);
      } finally {
        holder.close();
      }
    }
  }, 30_000);

  it("rejects, and never throws, when its worker cannot start", async () => {
    vi.doMock("node:worker_threads", () => ({
      Worker: class {
        constructor() {
          throw new Error("Cannot find module write-lock-wait-worker.js");
        }
      },
    }));
    const { waitForWriteLock } = await import("../../../src/engine/storage/write-lock-wait.js");
    let waiting: Promise<boolean> | undefined;
    expect(() => { waiting = waitForWriteLock("/no/database.sqlite"); }).not.toThrow();
    await expect(waiting).rejects.toThrow(/write-lock-wait-worker/u);
  });
});
