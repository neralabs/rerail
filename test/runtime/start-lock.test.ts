import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { withStartLock } from "../../src/runtime/daemon-control.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { raceForLock, raceSize, sourceModule } from "../support/processes/lock-race.js";

const homes: string[] = [];
afterEach(async () => {
  await Promise.all(homes.splice(0).map(home => removeFolder(home)));
});

async function temporaryHome(): Promise<string> {
  const home = await mkdtemp(join(tmpdir(), "rerail-start-lock-"));
  homes.push(home);
  return home;
}

describe("a daemon start taking the start lock", () => {
  it("waits while another start holds it, then runs", async () => {
    const home = await temporaryHome();
    const events: string[] = [];
    const otherHolds = Promise.withResolvers<void>();
    const otherMayEnd = Promise.withResolvers<void>();
    const other = withStartLock(home, async () => {
      events.push("the other start runs");
      otherHolds.resolve();
      await otherMayEnd.promise;
      events.push("the other start ends");
    });
    await otherHolds.promise;
    // This start finds the lock held; its first wait lets the other end. No real time passes.
    const start = withStartLock(home, async () => { events.push("this start runs"); }, {
      sleep: async () => { otherMayEnd.resolve(); await other; },
    });

    await Promise.all([other, start]);
    expect(events).toEqual(["the other start runs", "the other start ends", "this start runs"]);
  });
});

describe("starts racing for the lock of a start killed while it held it", () => {
  it("take it one at a time", async () => {
    const home = await temporaryHome();
    const size = raceSize({ processes: 8, rounds: 4 });

    const race = await raceForLock({
      take: `import { withStartLock } from ${JSON.stringify(sourceModule("src/runtime/daemon-control.ts"))};
const take = inside => withStartLock(${JSON.stringify(home)}, inside);`,
      ...size,
    });

    expect(race).toEqual({ overlaps: 0, ran: size.processes * size.rounds, busy: 0, errors: [] });
  }, 600_000);
});
