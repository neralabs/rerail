import { existsSync, readdirSync, readFileSync, renameSync, type RmOptions } from "node:fs";
import { chmod, mkdir, mkdtemp, readdir, readFile, rename as fsRename, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import {
  FOLDER_REMOVAL_RETRIES,
  removeScratch,
  removeWithRetry,
  renameWithRetry,
  renameWithRetrySync,
  replaceFile,
  replaceFileSync,
} from "../../src/engine/file-replace.js";
import { REMOVAL_RETRIES, removeFolder } from "../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

async function scratch(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-file-replace-"));
  roots.push(root);
  return root;
}

const held = (code: string): Error => Object.assign(new Error(`${code}: operation not permitted, rename`), { code });

/** The error a give-up on Windows throws: it says a program held the file, and keeps the code and the last refusal. */
const heldTooLong = { code: "EPERM", message: expect.stringContaining("another program held"), cause: expect.objectContaining({ code: "EPERM" }) };

/** A rename Windows refuses the first `failures` times, as while a scan holds the file, then the real one; records each source. */
function heldThenRenamed(failures: number): { sources: string[]; rename: (from: string, to: string) => Promise<void> } {
  const sources: string[] = [];
  return {
    sources,
    rename: async (from, to) => {
      sources.push(from);
      if (sources.length <= failures) throw held("EPERM");
      await fsRename(from, to);
    },
  };
}

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
    await expect(renameWithRetry("a.tmp", "a", { platform: "win32", rename: flakyRename(1_000).rename, retryMs: 120 })).rejects.toMatchObject(heldTooLong);
    const posix = flakyRename(1);
    await expect(renameWithRetry("a.tmp", "a", { platform: "linux", rename: posix.rename })).rejects.toMatchObject({ code: "EPERM" });
    expect(posix.calls()).toBe(1);
    const missing = flakyRename(1, "ENOENT");
    await expect(renameWithRetry("a.tmp", "a", { platform: "win32", rename: missing.rename })).rejects.toMatchObject({ code: "ENOENT" });
    expect(missing.calls()).toBe(1);
  });

  it("fails at once over a folder, which no wait lets Windows replace", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-file-replace-"));
    roots.push(root);
    const refused = flakyRename(1_000);
    await expect(renameWithRetry(join(root, "new.tmp"), root, { platform: "win32", rename: refused.rename })).rejects.toMatchObject({ code: "EPERM" });
    expect(refused.calls()).toBe(1);
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

describe("renameWithRetrySync", () => {
  it("retries a held file on Windows for the synchronous writers (daemon holder, remote config, bundles)", () => {
    let calls = 0;
    const rename = (): void => {
      calls += 1;
      if (calls <= 2) throw held("EBUSY");
    };
    renameWithRetrySync("a.tmp", "a", { platform: "win32", rename });
    expect(calls).toBe(3);
    let posixCalls = 0;
    expect(() => renameWithRetrySync("a.tmp", "a", { platform: "linux", rename: () => {
      posixCalls += 1;
      throw held("EPERM");
    } })).toThrow("EPERM");
    expect(posixCalls).toBe(1);
  });
});

describe("replaceFile", () => {
  it("makes the text the whole file through a file beside it, retrying a rename Windows refuses, never removing the old file first", async () => {
    const root = await scratch();
    const target = join(root, "settings.json");
    await writeFile(target, "{\"old\":true}\n");
    const rename = heldThenRenamed(1);
    await replaceFile(target, "{\"new\":true}\n", { mode: 0o600, platform: "win32", rename: rename.rename });
    expect(await readFile(target, "utf8")).toBe("{\"new\":true}\n");
    expect(rename.sources).toHaveLength(2);
    // `<file>.<pid>.<uuid>.tmp`: the uninstall knows a launcher write cut short by that name.
    expect(rename.sources[0]?.startsWith(`${target}.${process.pid}.`)).toBe(true);
    expect(rename.sources[0]).toMatch(/\.\d+\.[0-9a-f-]+\.tmp$/u);
    expect(await readdir(root)).toEqual(["settings.json"]);
  });

  it("removes its file and leaves the old one when the rename fails", async () => {
    const root = await scratch();
    const target = join(root, "active-profile");
    await writeFile(target, "old\n");
    const refused = async (): Promise<void> => {
      throw Object.assign(new Error("EXDEV: cross-device link not permitted, rename"), { code: "EXDEV" });
    };
    await expect(replaceFile(target, "new\n", { mode: 0o600, rename: refused })).rejects.toMatchObject({ code: "EXDEV" });
    expect(await readdir(root)).toEqual(["active-profile"]);
    expect(await readFile(target, "utf8")).toBe("old\n");
  });

  it("says another program held the file once Windows refused it past the retry, and removes its file", async () => {
    const root = await scratch();
    const target = join(root, "daemon.json");
    await writeFile(target, "{}\n");
    await expect(replaceFile(target, "{\"port\":1}\n", { mode: 0o600, platform: "win32", rename: flakyRename(1_000).rename, retryMs: 60 }))
      .rejects.toMatchObject(heldTooLong);
    expect(await readdir(root)).toEqual(["daemon.json"]);
  });

  it("keeps a read-only mode, the one mode bit Windows keeps", async () => {
    const root = await scratch();
    const target = join(root, "readonly.json");
    await replaceFile(target, "{}\n", { mode: 0o444 });
    expect((await stat(target)).mode & 0o222).toBe(0);
    await chmod(target, 0o644);
  });

  it.skipIf(process.platform === "win32")("gives the file exactly its mode, which the umask does not narrow", async () => {
    const root = await scratch();
    const target = join(root, "rerail");
    const umask = process.umask(0o022);
    try {
      await replaceFile(target, "#!/bin/sh\n", { mode: 0o775 });
    } finally {
      process.umask(umask);
    }
    expect((await stat(target)).mode & 0o777).toBe(0o775);
  });
});

describe("replaceFileSync", () => {
  it("makes the text the whole file, removes its file when the rename fails, and says a program held the file", async () => {
    const root = await scratch();
    const target = join(root, "remote.json");
    let sources = 0;
    replaceFileSync(target, "{\"remotes\":[]}\n", { mode: 0o600, platform: "win32", rename: (from, to) => {
      sources += 1;
      if (sources === 1) throw held("EBUSY");
      renameSync(from, to);
    } });
    expect(readFileSync(target, "utf8")).toBe("{\"remotes\":[]}\n");
    expect(() => replaceFileSync(target, "x", { mode: 0o600, rename: () => {
      throw Object.assign(new Error("EXDEV: cross-device link not permitted, rename"), { code: "EXDEV" });
    } })).toThrow(expect.objectContaining({ code: "EXDEV" }));
    expect(() => replaceFileSync(target, "x", { mode: 0o600, platform: "win32", retryMs: 60, rename: () => {
      throw held("EPERM");
    } })).toThrow(expect.objectContaining(heldTooLong));
    expect(readdirSync(root)).toEqual(["remote.json"]);
    expect(readFileSync(target, "utf8")).toBe("{\"remotes\":[]}\n");
  });
});

/** A file Windows reports held: the code a refused `rm` gives, and `path` naming the file, as Node's errors do. */
const heldFile = (code: string, path: string): Error => Object.assign(new Error(`${code}: resource busy or locked, unlink '${path}'`), { code, path });

/** An `rm` refused with `code` on the file `held` the first `failures` times, then removing nothing; records each try's options. */
function heldRemoval(failures: number, code = "EBUSY", held = join("versions", "nested", "held.sqlite")) {
  const tries: RmOptions[] = [];
  const rm = async (_path: string, options: RmOptions): Promise<void> => {
    tries.push(options);
    if (tries.length <= failures) throw heldFile(code, held);
  };
  return { tries, rm };
}

/** A wait that returns at once, recording how long it was asked to wait. */
function recordedWaits(): { waits: number[]; sleep: (ms: number) => Promise<void> } {
  const waits: number[] = [];
  return { waits, sleep: async (ms) => {
    waits.push(ms);
  } };
}

/** One try with no retries of `rm`'s own, which would repeat the whole set at every level of the folder. */
const ONE_TRY = { recursive: true, force: true };

describe("removeWithRetry", () => {
  it("tries again while a file in the folder is held, each try one rm of its own, waiting 100 ms longer each time", async () => {
    const removal = heldRemoval(3);
    const { waits, sleep } = recordedWaits();
    await removeWithRetry("versions", { platform: "win32", rm: removal.rm, sleep });
    expect(removal.tries).toEqual([ONE_TRY, ONE_TRY, ONE_TRY, ONE_TRY]);
    expect(waits).toEqual([100, 200, 300]);
  });

  it("gives up after its 10 retries and 5.5 s of waits, naming the file another program held open", async () => {
    const held = join("versions", "0.2.0", "nested", "held.sqlite");
    const removal = heldRemoval(Infinity, "EBUSY", held);
    const { waits, sleep } = recordedWaits();
    await expect(removeWithRetry("versions", { platform: "win32", rm: removal.rm, sleep })).rejects.toMatchObject({
      code: "EBUSY",
      message: `EBUSY: Windows refused for 5.5 s to remove versions, as another program held ${held} open`,
      cause: expect.objectContaining({ code: "EBUSY", path: held }),
    });
    expect(removal.tries).toEqual(Array.from({ length: FOLDER_REMOVAL_RETRIES.maxRetries + 1 }, () => ONE_TRY));
    expect(waits).toEqual([100, 200, 300, 400, 500, 600, 700, 800, 900, 1_000]);
    const unnamed = async (): Promise<void> => {
      throw Object.assign(new Error("EPERM: operation not permitted, rmdir"), { code: "EPERM" });
    };
    await expect(removeWithRetry("versions", { platform: "win32", rm: unnamed, sleep }))
      .rejects.toThrow("EPERM: Windows refused for 5.5 s to remove versions, as another program held it, or a file in it, open");
  });

  it("retries elsewhere as rm did, the last refusal standing, and fails at once on an error no wait clears", async () => {
    const posix = heldRemoval(Infinity, "ENOTEMPTY");
    await expect(removeWithRetry("versions", { platform: "linux", rm: posix.rm, sleep: recordedWaits().sleep }))
      .rejects.toMatchObject({ code: "ENOTEMPTY", message: expect.stringContaining("resource busy or locked") });
    expect(posix.tries).toHaveLength(FOLDER_REMOVAL_RETRIES.maxRetries + 1);
    const denied = heldRemoval(Infinity, "EACCES");
    await expect(removeWithRetry("versions", { platform: "win32", rm: denied.rm, sleep: recordedWaits().sleep }))
      .rejects.toMatchObject({ code: "EACCES", message: expect.stringContaining("resource busy or locked") });
    expect(denied.tries).toHaveLength(1);
  });

  describe.runIf(process.platform === "win32")("against a file this process holds open", () => {
    /** The folder `versions` in a new scratch folder, with a SQLite file open one folder down, which Windows refuses to delete. */
    async function heldFolder(): Promise<{ folder: string; held: string; database: Database.Database }> {
      const folder = join(await scratch(), "versions");
      const held = join(folder, "nested", "held.sqlite");
      await mkdir(dirname(held), { recursive: true });
      const database = new Database(held);
      database.exec("CREATE TABLE t (x)");
      return { folder, held, database };
    }

    it("gives up within 6.5 s, the 5.5 s of waits and a second for the tries, naming the file", async () => {
      const { folder, held, database } = await heldFolder();
      const limit = 6_500;
      const started = Date.now();
      try {
        const outcome = await Promise.race([
          removeWithRetry(folder).then(() => "removed", (error: unknown) => error),
          delay(limit, `still trying after ${limit} ms`),
        ]);
        expect(outcome).toBeInstanceOf(Error);
        expect(String(outcome)).toContain(`another program held ${held} open`);
      } finally {
        database.close();
      }
      expect(Date.now() - started).toBeGreaterThanOrEqual(5_500);
    });

    it("removes the folder once the file is let go of a second later", async () => {
      const { folder, database } = await heldFolder();
      const released = delay(1_000).then(() => database.close());
      try {
        await removeWithRetry(folder);
      } finally {
        await released;
      }
      expect(existsSync(folder)).toBe(false);
    });
  });
});

describe("removeScratch", () => {
  it("never fails the work that used the folder", async () => {
    const removal = heldRemoval(Infinity);
    await expect(removeScratch(join(tmpdir(), "rerail-missing"), { rm: removal.rm, sleep: recordedWaits().sleep })).resolves.toBeUndefined();
    expect(removal.tries).toHaveLength(FOLDER_REMOVAL_RETRIES.maxRetries + 1);
  });
});

describe("FOLDER_REMOVAL_RETRIES", () => {
  it("is what the tests' removeFolder retries with, which may load no Rerail module and so repeats it", () => {
    expect(REMOVAL_RETRIES).toEqual(FOLDER_REMOVAL_RETRIES);
  });
});
