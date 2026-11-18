import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { holdHomeDaemonLock, HomeDaemonConflictError } from "../../../src/engine/native/home-daemon-lock.js";
import type * as Native from "../../../src/engine/native/native.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/**
 * How the engine answers the daemon's take of its home's lock: the lock (the OS handed it over), held at the end of the
 * wait (`null`), a failed wait (a rejection), or a lock file that does not open or a first try that fails (a throw,
 * before any wait).
 */
const engine = vi.hoisted(() => ({
  answer: "taken" as "taken" | "held" | "opens-not" | Error,
  asked: [] as Array<{ path: string; waitMs: number }>,
}));

vi.mock("../../../src/engine/native/native.js", async importActual => ({
  ...await importActual<typeof Native>(),
  loadNativeBinding: () => ({
    lockExclusive: (path: string, waitMs: number): Promise<Native.NativeExclusiveFileLock | null> => {
      engine.asked.push({ path, waitMs });
      const { answer } = engine;
      if (answer === "opens-not") throw new Error("EACCES: the lock file cannot be opened");
      if (answer instanceof Error) return Promise.reject(answer);
      return Promise.resolve(answer === "held" ? null : { release: () => undefined });
    },
  }),
}));

const homes: string[] = [];

afterEach(async () => {
  engine.asked = [];
  for (const home of homes.splice(0)) await removeFolder(home);
});

async function temporaryHome(): Promise<string> {
  const home = await mkdtemp(join(tmpdir(), "rerail-home-lock-wait-"));
  homes.push(home);
  return home;
}

/**
 * The daemon's take of its home's lock is one wait on the engine, which the OS ends at the holder's release: a daemon
 * restarting itself hands the lock to the next as it exits. Its bound tells a running daemon from a holder about to let
 * go, so the wait is one call, opened once, with no try after its first.
 */
describe("the wait for a Rerail home's daemon lock", () => {
  it("takes the lock in one wait on the engine, bounded at a second", async () => {
    engine.answer = "taken";
    const home = await temporaryHome();
    const lease = await holdHomeDaemonLock(home);
    expect(lease).toBeDefined();
    lease!.release();
    expect(engine.asked).toEqual([{ path: join(home, "locks", "daemon.lock"), waitMs: 1_000 }]);
  });

  it("says another daemon runs when the lock is still held at the end of the wait", async () => {
    engine.answer = "held";
    await expect(holdHomeDaemonLock(await temporaryHome())).rejects.toBeInstanceOf(HomeDaemonConflictError);
  });

  it("ends with the wait's own error, never with a daemon that may not run", async () => {
    const failure = new Error("EIO: the lock could not be taken");
    engine.answer = failure;
    await expect(holdHomeDaemonLock(await temporaryHome())).rejects.toBe(failure);
  });

  it("runs without the lock where its file does not open or its first try fails, as where the engine does not load", async () => {
    engine.answer = "opens-not";
    await expect(holdHomeDaemonLock(await temporaryHome())).resolves.toBeUndefined();
  });
});
