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
  database.close();
  return path;
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
