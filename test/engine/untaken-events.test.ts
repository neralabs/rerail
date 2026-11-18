import { spawn } from "node:child_process";
import { once } from "node:events";
import { chmodSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp, readdir, rename } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setImmediate as nextTurn } from "node:timers/promises";
import { afterEach, describe, expect, it } from "vitest";
import {
  leaveUntaken,
  UNTAKEN_EVENTS_DIR,
  UNTAKEN_MOST_BYTES,
  UNTAKEN_MOST_EVENTS,
  untakenEventTaker,
  UntakenFolderFullError,
  untakenFolderId,
  untakenFor,
  untakenWaiting,
  type UntakenEvent,
  type UntakenHandler,
  type UntakenPass,
} from "../../src/engine/untaken-events.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const homes: string[] = [];
async function home(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-untaken-"));
  homes.push(path);
  return path;
}

afterEach(async () => {
  await Promise.all(homes.splice(0).map(path => removeFolder(path)));
});

/** A handler that keeps what it is handed and each pass it hears of; `take` ends at once unless `held` says otherwise. */
function keeper(held?: Promise<void>): UntakenHandler & { events: UntakenEvent[]; passes: UntakenPass[] } {
  const events: UntakenEvent[] = [];
  const passes: UntakenPass[] = [];
  return {
    events,
    passes,
    take: async event => {
      events.push(event);
      await held;
    },
    passed: pass => passes.push(pass),
  };
}

/** The event files in `rerailHome`'s folder, the taken mark left out. */
async function eventFiles(rerailHome: string): Promise<string[]> {
  return (await readdir(join(rerailHome, UNTAKEN_EVENTS_DIR))).filter(name => name.endsWith(".json"));
}

/** An event's name in the folder, as `leaveUntaken` names one, for the daemon 7. */
const eventName = (arrivedAt: number): string => `${String(arrivedAt).padStart(13, "0")}-7-1-${arrivedAt}-5.json`;

/**
 * A program other than Rerail holding `path` open, as Windows lets one: `share` is what it lets others do meanwhile
 * ("Read" lets the daemon read it but no one remove it; "None" lets no one do either). Ends when `release` is awaited.
 */
async function heldOpen(path: string, share: "Read" | "None"): Promise<{ release: () => Promise<void> }> {
  const script = `$f = [System.IO.File]::Open('${path}', 'Open', 'Read', '${share}'); [Console]::Out.WriteLine('held'); [void][Console]::In.ReadLine(); $f.Close()`;
  const holder = spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
  const exited = once(holder, "exit");
  for await (const chunk of holder.stdout) if (String(chunk).includes("held")) break;
  return {
    release: async () => {
      holder.stdin.end();
      await exited;
    },
  };
}

