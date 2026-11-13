import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { cacheHolder } from "../../../src/cli/setup-cli.js";
import { fakeMachine } from "../../support/platform/daemon-machine.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/** A Windows machine where every pid is a live process, with `pipes`. */
function windows(pipes: readonly string[]) {
  return fakeMachine("win32", { kill: () => true, pipes: () => pipes });
}

describe("the uninstall's cache holder", () => {
  it("on Windows counts a recorded pid only while a Rerail engine pipe exists, since Windows reuses a dead daemon's pid", async () => {
    const cache = await mkdtemp(join(tmpdir(), "rerail-cache-holder-"));
    roots.push(cache);
    // A daemon Windows ended at logoff left its record; the pid now belongs to an unrelated live process.
    const recorded = process.pid + 1;
    await writeFile(join(cache, "main-instance.json"), JSON.stringify({ pid: recorded }));
    expect(cacheHolder(cache, windows([]))).toBeUndefined();
    expect(cacheHolder(cache, windows(["rerail-0123456789abcdef01234567"]))).toBe(recorded);
    expect(cacheHolder(cache, { ...windows([]), platform: "linux" })).toBe(recorded);
  });

  // Asking the lock loads the engine from the running version's folder, and Windows keeps a loaded DLL from being
  // deleted: an uninstall that asked could then never remove `versions/`. So the pid rule answers first, and on Windows
  // it alone answers.
  it("on Windows never asks the main-instance lock, so the uninstall never loads the engine it is about to delete", async () => {
    const cache = await mkdtemp(join(tmpdir(), "rerail-cache-holder-"));
    roots.push(cache);
    const recorded = process.pid + 1;
    await writeFile(join(cache, "main-instance.json"), JSON.stringify({ pid: recorded }));
    const asked: string[] = [];
    const lockHeld = (directory: string) => {
      asked.push(directory);
      return false;
    };
    // A stale record: the daemon Windows ended at logoff is gone and no other Rerail daemon runs.
    expect(cacheHolder(cache, fakeMachine("win32", { mainInstanceLockHeld: lockHeld }))).toBeUndefined();
    // Another Rerail home's daemon runs, so a Rerail engine pipe exists, and the recorded pid is a live process.
    expect(cacheHolder(cache, { ...windows(["rerail-0123456789abcdef01234567"]), mainInstanceLockHeld: lockHeld })).toBe(recorded);
    expect(asked).toEqual([]);
  });

  // Elsewhere a loaded engine does not keep its file, so the lock, which the OS drops however the holder ends, refines
  // a live pid that may be another process by now.
  it("elsewhere asks the main-instance lock only for a live recorded pid, and lets the lock decide", async () => {
    const cache = await mkdtemp(join(tmpdir(), "rerail-cache-holder-"));
    roots.push(cache);
    const recorded = process.pid + 1;
    await writeFile(join(cache, "main-instance.json"), JSON.stringify({ pid: recorded }));
    const asked: string[] = [];
    const linux = (alive: boolean, held: boolean | undefined) => fakeMachine("linux", {
      ...(alive ? { kill: () => true } : {}),
      mainInstanceLockHeld: directory => {
        asked.push(directory);
        return held;
      },
    });
    expect(cacheHolder(cache, linux(false, true))).toBeUndefined();
    expect(asked).toEqual([]);
    expect(cacheHolder(cache, linux(true, false))).toBeUndefined();
    expect(cacheHolder(cache, linux(true, true))).toBe(recorded);
    // Where the native binding does not load, the live pid counts.
    expect(cacheHolder(cache, linux(true, undefined))).toBe(recorded);
    expect(asked).toEqual([cache, cache, cache]);
  });
});
