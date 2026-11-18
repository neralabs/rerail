import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { folderRemoved, KeptRuntimes, type KeptRuntimesOptions } from "../../../src/runtime/memory/kept-runtimes.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const MINUTE = 60_000;

/**
 * Kept runtimes over open runtimes named `keys`, each in the folder of the same name; `room.short` says the daemon needs
 * the room, `busy` holds the keys a session uses, `closed` the keys closed, in order, `collections` counts the full
 * collections asked for, and `clock.now` is the time.
 */
function kept(keys: readonly string[], options: Partial<KeptRuntimesOptions> = {}) {
  const open = new Set(keys);
  const busy = new Set<string>();
  const closed: string[] = [];
  const room = { short: false, reads: 0 };
  const clock = { now: 1_000_000 };
  const counts = { collections: 0 };
  const runtimes = new KeptRuntimes({
    needsRoom: () => {
      room.reads += 1;
      return room.short;
    },
    inUse: key => busy.has(key),
    folderOf: key => (open.has(key) ? key : undefined),
    close: async (key) => {
      open.delete(key);
      closed.push(key);
    },
    collect: () => {
      counts.collections += 1;
    },
    now: () => clock.now,
    removed: async () => false,
    ...options,
  });
  return { runtimes, busy, closed, room, clock, counts };
}

describe("the runtimes no session uses, kept for their checkout's next session", () => {
  // Leaving frees nothing a new allocation needs: a /clear starts the next session in the same checkout a moment later.
  it("closes nothing as a session leaves, though the daemon needs the room", async () => {
    const { runtimes, closed, room } = kept(["a"]);
    room.short = true;
    runtimes.left("a");
    await Promise.resolve();
    expect(closed).toEqual([]);
  });

  it("keeps every runtime no session uses while the daemon has room, however long ago its last session left", async () => {
    const { runtimes, closed, room, clock } = kept(["a", "b"]);
    runtimes.left("a");
    runtimes.left("b");
    clock.now += 24 * 60 * MINUTE;
    runtimes.sweep();
    await Promise.resolve();
    expect(room.reads).toBe(1);
    expect(closed).toEqual([]);
  });

  // Every runtime no session used closed 15 minutes after its last session left, whatever the memory. Short of room, a
  // kept runtime is freed at that time again, the one left last too, and none sooner: a session back within it finds
  // its rules compiled as it did.
  it("closes at the sweep each runtime kept 15 minutes or longer while the daemon needs the room, the one left last too, and none kept less", async () => {
    const { runtimes, closed, room, clock } = kept(["a", "b", "c"]);
    room.short = true;
    runtimes.left("a");
    clock.now += 5 * MINUTE;
    runtimes.left("b");
    clock.now += 9 * MINUTE;
    runtimes.left("c");
    runtimes.sweep();
    await Promise.resolve();
    expect(closed).toEqual([]);
    clock.now += MINUTE;
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["a"]));
    clock.now += 5 * MINUTE;
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["a", "b"]));
    clock.now += 15 * MINUTE;
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["a", "b", "c"]));
  });

  it("reads no memory at a sweep while no runtime has been kept 15 minutes", () => {
    const { runtimes, room, clock } = kept(["a"]);
    runtimes.sweep();
    runtimes.left("a");
    clock.now += 15 * MINUTE - 1;
    runtimes.sweep();
    expect(room.reads).toBe(0);
  });

  it("counts a runtime's 15 minutes from the last time a use of it ended", async () => {
    const { runtimes, closed, room, clock } = kept(["a"]);
    room.short = true;
    runtimes.left("a");
    clock.now += 14 * MINUTE;
    runtimes.claim("a")();
    clock.now += 14 * MINUTE;
    runtimes.sweep();
    await Promise.resolve();
    expect(closed).toEqual([]);
    clock.now += MINUTE;
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["a"]));
  });

  // Each close ran a full collection of its own, one after another on the event loop: a batch of kept runtimes took one
  // collection per runtime.
  it("collects once for a batch of closes, once each has closed", async () => {
    const closing = Promise.withResolvers<void>();
    const { runtimes, closed, counts } = kept(["a", "b", "c"], {
      close: async (key) => {
        closed.push(key);
        if (key === "b") await closing.promise;
      },
    });
    for (const key of ["a", "b", "c"]) runtimes.left(key);
    const all = runtimes.closeAll();
    await Promise.resolve();
    expect(counts.collections).toBe(0);
    closing.resolve();
    await all;
    expect(closed).toEqual(["a", "b", "c"]);
    expect(counts.collections).toBe(1);
    await runtimes.closeAll();
    expect(counts.collections).toBe(1);
  });

  // A quiet session's end makes or takes its runtime and notes no use of it: released as the end began, the runtime was
  // kept and closable while the end still ran in it.
  it("keeps no runtime an event's work is on its way to or running in, and keeps it again once that work ends with no session in it", async () => {
    const { runtimes, closed, room, busy } = kept(["a", "b"]);
    const release = runtimes.claim("a");
    runtimes.left("a");
    room.short = true;
    await runtimes.closeAll();
    expect(closed).toEqual([]);
    busy.add("a");
    release();
    await runtimes.closeAll();
    expect(closed).toEqual([]);
    busy.delete("a");
    runtimes.left("a");
    await runtimes.closeAll();
    expect(closed).toEqual(["a"]);
  });

  it("keeps a runtime whose claim ends with no session using it, and counts each claim", async () => {
    const { runtimes, closed } = kept(["a"]);
    const first = runtimes.claim("a");
    const second = runtimes.claim("a");
    first();
    first();
    await runtimes.closeAll();
    expect(closed).toEqual([]);
    second();
    await runtimes.closeAll();
    expect(closed).toEqual(["a"]);
  });

  it("closes no runtime a session uses, should one be kept", async () => {
    const { runtimes, closed, busy, counts } = kept(["a"]);
    runtimes.left("a");
    busy.add("a");
    await runtimes.closeAll();
    expect(closed).toEqual([]);
    expect(counts.collections).toBe(0);
  });

  it("settles a close of every kept runtime once each has closed", async () => {
    const closing = Promise.withResolvers<void>();
    const { runtimes } = kept(["a"], { close: async () => await closing.promise });
    runtimes.left("a");
    let settled = false;
    const all = runtimes.closeAll().then(() => {
      settled = true;
    });
    await Promise.resolve();
    expect(settled).toBe(false);
    closing.resolve();
    await all;
    expect(settled).toBe(true);
  });

  // The engine service closes its runtimes itself as it closes: closing one here as well would close it twice.
  it("keeps nothing once stopped, and a look that answers after closes nothing", async () => {
    const answer = Promise.withResolvers<boolean>();
    const { runtimes, closed } = kept(["a"], { removed: async () => await answer.promise });
    runtimes.left("a");
    runtimes.sweep();
    runtimes.stop();
    answer.resolve(true);
    await new Promise(resolve => setImmediate(resolve));
    runtimes.left("a");
    await runtimes.closeAll();
    expect(closed).toEqual([]);
  });
});

