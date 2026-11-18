import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { holdHomeDaemonLock, HomeDaemonConflictError } from "../../../src/engine/native/home-daemon-lock.js";
import type * as Native from "../../../src/engine/native/native.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/**
 * How the engine answers the daemon's take of its home's lock: the lock (the OS handed it over), held at the end of the
 * wait (`null`), a failed wait (a rejection), a lock file that does not open or a first try that fails (a throw, before
 * any wait), or a wait still pending when the call returns, which the test ends through `waiting`.
 */
const engine = vi.hoisted(() => ({
  answer: "taken" as "taken" | "held" | "opens-not" | "waits" | Error,
  asked: [] as Array<{ path: string; waitMs: number }>,
  waiting: undefined as PromiseWithResolvers<{ release: () => void } | null> | undefined,
}));

vi.mock("../../../src/engine/native/native.js", async importActual => ({
  ...await importActual<typeof Native>(),
  loadNativeBinding: () => ({
    lockExclusive: (path: string, waitMs: number): Promise<Native.NativeExclusiveFileLock | null> => {
      engine.asked.push({ path, waitMs });
      const { answer } = engine;
      if (answer === "opens-not") throw new Error("EACCES: the lock file cannot be opened");
      if (answer instanceof Error) return Promise.reject(answer);
      if (answer === "waits") {
        engine.waiting = Promise.withResolvers();
        return engine.waiting.promise;
      }
      return Promise.resolve(answer === "held" ? null : { release: () => undefined });
    },
  }),
}));

const homes: string[] = [];

afterEach(async () => {
  engine.asked = [];
  engine.waiting = undefined;
  for (const home of homes.splice(0)) await removeFolder(home);
});

async function temporaryHome(): Promise<string> {
  const home = await mkdtemp(join(tmpdir(), "rerail-home-lock-wait-"));
  homes.push(home);
  return home;
}

/** The record a daemon holding `home` writes beside its lock. */
async function recordHolder(home: string, pid: number): Promise<void> {
  await mkdir(join(home, "locks"), { recursive: true });
  await writeFile(join(home, "locks", "daemon.json"), `${JSON.stringify({ pid })}\n`);
}

/**
 * The daemon's take of its home's lock is one wait on the engine, which the OS ends at the holder's release, opened once,
 * with no try after its first. A holder that answers as the running daemon is told apart at once; the bound stays for
 * the holders no answer tells apart.
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

  it("says at once that another daemon runs when the holder its record names answers as the home's daemon", async () => {
    engine.answer = "waits";
    const home = await temporaryHome();
    await recordHolder(home, 4321);
    const asked: Array<[number, number]> = [];
    const refused = holdHomeDaemonLock(home, async (pid, withinMs) => { asked.push([pid, withinMs]); return pid === 4321; });

    await expect(refused).rejects.toMatchObject({ name: "HomeDaemonConflictError", pid: 4321 });
    expect(asked).toEqual([[4321, 1_000]]);
    // The engine's wait was still pending; a lock it is handed after the refusal is let go.
    let released = false;
    engine.waiting?.resolve({ release: () => { released = true; } });
    await vi.waitFor(() => expect(released).toBe(true));
  });

  it("waits for the engine when the holder its record names does not answer as a running daemon, or none is named", async () => {
    engine.answer = "waits";
    const home = await temporaryHome();
    await recordHolder(home, 4321);
    const taking = holdHomeDaemonLock(home, async () => false);
    await vi.waitFor(() => expect(engine.waiting).toBeDefined());
    engine.waiting?.resolve({ release: () => undefined });
    const lease = await taking;
    expect(lease).toBeDefined();
    lease!.release();

    engine.answer = "waits";
    const unrecorded = await temporaryHome();
    const asked: number[] = [];
    const waited = holdHomeDaemonLock(unrecorded, async pid => { asked.push(pid); return true; });
    await vi.waitFor(() => expect(engine.waiting).toBeDefined());
    engine.waiting?.resolve(null);
    await expect(waited).rejects.toMatchObject({ name: "HomeDaemonConflictError", pid: undefined });
    expect(asked).toEqual([]);
  });

  it("asks no holder when the lock is free", async () => {
    engine.answer = "taken";
    const home = await temporaryHome();
    await recordHolder(home, 4321);
    const asked: number[] = [];
    const lease = await holdHomeDaemonLock(home, async pid => { asked.push(pid); return true; });
    expect(lease).toBeDefined();
    lease!.release();
    expect(asked).toEqual([]);
  });
});
