import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { withPacksLock } from "../../src/setup/packs-lock.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";
import { raceForLock, raceSize, sourceModule } from "../support/processes/lock-race.js";

const homes: string[] = [];
afterEach(() => {
  for (const home of homes.splice(0)) removeFolderSync(home);
});

function temporaryHome(): string {
  const home = mkdtempSync(join(tmpdir(), "rerail-packs-lock-"));
  homes.push(home);
  return home;
}

describe("withPacksLock", () => {
  it("runs one change at a time: a second waits until the first is done, then runs", async () => {
    const home = temporaryHome();
    const events: string[] = [];
    const firstHolds = Promise.withResolvers<void>();
    const firstMayEnd = Promise.withResolvers<void>();
    const first = withPacksLock(home, async () => {
      events.push("first starts");
      firstHolds.resolve();
      await firstMayEnd.promise;
      events.push("first ends");
    });
    await firstHolds.promise;
    // The second finds the lock held; its first wait lets the first end. No real time passes.
    const second = withPacksLock(home, async () => { events.push("second runs"); }, {
      sleep: async () => { firstMayEnd.resolve(); await first; },
    });

    await Promise.all([first, second]);

    expect(events).toEqual(["first starts", "first ends", "second runs"]);
  });

  it("gives up after the wait, naming the process that holds it, and leaves its lock alone", async () => {
    const home = temporaryHome();
    let ran = false;

    await withPacksLock(home, async () => {
      await expect(withPacksLock(home, async () => { ran = true; }, { waitMs: 0 }))
        .rejects.toThrow(`Another Rerail process (pid ${process.pid}) is changing the rule packs`);
    });
    expect(ran).toBe(false);
  });

  it("releases the lock when the change throws", async () => {
    const home = temporaryHome();

    await expect(withPacksLock(home, async () => { throw new Error("npm failed"); })).rejects.toThrow("npm failed");
    expect(await withPacksLock(home, async () => "ran", { waitMs: 0 })).toBe("ran");
  });
});

describe("pack changes racing for the lock of one killed while it held it", () => {
  // Each racer removes a file in the home 20 ms after it asks to, as a busy disk or a virus scanner can make it. A lock
  // whose takers judge a dead holder's lock and remove it lets two in when two judge it at once and the second removes
  // the lock the first has just made in its place. The delay widens that moment from microseconds to 20 ms, so such a
  // lock fails this race every run, where without the delay it failed one run in three.
  it("take it one at a time, even where removing a file takes a while", async () => {
    const home = temporaryHome();
    const size = raceSize({ processes: 8, rounds: 4 });

    const race = await raceForLock({
      take: `import fs from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import { withPacksLock } from ${JSON.stringify(sourceModule("src/setup/packs-lock.ts"))};
const home = ${JSON.stringify(home)};
for (const name of ["rmSync", "unlinkSync"]) {
  const remove = fs[name];
  fs[name] = (path, ...rest) => {
    if (String(path).startsWith(home)) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 20);
    return remove(path, ...rest);
  };
}
// The named imports of node:fs, which the lock's module uses, follow the module object from here on.
syncBuiltinESMExports();
const take = inside => withPacksLock(home, inside);`,
      ...size,
    });

    expect(race).toEqual({ overlaps: 0, ran: size.processes * size.rounds, busy: 0, errors: [] });
  }, 600_000);
});