describe("the look for a kept runtime's removed checkout", () => {
  it("closes a kept runtime whose folder was removed, though the daemon has room", async () => {
    const { runtimes, closed } = kept(["a", "b"], { removed: async folder => folder === "b" });
    runtimes.left("a");
    runtimes.left("b");
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["b"]));
  });

  // A checkout on a share that stops answering: a look on the event loop held every hook for as long as the share did.
  it("looks off the sweep's own path, one look at a time, so a folder that never answers holds no sweep", async () => {
    let looks = 0;
    const { runtimes, closed, room, clock } = kept(["a", "b", "c"], {
      removed: () => {
        looks += 1;
        return new Promise<boolean>(() => undefined);
      },
    });
    runtimes.left("a");
    runtimes.left("b");
    clock.now += 15 * MINUTE;
    runtimes.left("c");
    room.short = true;
    runtimes.sweep();
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["a", "b"]));
    expect(looks).toBe(1);
  });

  it("reads a folder as removed only when it is not there, not when reading it fails", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kept-"));
    try {
      expect(await folderRemoved(root)).toBe(false);
      expect(await folderRemoved(join(root, "gone"))).toBe(true);
      expect(await folderRemoved(join(root, "gone", "deeper"))).toBe(true);
      expect(await folderRemoved(root + "\0")).toBe(false);
    } finally {
      await removeFolder(root);
    }
  });
});
