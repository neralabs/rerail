import { chmod, mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { removeScratch, renameWithRetry } from "../../src/engine/file-replace.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

const held = (code: string): Error => Object.assign(new Error(`${code}: operation not permitted, rename`), { code });

/** A rename that fails with `code` the first `failures` times, then succeeds; counts its calls. */
function flakyRename(failures: number, code = "EPERM"): { calls: () => number; rename: (from: string, to: string) => Promise<void> } {
  let calls = 0;
  return {
    calls: () => calls,
    rename: async () => {
      calls += 1;
      if (calls <= failures) throw held(code);
    },
  };
}

describe("renameWithRetry", () => {
  it("retries a rename Windows refuses while another process holds the file", async () => {
    for (const code of ["EPERM", "EACCES", "EBUSY"]) {
      const flaky = flakyRename(2, code);
      await renameWithRetry("a.tmp", "a", { platform: "win32", rename: flaky.rename });
      expect(flaky.calls()).toBe(3);
    }
  });

  it("gives up once the retry time is spent, and never retries elsewhere or on another error", async () => {
    await expect(renameWithRetry("a.tmp", "a", { platform: "win32", rename: flakyRename(1_000).rename, retryMs: 120 })).rejects.toMatchObject({ code: "EPERM" });
    const posix = flakyRename(1);
    await expect(renameWithRetry("a.tmp", "a", { platform: "linux", rename: posix.rename })).rejects.toMatchObject({ code: "EPERM" });
    expect(posix.calls()).toBe(1);
    const missing = flakyRename(1, "ENOENT");
    await expect(renameWithRetry("a.tmp", "a", { platform: "win32", rename: missing.rename })).rejects.toMatchObject({ code: "ENOENT" });
    expect(missing.calls()).toBe(1);
  });

  it("makes a read-only file writable on Windows so it can be replaced", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-file-replace-"));
    roots.push(root);
    const target = join(root, "settings.json");
    await writeFile(target, "{}\n");
    await chmod(target, 0o444);
    await renameWithRetry(join(root, "new.tmp"), target, { platform: "win32", rename: flakyRename(1).rename });
    expect((await stat(target)).mode & 0o200).not.toBe(0);
  });
});

describe("removeScratch", () => {
  it("never fails the work that used the folder", async () => {
    const rm = async (): Promise<void> => {
      throw Object.assign(new Error("EBUSY: resource busy or locked, rmdir"), { code: "EBUSY" });
    };
    await expect(removeScratch(join(tmpdir(), "rerail-missing"), rm)).resolves.toBeUndefined();
  });
});
