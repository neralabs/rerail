import { mkdtemp, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { leaveUntaken, UNTAKEN_EVENTS_DIR, untakenEventTaker, untakenFor, type UntakenEvent } from "../../../src/runtime/untaken-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const homes: string[] = [];
async function home(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-untaken-"));
  homes.push(path);
  return path;
}

afterEach(async () => {
  await Promise.all(homes.splice(0).map(path => removeFolder(path)));
});

describe("the home's untaken events", () => {
  it("are taken in the order their hooks met them, each removed once recorded, and name the daemon they were left for", async () => {
    const rerailHome = await home();
    await leaveUntaken(rerailHome, 41, "third", 3_000);
    await leaveUntaken(rerailHome, 41, "first", 1_000);
    await leaveUntaken(rerailHome, 42, "second", 2_000);
    expect(untakenFor(rerailHome, 41)).toBe(true);
    expect(untakenFor(rerailHome, 43)).toBe(false);
    const recorded: UntakenEvent[] = [];
    untakenEventTaker(rerailHome, event => recorded.push(event)).take();
    expect(recorded).toEqual([
      { body: "first", arrivedAt: 1_000 },
      { body: "second", arrivedAt: 2_000 },
      { body: "third", arrivedAt: 3_000 },
    ]);
    expect(await readdir(join(rerailHome, UNTAKEN_EVENTS_DIR))).toEqual([]);
    expect(untakenFor(rerailHome, 41)).toBe(false);
  });

  it("are taken as they are left once the daemon watches its home, with no request and no timer", async () => {
    const rerailHome = await home();
    const taken = Promise.withResolvers<UntakenEvent>();
    const taker = untakenEventTaker(rerailHome, event => taken.resolve(event));
    try {
      expect(taker.watch()).toBe(true);
      await leaveUntaken(rerailHome, 7, "left later", 5_000);
      expect(await taken.promise).toEqual({ body: "left later", arrivedAt: 5_000 });
    } finally {
      taker.close();
    }
  });

  it("orders two events one hook left in the same millisecond as it left them", async () => {
    const rerailHome = await home();
    for (let count = 0; count < 11; count += 1) await leaveUntaken(rerailHome, 7, `event ${count}`, 9_000);
    const recorded: string[] = [];
    untakenEventTaker(rerailHome, event => recorded.push(event.body)).take();
    expect(recorded).toEqual(Array.from({ length: 11 }, (_, count) => `event ${count}`));
  });
});