describe("the home's untaken events", () => {
  it("are taken in the order their hooks met them, each removed once handed on, and name the daemon they were left for", async () => {
    const rerailHome = await home();
    await leaveUntaken(rerailHome, 41, "third", 3_000);
    await leaveUntaken(rerailHome, 41, "first", 1_000);
    await leaveUntaken(rerailHome, 42, "second", 2_000);
    const folder = untakenFolderId(rerailHome)!;
    expect(untakenFor(rerailHome, 41, folder)).toBe(true);
    expect(untakenFor(rerailHome, 43, folder)).toBe(false);
    expect(untakenFor(rerailHome, 41, `${folder}0`)).toBe(false);
    const handler = keeper();
    await untakenEventTaker(rerailHome, handler).taken();
    expect(handler.events).toEqual([
      { body: "first", arrivedAt: 1_000 },
      { body: "second", arrivedAt: 2_000 },
      { body: "third", arrivedAt: 3_000 },
    ]);
    expect(await eventFiles(rerailHome)).toEqual([]);
    expect(untakenFor(rerailHome, 41, folder)).toBe(false);
  });

  it("are taken as they are left once the daemon watches its home, with no request and no timer", async () => {
    const rerailHome = await home();
    const taken = Promise.withResolvers<UntakenEvent>();
    const taker = untakenEventTaker(rerailHome, { take: async event => taken.resolve(event), passed: () => undefined });
    try {
      expect(taker.watch(() => undefined)).toBe(untakenFolderId(rerailHome));
      await leaveUntaken(rerailHome, 7, "left later", 5_000);
      expect(await taken.promise).toEqual({ body: "left later", arrivedAt: 5_000 });
    } finally {
      taker.close();
    }
  });

  it("orders two events one hook left in the same millisecond as it left them", async () => {
    const rerailHome = await home();
    for (let count = 0; count < 11; count += 1) await leaveUntaken(rerailHome, 7, `event ${count}`, 9_000);
    const handler = keeper();
    await untakenEventTaker(rerailHome, handler).taken();
    expect(handler.events.map(event => event.body)).toEqual(Array.from({ length: 11 }, (_, count) => `event ${count}`));
  });

  // A watch follows its folder's change notices. On Windows a moved folder sends its watch none, and a removed one fails
  // it; the state file's folder id then keeps hooks asking the daemon (hook-failures.test.ts).
  it.runIf(process.platform !== "win32")("are taken by a watch that follows its folder once it was removed or moved, which names each new folder", async () => {
    const rerailHome = await home();
    const taken: string[] = [];
    const watched: (string | undefined)[] = [];
    let next = Promise.withResolvers<void>();
    const taker = untakenEventTaker(rerailHome, {
      take: async event => {
        taken.push(event.body);
        next.resolve();
      },
      passed: () => undefined,
    });
    try {
      watched.push(taker.watch(folder => watched.push(folder)));
      await removeFolder(join(rerailHome, UNTAKEN_EVENTS_DIR));
      await leaveUntaken(rerailHome, 7, "after the removal", 1_000);
      await next.promise;
      next = Promise.withResolvers<void>();
      await rename(join(rerailHome, UNTAKEN_EVENTS_DIR), join(rerailHome, "moved"));
      await leaveUntaken(rerailHome, 7, "after the move", 2_000);
      await next.promise;
      next = Promise.withResolvers<void>();
      await leaveUntaken(rerailHome, 7, "later still", 3_000);
      await next.promise;
      expect(taken).toEqual(["after the removal", "after the move", "later still"]);
      expect(watched.at(-1)).toBe(untakenFolderId(rerailHome));
      expect(watched).not.toContain(undefined);
    } finally {
      taker.close();
    }
  });

  it("hand a backlog on a slice per turn of the event loop, oldest first, though the watch and a later call ask for them meanwhile", async () => {
    const rerailHome = await home();
    for (let count = 0; count < 100; count += 1) await leaveUntaken(rerailHome, 7, `event ${count}`, 1_000 + count);
    const handler = keeper();
    const taker = untakenEventTaker(rerailHome, handler);
    try {
      taker.watch(() => undefined);
      const first = taker.taken();
      // The pass's own removals wake the watch, and an event sent meanwhile asks too; neither takes the rest at once.
      const second = taker.taken();
      const seen: number[] = [];
      while (handler.events.length < 100) {
        await nextTurn();
        seen.push(handler.events.length);
      }
      const steps = seen.map((count, index) => count - (seen[index - 1] ?? 0));
      expect(Math.max(...steps)).toBeLessThanOrEqual(16);
      await Promise.all([first, second]);
      expect(handler.events.map(event => event.body)).toEqual(Array.from({ length: 100 }, (_, count) => `event ${count}`));
      expect(await eventFiles(rerailHome)).toEqual([]);
    } finally {
      taker.close();
    }
  });

  // A folder's mode keeps no one from removing a file in it on Windows, and root removes it anyway.
  it.runIf(process.platform !== "win32" && process.getuid?.() !== 0)("are counted by no hook once a pass has begun to take them, and a file the daemon cannot remove is handed on once", async () => {
    const rerailHome = await home();
    await leaveUntaken(rerailHome, 7, "first", 1_000);
    const folder = untakenFolderId(rerailHome)!;
    const handler = keeper();
    // A folder its owner may not write keeps the daemon from removing what it took, as a program Windows lets hold a
    // file does. Taken away as the event is handed on, after the pass wrote its taken mark.
    const take = handler.take;
    handler.take = async event => {
      chmodSync(join(rerailHome, UNTAKEN_EVENTS_DIR), 0o500);
      await take(event);
    };
    const taker = untakenEventTaker(rerailHome, handler);
    try {
      await taker.taken();
      await taker.taken();
      expect(handler.events.map(event => event.body)).toEqual(["first"]);
      expect(untakenFor(rerailHome, 7, folder)).toBe(false);
      expect(untakenWaiting(rerailHome, 7)).toBeUndefined();
    } finally {
      chmodSync(join(rerailHome, UNTAKEN_EVENTS_DIR), 0o700);
    }
    handler.take = take;
    await leaveUntaken(rerailHome, 7, "second", 2_000);
    expect(untakenFor(rerailHome, 7, folder)).toBe(true);
    expect(untakenWaiting(rerailHome, 7)).toEqual({ events: 1, since: 2_000 });
    await taker.taken();
    expect(handler.events.map(event => event.body)).toEqual(["first", "second"]);
    expect(await eventFiles(rerailHome)).toEqual([]);
  });

  it("are kept up to their bound, past which a hook's event is refused, by count and by bytes", async () => {
    const rerailHome = await home();
    for (let count = 0; count < UNTAKEN_MOST_EVENTS; count += 1) await leaveUntaken(rerailHome, 7, "e", 1_000 + count);
    await expect(leaveUntaken(rerailHome, 7, "one more", 9_000)).rejects.toBeInstanceOf(UntakenFolderFullError);
    const other = await home();
    await leaveUntaken(other, 7, "x".repeat(UNTAKEN_MOST_BYTES - 10), 1_000);
    await expect(leaveUntaken(other, 7, "x".repeat(11), 2_000)).rejects.toBeInstanceOf(UntakenFolderFullError);
    await leaveUntaken(other, 7, "x".repeat(10), 3_000);
  });

  it("fail a name the daemon cannot read in the pass that met it, never again, and count it for no hook", async () => {
    const rerailHome = await home();
    await leaveUntaken(rerailHome, 7, "first", 1_000);
    // A folder under an event's name: no read and no removal takes it, on every system.
    mkdirSync(join(rerailHome, UNTAKEN_EVENTS_DIR, eventName(3_000)));
    const folder = untakenFolderId(rerailHome)!;
    const handler = keeper();
    let reported = Promise.withResolvers<void>();
    const taker = untakenEventTaker(rerailHome, {
      take: handler.take,
      passed: pass => {
        handler.passes.push(pass);
        reported.resolve();
      },
    });
    try {
      taker.watch(() => undefined);
      await taker.taken();
      await reported.promise;
      // Before, each of these passes read it again and reported it again, and the pass's taken mark woke the watch for
      // the next: one such file kept the daemon logging it as lost thousands of times a second.
      await taker.taken();
      await taker.taken();
      expect(untakenFor(rerailHome, 7, folder)).toBe(false);
      expect(untakenWaiting(rerailHome, 7)).toBeUndefined();
      reported = Promise.withResolvers<void>();
      await leaveUntaken(rerailHome, 7, "second", 4_000);
      await taker.taken();
      await reported.promise;
      expect(handler.events.map(event => event.body)).toEqual(["first", "second"]);
      expect(handler.passes).toEqual([{ events: 2, from: 1_000, to: 3_000, failed: 1 }, { events: 1, from: 4_000, to: 4_000, failed: 0 }]);
      expect(existsSync(join(rerailHome, UNTAKEN_EVENTS_DIR, eventName(3_000)))).toBe(true);
    } finally {
      taker.close();
    }
  });

  // A file mode keeps no one from reading on Windows, and root reads it anyway.
  it.runIf(process.platform !== "win32" && process.getuid?.() !== 0)("remove a file the daemon cannot read once its pass failed it", async () => {
    const rerailHome = await home();
    await leaveUntaken(rerailHome, 7, "unread", 1_000);
    const [name] = await eventFiles(rerailHome);
    chmodSync(join(rerailHome, UNTAKEN_EVENTS_DIR, name!), 0o000);
    const handler = keeper();
    const taker = untakenEventTaker(rerailHome, handler);
    await taker.taken();
    await taker.taken();
    expect(handler.events).toEqual([]);
    expect(await eventFiles(rerailHome)).toEqual([]);
  });

  it("move the taken mark only on, so a name the daemon could not take stays counted by no hook once an older event is taken", async () => {
    const rerailHome = await home();
    mkdirSync(join(rerailHome, UNTAKEN_EVENTS_DIR), { recursive: true });
    mkdirSync(join(rerailHome, UNTAKEN_EVENTS_DIR, eventName(2_000)));
    await leaveUntaken(rerailHome, 7, "newest", 3_000);
    const taker = untakenEventTaker(rerailHome, keeper());
    await taker.taken();
    // An event its hook met before the newest one, left once that was taken: a hook that waited out a busy daemon.
    await leaveUntaken(rerailHome, 7, "older", 1_000);
    await taker.taken();
    // Before, the mark went back to the older event, and hooks counted the name that stays as waiting.
    expect(untakenWaiting(rerailHome, 7)).toBeUndefined();
    expect(untakenFor(rerailHome, 7, untakenFolderId(rerailHome)!)).toBe(false);
  });

  // Windows lets a program hold a file so that no one can remove it, or read it either; other systems have no such hold.
  it.runIf(process.platform === "win32").each(["Read", "None"] as const)(
    "count for no hook, and remove at a later pass, a file another program holds sharing %s, handed on at most once",
    async share => {
      const rerailHome = await home();
      await leaveUntaken(rerailHome, 7, "held", 1_000);
      const [name] = await eventFiles(rerailHome);
      const folder = untakenFolderId(rerailHome)!;
      const handler = keeper();
      const taker = untakenEventTaker(rerailHome, handler);
      const holder = await heldOpen(join(rerailHome, UNTAKEN_EVENTS_DIR, name!), share);
      try {
        await taker.taken();
        await taker.taken();
        expect(await eventFiles(rerailHome)).toEqual([name]);
        expect(untakenFor(rerailHome, 7, folder)).toBe(false);
        expect(untakenWaiting(rerailHome, 7)).toBeUndefined();
      } finally {
        await holder.release();
      }
      await taker.taken();
      expect(await eventFiles(rerailHome)).toEqual([]);
      expect(handler.events.map(event => event.body)).toEqual(share === "Read" ? ["held"] : []);
    },
  );

  it("report each pass once its checks end, with how many failed", async () => {
    const rerailHome = await home();
    await leaveUntaken(rerailHome, 7, "first", 1_000);
    await leaveUntaken(rerailHome, 7, "second", 2_000);
    writeFileSync(join(rerailHome, UNTAKEN_EVENTS_DIR, "0000000003000-7-1-1-5.json"), "third");
    const reported = Promise.withResolvers<UntakenPass>();
    const checks = Promise.withResolvers<void>();
    let handed = 0;
    const taker = untakenEventTaker(rerailHome, {
      take: async ({ body }) => {
        handed += 1;
        await checks.promise;
        if (body === "second") throw new Error("not an engine request");
      },
      passed: pass => reported.resolve(pass),
    });
    await taker.taken();
    expect(handed).toBe(3);
    checks.resolve();
    expect(await reported.promise).toEqual({ events: 3, from: 1_000, to: 3_000, failed: 1 });
  });
});
