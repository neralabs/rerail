import { existsSync, readdirSync, readFileSync } from "node:fs";
import { mkdir, mkdtemp, rm, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LOCKS_DIR, withHomeLock, type LockHolder, type LockRequest } from "../../../src/engine/native/file-lock.js";
import { raceForLock, raceSize, sourceModule } from "../../support/processes/lock-race.js";

vi.mock("node:fs/promises", { spy: true });
const { unlink: realUnlink } = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");

const homes: string[] = [];
afterEach(async () => {
  vi.mocked(unlink).mockReset();
  await Promise.all(homes.splice(0).map(home => rm(home, { recursive: true, force: true })));
});

async function temporaryHome(): Promise<string> {
  const home = await mkdtemp(join(tmpdir(), "rerail-file-lock-"));
  homes.push(home);
  return home;
}

class Busy extends Error {
  constructor(readonly holder: LockHolder | undefined) {
    super(`busy (${holder === undefined ? "unknown" : `pid ${holder.pid}`})`);
  }
}

const request = { name: "work", busy: (holder: LockHolder | undefined) => new Busy(holder) } satisfies LockRequest;
/** The engine as a Rerail whose engine does not load has it. */
const withoutEngine = {
  ...request,
  engine: () => { throw new Error("Unsupported platform or missing native addon"); },
  withoutEngine: "folder",
} satisfies LockRequest;

const folderOf = (home: string): string => join(home, LOCKS_DIR, "work-without-engine");

/** A pid no process has: above Linux's default limit, and odd, which no Windows pid is. */
const DEAD_PID = 2 ** 22 - 1;

/** The folder lock as a holder with `pid` leaves it. */
async function folderHeldBy(home: string, pid: number): Promise<string> {
  const folder = folderOf(home);
  await mkdir(folder, { recursive: true });
  const owner = join(folder, "owner-00000000-0000-4000-8000-000000000000.json");
  await writeFile(owner, JSON.stringify({ pid, since: "2026-11-10T12:00:00.000Z" }));
  return owner;
}

describe.each<[string, LockRequest]>([
  ["the OS lock", request],
  ["the folder lock, without the engine", withoutEngine],
])("%s", (_lock, lock) => {
  it("refuses a second holder in this process while the first holds it, and names the holder", async () => {
    const home = await temporaryHome();

    const refused = await withHomeLock(home, lock, async () => await withHomeLock(home, lock, async () => "ran").catch((error: unknown) => error));

    expect(refused).toBeInstanceOf(Busy);
    expect((refused as Busy).holder).toMatchObject({ pid: process.pid });
  });

  it("waits while another holds it, then takes it", async () => {
    const home = await temporaryHome();
    const events: string[] = [];
    const firstHolds = Promise.withResolvers<void>();
    const firstMayEnd = Promise.withResolvers<void>();
    const first = withHomeLock(home, lock, async () => {
      events.push("first runs");
      firstHolds.resolve();
      await firstMayEnd.promise;
      events.push("first ends");
    });
    await firstHolds.promise;
    // The second finds the lock held; its first wait lets the first end. No real time passes.
    const second = withHomeLock(home, { ...lock, waitMs: 60_000, sleep: async () => { firstMayEnd.resolve(); await first; } }, async () => {
      events.push("second runs");
    });

    await Promise.all([first, second]);
    expect(events).toEqual(["first runs", "first ends", "second runs"]);
  });

  it("lets go when the work throws", async () => {
    const home = await temporaryHome();

    await expect(withHomeLock(home, lock, async () => { throw new Error("npm failed"); })).rejects.toThrow("npm failed");
    await expect(withHomeLock(home, lock, async () => "ran")).resolves.toBe("ran");
  });

  it("is free at once for processes racing for it after its holder was killed", async () => {
    const home = await temporaryHome();
    const size = raceSize({ processes: 6, rounds: 3 });

    const race = await raceForLock({
      take: `import { withHomeLock } from ${JSON.stringify(sourceModule("src/engine/native/file-lock.ts"))};
const lock = ${JSON.stringify({ name: "work", withoutEngine: lock.withoutEngine })};
${lock.withoutEngine === "folder" ? `lock.engine = () => { throw new Error("no engine"); };` : ""}
const take = inside => withHomeLock(${JSON.stringify(home)}, { ...lock, waitMs: 60_000, busy: () => new Error("busy") }, inside);`,
      ...size,
    });

    expect(race).toEqual({ overlaps: 0, ran: size.processes * size.rounds, busy: 0, errors: [] });
  }, 600_000);
});

describe("the OS lock", () => {
  it("records its holder while it holds the lock, and keeps the lock file once it lets go", async () => {
    const home = await temporaryHome();
    const locks = join(home, LOCKS_DIR);

    const recorded = await withHomeLock(home, request, async () => JSON.parse(readFileSync(join(locks, "work.json"), "utf8")) as unknown);

    expect(recorded).toEqual({ pid: process.pid, since: expect.any(String) });
    expect(readdirSync(locks)).toEqual(["work.lock"]);
  });

  it("fails without the engine unless the request takes the folder lock", async () => {
    const home = await temporaryHome();

    await expect(withHomeLock(home, { ...withoutEngine, withoutEngine: "fail" }, async () => "ran")).rejects.toThrow("missing native addon");
  });
});

describe("the folder lock, without the engine", () => {
  it("leaves nothing behind once it lets go", async () => {
    const home = await temporaryHome();

    await withHomeLock(home, withoutEngine, async () => {
      expect(readdirSync(folderOf(home))).toEqual([expect.stringMatching(/^owner-[0-9a-f-]+\.json$/u)]);
    });

    expect(readdirSync(join(home, LOCKS_DIR))).toEqual([]);
  });

  it("takes over the lock of a holder whose process is gone", async () => {
    const home = await temporaryHome();
    const owner = await folderHeldBy(home, DEAD_PID);

    await expect(withHomeLock(home, withoutEngine, async () => existsSync(owner))).resolves.toBe(false);
  });

  it("takes a folder a holder left empty, letting go halfway", async () => {
    const home = await temporaryHome();
    await mkdir(folderOf(home), { recursive: true });

    await expect(withHomeLock(home, withoutEngine, async () => "ran")).resolves.toBe("ran");
  });

  it("leaves alone the lock of a running holder", async () => {
    const home = await temporaryHome();
    const owner = await folderHeldBy(home, process.pid);

    await expect(withHomeLock(home, withoutEngine, async () => "ran")).rejects.toBeInstanceOf(Busy);
    expect(existsSync(owner)).toBe(true);
  });

  it("never removes the lock of a taker that cleared the same dead holder first", async () => {
    const home = await temporaryHome();
    const owner = await folderHeldBy(home, DEAD_PID);
    const othersLetGo = Promise.withResolvers<void>();
    let others: Promise<void> | undefined;
    // Between finding the holder dead and removing its owner file, another taker clears it, takes the lock and holds it.
    vi.mocked(unlink).mockImplementationOnce(async path => {
      expect(path).toBe(owner);
      const holding = Promise.withResolvers<void>();
      others = withHomeLock(home, withoutEngine, async () => { holding.resolve(); await othersLetGo.promise; });
      await holding.promise;
      await realUnlink(path);
    });

    await expect(withHomeLock(home, withoutEngine, async () => "ran")).rejects.toBeInstanceOf(Busy);
    expect(readdirSync(folderOf(home))).toEqual([expect.stringMatching(/^owner-[0-9a-f-]+\.json$/u)]);
    othersLetGo.resolve();
    await others;
  });
});
