import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { holdHomeDaemonLock, HomeDaemonConflictError } from "../../../src/engine/native/home-daemon-lock.js";
import type * as Native from "../../../src/engine/native/native.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** What each try for the home's lock answers, in turn: held (`null`), taken (a lock), or a throw. The last repeats. */
const tries = vi.hoisted(() => ({ answers: [] as Array<"held" | "taken" | Error>, made: 0 }));

vi.mock("../../../src/engine/native/native.js", async importActual => ({
  ...await importActual<typeof Native>(),
  loadNativeBinding: () => ({
    tryLockExclusive: (): Native.NativeExclusiveFileLock | null => {
      const answer = tries.answers[Math.min(tries.made, tries.answers.length - 1)]!;
      tries.made += 1;
      if (answer instanceof Error) throw answer;
      return answer === "held" ? null : { release: () => undefined };
    },
  }),
}));

const homes: string[] = [];

afterEach(async () => {
  tries.made = 0;
  for (const home of homes.splice(0)) await removeFolder(home);
});

async function temporaryHome(): Promise<string> {
  const home = await mkdtemp(join(tmpdir(), "rerail-home-lock-wait-"));
  homes.push(home);
  return home;
}

/**
 * The wait of a daemon whose first try found its home's lock held. A try can throw while another program has the lock
 * file open (Windows), so a throw alone never ends the wait; the wait says another daemon runs only when its last try
 * found the lock held.
 */
describe("the wait for a Rerail home's daemon lock", () => {
  it("goes on past a try that throws, and takes the lock once it is free", async () => {
    tries.answers = ["held", new Error("EBUSY: the lock file is open in another program"), "taken"];
    const lease = await holdHomeDaemonLock(await temporaryHome());
    expect(lease).toBeDefined();
    lease!.release();
  });

  it("says another daemon runs when its last try found the lock held, whatever tries threw before", async () => {
    tries.answers = ["held", new Error("EBUSY: the lock file is open in another program"), "held"];
    await expect(holdHomeDaemonLock(await temporaryHome())).rejects.toBeInstanceOf(HomeDaemonConflictError);
  });

  it("ends with the error of its last try when every later try threw, never with a daemon that may not run", async () => {
    const failure = new Error("EACCES: the lock file cannot be opened");
    tries.answers = ["held", failure];
    await expect(holdHomeDaemonLock(await temporaryHome())).rejects.toBe(failure);
  });
});
