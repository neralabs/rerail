import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { folderRemoved, KeptRuntimes, type KeptRuntimesOptions } from "../../../src/runtime/memory/kept-runtimes.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/**
 * Kept runtimes over open runtimes named `keys`, each in the folder of the same name; `room.short` says the daemon needs
 * the room, `using` holds the keys a session uses, and `closed` the keys closed, in order.
 */
function kept(keys: readonly string[], options: Partial<KeptRuntimesOptions> = {}) {
  const open = new Set(keys);
  const using = new Set<string>();
  const closed: string[] = [];
  const room = { short: false, reads: 0 };
  const runtimes = new KeptRuntimes({
    needsRoom: () => {
      room.reads += 1;
      return room.short;
    },
    inUse: key => using.has(key),
    folderOf: key => (open.has(key) ? key : undefined),
    close: async (key) => {
      open.delete(key);
      closed.push(key);
    },
    removed: async () => false,
    ...options,
  });
  return { runtimes, using, closed, room };
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

  it("closes every kept runtime but the one left last at the sweep while the daemon needs the room, and all of them as a runtime opens", async () => {
    const { runtimes, closed, room } = kept(["a", "b", "c", "d"]);
    for (const key of ["a", "b", "c"]) runtimes.left(key);
    runtimes.sweep();
    await vi.waitFor(() => expect(room.reads).toBe(1));
    expect(closed).toEqual([]);
    room.short = true;
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["a", "b"]));
    runtimes.sweep();
    runtimes.left("d");
    runtimes.beforeOpen();
    await vi.waitFor(() => expect(closed).toEqual(["a", "b", "c", "d"]));
  });

  it("reads no memory at a sweep while one runtime or none is kept, nor as a runtime opens while none is", () => {
    const { runtimes, room } = kept(["a"]);
    runtimes.sweep();
    runtimes.beforeOpen();
    runtimes.left("a");
    runtimes.sweep();
    expect(room.reads).toBe(0);
  });

  it("keeps the order in which runtimes were last left: one used and left again is the one left last", async () => {
    const { runtimes, closed, room } = kept(["a", "b"]);
    runtimes.left("a");
    runtimes.left("b");
    runtimes.claim("a")();
    room.short = true;
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["b"]));
  });

  // A scan's open came for a quiet session while its close ran: the close's release kept the runtime, and the open used
  // it without taking it out, so the next room check closed it under the reopened session.
  it("keeps no runtime a use is on its way to, and keeps it again once that use ends with no session in it", async () => {
    const { runtimes, closed, room, using } = kept(["a", "b"]);
    const release = runtimes.claim("a");
    runtimes.left("a");
    room.short = true;
    await runtimes.closeAll();
    expect(closed).toEqual([]);
    using.add("a");
    release();
    await runtimes.closeAll();
    expect(closed).toEqual([]);
    using.delete("a");
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
    const { runtimes, closed, using } = kept(["a"]);
    runtimes.left("a");
    using.add("a");
    await runtimes.closeAll();
    expect(closed).toEqual([]);
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

  it("keeps the one left last among those no session uses at the sweep, though a session came back to one left after it", async () => {
    const { runtimes, closed, room } = kept(["a", "b", "c"]);
    for (const key of ["a", "b", "c"]) runtimes.left(key);
    runtimes.claim("c");
    room.short = true;
    runtimes.sweep();
    await vi.waitFor(() => expect(closed).toEqual(["a"]));
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
    const { runtimes, closed, room } = kept(["a", "b", "c"], {
      removed: () => {
        looks += 1;
        return new Promise<boolean>(() => undefined);
      },
    });
    for (const key of ["a", "b", "c"]) runtimes.left(key);
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
