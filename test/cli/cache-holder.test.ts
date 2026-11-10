import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { cacheHolder } from "../../src/cli/setup-cli.js";
import { fakeMachine } from "../support/platform/daemon-machine.js";
import { removeFolder } from "../support/platform/remove-folder.js";

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

  // The holder keeps `main-instance.lock` for its whole life and the OS drops it however the holder ends, so the lock
  // decides; the pid rule above is for a machine where the native binding does not load.
  it("counts the recorded pid only while a process holds the cache's main-instance lock, whatever engine pipes exist", async () => {
    const cache = await mkdtemp(join(tmpdir(), "rerail-cache-holder-"));
    roots.push(cache);
    const recorded = process.pid + 1;
    await writeFile(join(cache, "main-instance.json"), JSON.stringify({ pid: recorded }));
    // Another Rerail home's daemon runs, so a Rerail engine pipe exists, and the recorded pid is a live process.
    const machine = (held: boolean) => ({ ...windows(["rerail-0123456789abcdef01234567"]), mainInstanceLockHeld: () => held });
    expect(cacheHolder(cache, machine(false))).toBeUndefined();
    expect(cacheHolder(cache, machine(true))).toBe(recorded);
    expect(cacheHolder(cache, { ...machine(false), platform: "linux" })).toBeUndefined();
  });
});
