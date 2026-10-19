import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { installedPacksPrefix } from "../../src/engine/index.js";
import { withPacksLock } from "../../src/setup/packs-lock.js";

const homes: string[] = [];
afterEach(() => {
  for (const home of homes.splice(0)) rmSync(home, { recursive: true, force: true });
});

function temporaryHome(): string {
  const home = mkdtempSync(join(tmpdir(), "rerail-packs-lock-"));
  homes.push(home);
  return home;
}

const lockOf = (home: string): string => join(installedPacksPrefix(home), ".lock");

/** A lock file as another process's `withPacksLock` writes it. */
function heldBy(home: string, pid: number, at = Date.now()): void {
  mkdirSync(installedPacksPrefix(home), { recursive: true });
  writeFileSync(lockOf(home), JSON.stringify({ pid, token: "other", at }));
}

describe("withPacksLock", () => {
  it("runs one change at a time: a second waits until the first is done, then runs", async () => {
    const home = temporaryHome();
    const events: string[] = [];
    const firstMayEnd = Promise.withResolvers<void>();
    const first = withPacksLock(home, async () => {
      events.push("first starts");
      await firstMayEnd.promise;
      events.push("first ends");
    });
    // The second finds the lock held; its first wait lets the first end. No real time passes.
    const second = withPacksLock(home, async () => { events.push("second runs"); }, {
      sleep: async () => { firstMayEnd.resolve(); await first; },
    });

    await Promise.all([first, second]);

    expect(events).toEqual(["first starts", "first ends", "second runs"]);
    expect(existsSync(lockOf(home))).toBe(false);
  });

  it("takes over a lock whose process is gone", async () => {
    const home = temporaryHome();
    heldBy(home, 999_999);

    expect(await withPacksLock(home, async () => "ran", { running: () => false, sleep: async () => { throw new Error("waited"); } })).toBe("ran");
  });

  it("takes over a lock older than any change takes, even if its pid now names a running process", async () => {
    const home = temporaryHome();
    heldBy(home, process.pid, Date.now() - 31 * 60_000);

    expect(await withPacksLock(home, async () => "ran", { sleep: async () => { throw new Error("waited"); } })).toBe("ran");
  });

  it("gives up after the wait, naming the process that holds it, and leaves its lock alone", async () => {
    const home = temporaryHome();
    heldBy(home, 4242);
    let ran = false;

    await expect(withPacksLock(home, async () => { ran = true; }, { running: () => true, waitMs: 1_000, sleep: async () => {} }))
      .rejects.toThrow(/Another Rerail process \(pid 4242\) is changing the rule packs/u);
    expect(ran).toBe(false);
    expect(existsSync(lockOf(home))).toBe(true);
  });

  it("releases the lock when the change throws", async () => {
    const home = temporaryHome();

    await expect(withPacksLock(home, async () => { throw new Error("npm failed"); })).rejects.toThrow("npm failed");
    expect(existsSync(lockOf(home))).toBe(false);
  });
});
