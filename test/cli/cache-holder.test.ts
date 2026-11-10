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
});
