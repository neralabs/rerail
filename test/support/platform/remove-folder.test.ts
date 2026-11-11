import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { REMOVAL_RETRIES, removeFolder, removeFolderSync, runningIn } from "./remove-folder.js";

const folders: string[] = [];
const children: ChildProcess[] = [];
afterEach(async () => {
  for (const child of children.splice(0)) {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill();
      await once(child, "exit");
    }
  }
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
});

async function folder(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-remove-folder-"));
  folders.push(path);
  await writeFile(join(path, "a.txt"), "a\n");
  return path;
}

/** A Node process this test starts in `cwd`, once it runs. */
async function runningProcess(cwd: string): Promise<ChildProcess> {
  const child = spawn(process.execPath, ["-e", "process.stdout.write('ready'); setInterval(() => {}, 1000);"], {
    cwd, stdio: ["ignore", "pipe", "ignore"], windowsHide: true,
  });
  children.push(child);
  await once(child.stdout, "data");
  return child;
}

describe("removeFolder", () => {
  it("refuses a folder a process this test started still runs in, naming it, and removes it at once after it exits", async () => {
    const path = await folder();
    const child = await runningProcess(path);
    if (process.platform === "win32") {
      // The one try every test made before: Windows keeps the folder while the process runs in it.
      await expect(rm(path, { recursive: true, force: true })).rejects.toMatchObject({ code: "EBUSY" });
    }

    expect(runningIn(path)).toEqual([expect.stringContaining(`pid ${child.pid}: `)]);
    await expect(removeFolder(path)).rejects.toThrow(`pid ${child.pid}: `);
    expect(() => removeFolderSync(path)).toThrow(`pid ${child.pid}: `);
    child.kill();
    await once(child, "exit");

    expect(runningIn(path)).toEqual([]);
    await removeFolder(path, { retry: false });
    expect(existsSync(path)).toBe(false);
  });

  it("removes a link to a folder a process runs in, and leaves the folder and the process alone", async () => {
    const target = await folder();
    const child = await runningProcess(target);
    const link = join(await folder(), "link");
    // A junction on Windows, which needs no administrator rights; a symbolic link elsewhere, which ignores the type.
    await symlink(target, link, "junction");

    await removeFolder(link, { retry: false });
    expect(existsSync(link)).toBe(false);
    expect(existsSync(join(target, "a.txt"))).toBe(true);
    expect(runningIn(target)).toEqual([expect.stringContaining(`pid ${child.pid}: `)]);
  });

  it("waits out a holder this test did not start, which lets go a moment later", async () => {
    const path = await folder();
    // SQLite opens its file so that Windows refuses to delete it; the connection closes 300 ms on, as a late release.
    const database = new Database(join(path, "held.sqlite"));
    database.exec("CREATE TABLE t (x)");
    const closing = setTimeout(() => database.close(), 300);
    try {
      if (process.platform === "win32") await expect(removeFolder(path, { retry: false })).rejects.toMatchObject({ code: "EBUSY" });
      await removeFolder(path);
      expect(existsSync(path)).toBe(false);
    } finally {
      clearTimeout(closing);
      if (database.open) database.close();
    }
  });

  it.runIf(process.platform === "win32")("gives up on a holder that never lets go within its retries, naming the file held", async () => {
    const path = await folder();
    const held = join(path, "nested", "held.sqlite");
    await mkdir(dirname(held));
    // This test process holds the file open throughout, so that Windows refuses to delete it.
    const database = new Database(held);
    database.exec("CREATE TABLE t (x)");
    const { maxRetries, retryDelay } = REMOVAL_RETRIES;
    // The waits between tries, 100 ms and then each 100 ms longer than the last (5.5 s), and a second for the tries.
    const limit = retryDelay * maxRetries * (maxRetries + 1) / 2 + 1_000;
    try {
      const outcome = await Promise.race([
        removeFolder(path).then(() => "removed", (error: unknown) => error),
        delay(limit, `still trying after ${limit} ms`),
      ]);
      expect(outcome).toBeInstanceOf(Error);
      expect(String(outcome)).toContain(held);
      expect(String(outcome)).toContain("no process this test started");
    } finally {
      database.close();
    }
  });
});
