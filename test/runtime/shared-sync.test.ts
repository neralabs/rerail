import { realpathSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteSessionStore } from "../../src/runtime/session-store.js";
import { SharedSync, syncFile } from "../../src/runtime/shared-sync.js";

// Every file `syncFile` syncs, in order; it still syncs them.
const synced = vi.hoisted((): string[] => []);
vi.mock("../../src/runtime/shared-sync.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../src/runtime/shared-sync.js")>();
  return {
    ...actual,
    syncFile: async (path: string): Promise<void> => {
      synced.push(path);
      await actual.syncFile(path);
    },
  };
});

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDirectory(): Promise<string> {
  const path = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-shared-sync-")));
  temporaryDirectories.push(path);
  return path;
}

/** Syncs that end or fail when the test says, in the order they began. */
function heldSyncs() {
  const runs: Array<{ end(): void; fail(error: Error): void }> = [];
  const sync = () => new Promise<void>((resolve, reject) => { runs.push({ end: () => resolve(), fail: reject }); });
  return { runs, sync };
}

/** Lets every promise settled so far run its callbacks. */
const settled = () => new Promise(resolve => setImmediate(resolve));

describe("SharedSync", () => {
  it("begins a sync for a write when none runs, and settles when it ends", async () => {
    const { runs, sync } = heldSyncs();
    const shared = new SharedSync(sync);
    let done = false;
    void shared.sync().then(() => { done = true; });
    expect(runs).toHaveLength(1);
    await settled();
    expect(done).toBe(false);
    runs[0]!.end();
    await settled();
    expect(done).toBe(true);
  });

  it("makes the writes that come while a sync runs wait for the next one, which they all share", async () => {
    const { runs, sync } = heldSyncs();
    const shared = new SharedSync(sync);
    const done: string[] = [];
    void shared.sync().then(() => done.push("first"));
    void shared.sync().then(() => done.push("second"));
    void shared.sync().then(() => done.push("third"));
    expect(runs).toHaveLength(1);
    runs[0]!.end();
    await settled();
    // The first sync may have begun before the second and third writes, so it covers only the first.
    expect(done).toEqual(["first"]);
    expect(runs).toHaveLength(2);
    runs[1]!.end();
    await settled();
    expect(done).toEqual(["first", "second", "third"]);
    expect(runs).toHaveLength(2);
    void shared.sync();
    expect(runs).toHaveLength(3);
    runs[2]!.end();
  });

  it("fails only the writes a failed sync was to cover", async () => {
    const { runs, sync } = heldSyncs();
    const shared = new SharedSync(sync);
    const first = shared.sync();
    const second = shared.sync();
    runs[0]!.fail(new Error("the disk failed"));
    await expect(first).rejects.toThrow("the disk failed");
    await settled();
    runs[1]!.end();
    await expect(second).resolves.toBeUndefined();
  });
});

describe("syncFile", () => {
  it("syncs a file through a handle of its own, and fails for one that is not there", async () => {
    const root = await temporaryDirectory();
    const path = join(root, "file");
    await writeFile(path, "written");
    await expect(syncFile(path)).resolves.toBeUndefined();
    await expect(syncFile(join(root, "missing"))).rejects.toMatchObject({ code: "ENOENT" });
  });
});

describe("SqliteSessionStore.appendSynced", () => {
  it("syncs the write-ahead log SQLite wrote the event to, once for the events that commit while a sync runs", async () => {
    const root = await temporaryDirectory();
    const path = join(root, "rerail.sqlite");
    const store = new SqliteSessionStore(path);
    try {
      store.openSession({ host: "claude-code", sessionId: "session-1", cwd: "/workspace" }, 100);
      synced.splice(0);
      await Promise.all([1, 2, 3].map(index => store.appendSynced({
        sessionId: "session-1", eventId: `event-${index}`, kind: "result", observedAt: 100 + index, payload: { output: "ok" },
      })));
      // One sync for the first event, one shared by the two that committed while it ran.
      expect(synced.map(file => realpathSync.native(file))).toEqual(Array.from({ length: 2 }, () => realpathSync.native(`${path}-wal`)));
    } finally {
      store.close();
    }
  });
});
