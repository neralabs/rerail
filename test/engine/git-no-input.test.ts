import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, expect, it, vi } from "vitest";

/**
 * The code of each error on the stdin of a Git that Rerail started. Each start holds the thread for 300 ms after it, as
 * a busy daemon thread is held, so a Git that exits at once has exited before the read reaches its stdin.
 */
const stdin = vi.hoisted(() => ({ errors: [] as string[], held: new Int32Array(new SharedArrayBuffer(4)) }));
vi.mock("node:child_process", async importOriginal => {
  const original = await importOriginal<typeof import("node:child_process")>();
  const spawn = ((...args: Parameters<typeof original.spawn>) => {
    const child = original.spawn(...args);
    Atomics.wait(stdin.held, 0, 0, 300);
    child.stdin?.prependListener("error", (error: NodeJS.ErrnoException) => stdin.errors.push(error.code ?? error.name));
    return child;
  }) as typeof original.spawn;
  return { ...original, spawn };
});

import { runGitAsync } from "../../src/engine/git.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const folders: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
});

// Under vitest the first such error's stack made the runner decode git.ts's source map and keep it: a reopened
// session's heap figure read a megabyte more.
it("writes nothing to the stdin of a Git given no input, so one that already exited makes no error there", async () => {
  const outside = await realpath(await mkdtemp(join(tmpdir(), "rerail-no-input-")));
  folders.push(outside);
  vi.stubEnv("GIT_CEILING_DIRECTORIES", dirname(outside));
  const result = await runGitAsync(outside, ["rev-parse", "--verify", "--quiet", "HEAD"]);
  expect(result.status).toBe(128);
  expect(stdin.errors).toEqual([]);
});
