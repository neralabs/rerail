// The daemon's sync of the history in the background (`startHistorySync`): while commits wait, the log reaches the
// disk about every period, one sync at a time, the next due a period after the last one ended; an idle daemon does no
// disk work; it stops with the daemon; a failed sync is said once and tried again. The period is injected, so no case
// waits 5 s on the real clock.
import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RerailDaemon, startHistorySync, type HistorySync } from "../../src/daemon/index.js";
import { openRerailDatabase, type RerailDatabase } from "../../src/engine/index.js";
import { cutPowerInCopy } from "../support/engine/power-cut.js";
import { recordLogSyncs, syncedLength } from "../support/engine/log-syncs.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";

const PERIOD_MS = 1_000;
const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  vi.useRealTimers();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** A store with one table, open as the daemon's writers are, and a commit to make in it. */
function store(): { readonly path: string; readonly database: RerailDatabase; commit(): void } {
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "rerail-history-sync-")));
  cleanups.push(() => removeFolderSync(folder));
  const path = join(folder, "rerail.sqlite");
  const database = openRerailDatabase(path);
  cleanups.push(() => { database.$client.close(); });
  database.$client.exec("CREATE TABLE noted (id INTEGER PRIMARY KEY, at INTEGER NOT NULL)");
  const insert = database.$client.prepare("INSERT INTO noted (at) VALUES (?)");
  return { path, database, commit: () => void insert.run(Date.now()) };
}

/** A sync that ends only when the case says so, counting its calls. */
function heldSync(): { readonly calls: number; readonly sync: () => Promise<void>; end(error?: Error): void } {
  const ends: ((error?: Error) => void)[] = [];
  return {
    get calls() { return ends.length; },
    sync: async () => await new Promise<void>((resolve, reject) => ends.push(error => (error === undefined ? resolve() : reject(error)))),
    end: error => ends.at(-1)!(error),
  };
}

function started(path: string, options: Parameters<typeof startHistorySync>[1]): HistorySync {
  const sync = startHistorySync(path, options);
  cleanups.push(async () => await sync.stop());
  return sync;
}

describe("the daemon's sync of the history", () => {
  it("syncs one at a time, the next due a period after the last one ended", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { path, commit } = store();
    const held = heldSync();
    started(path, { periodMs: PERIOD_MS, sync: held.sync });
    commit();
    await vi.advanceTimersByTimeAsync(PERIOD_MS);
    expect(held.calls).toBe(1);
    // The disk stalls the sync: commits go on, and no second sync starts beside it.
    for (let second = 0; second < 10; second += 1) {
      commit();
      await vi.advanceTimersByTimeAsync(PERIOD_MS);
    }
    expect(held.calls).toBe(1);
    held.end();
    await vi.advanceTimersByTimeAsync(PERIOD_MS - 1);
    expect(held.calls).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(held.calls).toBe(2);
    held.end();
  });

  it("does nothing while no commit came since the last sync", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { path, commit } = store();
    commit();
    const sync = vi.fn(async () => undefined);
    started(path, { periodMs: PERIOD_MS, sync });
    // The commits made before the daemon started are not its own to sync: an idle daemon touches no disk.
    await vi.advanceTimersByTimeAsync(10 * PERIOD_MS);
    expect(sync).not.toHaveBeenCalled();
    commit();
    await vi.advanceTimersByTimeAsync(PERIOD_MS);
    expect(sync).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10 * PERIOD_MS);
    expect(sync).toHaveBeenCalledTimes(1);
  });

  it("stops with the daemon: a sync under way ends first, and nothing comes after", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { path, commit } = store();
    const held = heldSync();
    const sync = started(path, { periodMs: PERIOD_MS, sync: held.sync });
    commit();
    await vi.advanceTimersByTimeAsync(PERIOD_MS);
    let stopped = false;
    const stopping = sync.stop().then(() => { stopped = true; });
    await vi.advanceTimersByTimeAsync(0);
    expect(stopped).toBe(false);
    held.end();
    await stopping;
    expect(vi.getTimerCount()).toBe(0);
    commit();
    await vi.advanceTimersByTimeAsync(10 * PERIOD_MS);
    expect(held.calls).toBe(1);
  });

  it("says a failed sync once in the daemon's log and tries again at the next period", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { path, commit } = store();
    const lines: string[] = [];
    let failures = 3;
    const sync = vi.fn(async () => {
      if (failures-- > 0) throw new Error("EIO: i/o error, fsync");
    });
    started(path, { periodMs: PERIOD_MS, sync, log: line => lines.push(line) });
    commit();
    // No commit after the first: the commits a failed sync left are still unsynced, so each period tries again.
    await vi.advanceTimersByTimeAsync(4 * PERIOD_MS);
    expect(sync).toHaveBeenCalledTimes(4);
    expect(lines).toEqual([
      "rerail daemon: syncing the history to the disk failed, trying again every 1 s: EIO: i/o error, fsync\n",
      "rerail daemon: syncing the history to the disk works again\n",
    ]);
    await vi.advanceTimersByTimeAsync(4 * PERIOD_MS);
    expect(sync).toHaveBeenCalledTimes(4);
  });

  it("runs while the daemon runs, and its close stops it", async () => {
    const { path, commit } = store();
    const synced: string[] = [];
    const daemon = await RerailDaemon.start({
      version: "test", databasePath: path, workspaceRoot: dirname(path), port: 0, remoteSink: false, checkoutPruning: false,
      historySync: { periodMs: 20, sync: async databasePath => void synced.push(databasePath) },
    });
    let closed = false;
    cleanups.push(async () => { if (!closed) await daemon.close(); });
    commit();
    for (let waited = 0; synced.length === 0 && waited < 10_000; waited += 20) await new Promise(resolve => setTimeout(resolve, 20));
    expect(synced).toEqual([path]);
    await daemon.close();
    closed = true;
    const atClose = synced.length;
    commit();
    await new Promise(resolve => setTimeout(resolve, 200));
    expect(synced).toHaveLength(atClose);
  });

  it("keeps a commit through a power cut more than one period after it", async () => {
    recordLogSyncs();
    const { path, commit } = store();
    const periodMs = 20;
    started(path, { periodMs });
    commit();
    const committedAt = Date.now();
    const kept = (): number[] => {
      const copy = cutPowerInCopy(path);
      cleanups.push(() => removeFolderSync(dirname(copy)));
      const database = new Database(copy);
      try {
        return database.prepare("SELECT count(*) FROM noted").pluck().all() as number[];
      } finally {
        database.close();
      }
    };
    // The power goes more than one period after the commit, once a sync ended or 10 s on: a disk can take a while to
    // confirm a sync (a Windows lab disk took over 200 ms), and the cut keeps only what a sync that ended put there.
    const sleep = async (ms: number): Promise<void> => await new Promise(resolve => setTimeout(resolve, ms));
    while (syncedLength(path) === 0 && Date.now() - committedAt < 10_000) await sleep(periodMs);
    await sleep(Math.max(0, 2 * periodMs - (Date.now() - committedAt)));
    expect(kept()).toEqual([1]);
  }, 30_000);
});
