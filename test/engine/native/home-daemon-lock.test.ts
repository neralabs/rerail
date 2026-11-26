import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as FileReplace from "../../../src/engine/file-replace.js";
import {
  holdHomeDaemonLock,
  HomeDaemonConflictError,
  homeDaemonHolder,
  homeDaemonLockHeld,
  homeDaemonStarting,
  type HomeDaemonLease,
} from "../../../src/engine/native/home-daemon-lock.js";
import { LOCKS_DIR } from "../../../src/engine/native/file-lock.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { DAEMON_LOG_FILE } from "../../../src/engine/storage/rerail-home.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const packageRoot = resolve(import.meta.dirname, "..", "..", "..");
const lockModule = pathToFileURL(join(packageRoot, "src", "engine", "native", "home-daemon-lock.ts")).href;

/** How many of this process's next whole-file writes fail, as one does while another program has the file open. */
const writes = vi.hoisted(() => ({ failing: 0 }));

vi.mock("../../../src/engine/file-replace.js", async importActual => {
  const real = await importActual<typeof FileReplace>();
  return {
    ...real,
    replaceFileSync: (...args: Parameters<typeof real.replaceFileSync>): void => {
      if (writes.failing > 0) {
        writes.failing -= 1;
        throw Object.assign(new Error("EPERM: operation not permitted, rename"), { code: "EPERM" });
      }
      real.replaceFileSync(...args);
    },
  };
});

/** Whether the file system gives no id for a daemon's lock file, as a file system with no file ids gives none. */
const fileIds = vi.hoisted(() => ({ hidden: false }));

vi.mock("node:fs", async importActual => {
  const real = await importActual<typeof import("node:fs")>();
  // The cast keeps statSync's overloads: the wrapper passes every argument on. Node gives such a file the id 0.
  const statSync = ((...args: Parameters<typeof real.statSync>) => {
    const found = real.statSync(...args);
    return fileIds.hidden && String(args[0]).endsWith("daemon.lock") ? Object.assign(found!, { ino: 0n }) : found;
  }) as typeof real.statSync;
  return { ...real, statSync };
});

const children: ChildProcess[] = [];
const leases: HomeDaemonLease[] = [];
const homes: string[] = [];

afterEach(async () => {
  writes.failing = 0;
  fileIds.hidden = false;
  vi.restoreAllMocks();
  for (const lease of leases.splice(0)) lease.release();
  for (const child of children.splice(0)) {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      child.kill("SIGKILL");
      await exited;
    }
  }
  for (const home of homes.splice(0)) await removeFolder(home);
});

async function temporaryHome(): Promise<string> {
  const home = await mkdtemp(join(tmpdir(), "rerail-home-daemon-"));
  homes.push(home);
  return home;
}

/** A separate process that becomes the daemon of `home` and stays so until it is killed. */
async function holdInAnotherProcess(home: string): Promise<ChildProcess> {
  const child = spawn(process.execPath, [
    "--import", "tsx", "--input-type=module", "-e",
    `import { holdHomeDaemonLock } from ${JSON.stringify(lockModule)};
     await holdHomeDaemonLock(${JSON.stringify(home)});
     process.stdout.write("held\\n");
     process.stdin.resume();`,
  ], { cwd: packageRoot, stdio: ["pipe", "pipe", "pipe"] });
  children.push(child);
  let stderr = "";
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
  const outcome = await Promise.race([
    once(child.stdout, "data").then(([chunk]) => String(chunk)),
    once(child, "exit").then(() => `lock holder exited early: ${stderr}`),
  ]);
  expect(outcome).toBe("held\n");
  return child;
}

describe("the daemon lock of a Rerail home", () => {
  it("keeps a home to one daemon: a second meets a refusal naming the first, and the home is free once the first ends", async () => {
    const home = await temporaryHome();
    const first = await holdInAnotherProcess(home);
    expect(homeDaemonLockHeld(home)).toBe(true);
    expect(homeDaemonHolder(home)).toBe(first.pid);
    const refusal = await holdHomeDaemonLock(home).then(lease => {
      if (lease !== undefined) leases.push(lease);
      return lease;
    }, (error: unknown) => error);
    expect(refusal).toBeInstanceOf(HomeDaemonConflictError);
    expect((refusal as HomeDaemonConflictError).pid).toBe(first.pid);
    expect(String(refusal)).toContain(`pid ${first.pid}`);

    const exited = once(first, "exit");
    first.kill("SIGKILL");
    await exited;
    expect(homeDaemonLockHeld(home)).toBe(false);
    const lease = await holdHomeDaemonLock(home);
    expect(lease).toBeDefined();
    leases.push(lease!);
    expect(homeDaemonHolder(home)).toBe(process.pid);
    lease!.release();
    expect(homeDaemonHolder(home)).toBeUndefined();
    expect(homeDaemonLockHeld(home)).toBe(false);
  }, 30_000);

  // A start beside a daemon still in its start waits for it on this record: before, nothing told a daemon in its start
  // from one that had started and did not answer. The record says where the daemon's output starts in the home's log,
  // so a start that found it and sees it fail shows that output whole, what it wrote before the find included.
  it("records its holder as starting from the lock on, with where the log ended then, and as started once it says so, its pid kept throughout", async () => {
    const home = await temporaryHome();
    const earlier = "rerail daemon: an earlier run's line\n";
    await writeFile(join(home, DAEMON_LOG_FILE), earlier);
    const lease = await holdHomeDaemonLock(home);
    expect(lease).toBeDefined();
    leases.push(lease!);
    expect(homeDaemonStarting(home)).toEqual({ pid: process.pid, logOffset: Buffer.byteLength(earlier) });
    expect(homeDaemonHolder(home)).toBe(process.pid);
    lease!.started();
    expect(homeDaemonStarting(home)).toBeUndefined();
    expect(homeDaemonHolder(home)).toBe(process.pid);
    lease!.release();
    expect(homeDaemonStarting(home)).toBeUndefined();
  });

  // The record said `starting` for the daemon's whole life when its rewrite at the end of the start failed: the rewrite
  // was made only after a read of the record, which can fail, and a failed write was not logged.
  it("records its start as over though its record does not read", async () => {
    const home = await temporaryHome();
    const lease = await holdHomeDaemonLock(home);
    leases.push(lease!);
    await writeFile(join(home, LOCKS_DIR, "daemon.json"), "not a record");
    lease!.started();
    expect(homeDaemonHolder(home)).toBe(process.pid);
  });

  // Where the file system gives no file ids, nothing tells whether the home at the path is still this daemon's: the end
  // of its start wrote over the record of another daemon in its start, in a home made again at that path.
  it.each(["from the lock on", "once the lock is taken"])("where the file system gives no file id %s, records its start as over in its own record or one that does not read, and leaves another process's", async when => {
    const home = await temporaryHome();
    fileIds.hidden = when === "from the lock on";
    const lease = await holdHomeDaemonLock(home);
    leases.push(lease!);
    fileIds.hidden = true;
    expect(lease!.holdsHome()).toBe(true);
    lease!.started();
    expect(homeDaemonStarting(home)).toBeUndefined();
    expect(homeDaemonHolder(home)).toBe(process.pid);
    const record = join(home, LOCKS_DIR, "daemon.json");
    const other = { pid: process.pid + 1, logOffset: 0 };
    await writeFile(record, JSON.stringify({ ...other, starting: true }));
    lease!.started();
    expect(homeDaemonStarting(home)).toEqual(other);
    await writeFile(record, "not a record");
    lease!.started();
    expect(homeDaemonHolder(home)).toBe(process.pid);
  });

  // The daemon calls it once it serves: a throw there would end a daemon that serves.
  it("says in its log that it could not record its start as over when its write fails, and returns", async () => {
    const home = await temporaryHome();
    const lease = await holdHomeDaemonLock(home);
    leases.push(lease!);
    const log = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    writes.failing = 1;
    expect(() => lease!.started()).not.toThrow();
    expect(log.mock.calls).toEqual([[`rerail daemon: could not record in ${join(home, LOCKS_DIR, "daemon.json")} that this daemon's start is over\n`]]);
  });

  it("reads a starting record that says nothing of the log as output from the log's beginning", async () => {
    const home = await temporaryHome();
    await mkdir(join(home, LOCKS_DIR), { recursive: true });
    await writeFile(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: 4242, starting: true }));
    expect(homeDaemonStarting(home)).toEqual({ pid: 4242, logOffset: 0 });
  });

  it("waits out a process that holds the lock for an instant, as a start asking whether a daemon runs does", async () => {
    const home = await temporaryHome();
    expect(homeDaemonLockHeld(home)).toBeUndefined();
    // Asking makes nothing: a home where no daemon ever ran gets no lock file.
    expect(existsSync(join(home, LOCKS_DIR))).toBe(false);
    const lease = await holdHomeDaemonLock(home);
    leases.push(lease!);
    lease!.release();
    const asking = loadNativeBinding().tryLockExclusive(join(home, LOCKS_DIR, "daemon.lock"));
    expect(asking).not.toBeNull();
    setTimeout(() => asking!.release(), 200);
    const taken = await holdHomeDaemonLock(home);
    expect(taken).toBeDefined();
    leases.push(taken!);
    expect(homeDaemonHolder(home)).toBe(process.pid);
  });

  // Windows keeps a loaded DLL from being deleted, and the uninstall's stop asks the lock before it removes `versions/`.
  it.runIf(process.platform === "win32")("tells on Windows that a daemon holds the home without loading the engine", async () => {
    const home = await temporaryHome();
    await holdInAnotherProcess(home);
    const asking = spawn(process.execPath, [
      "--import", "tsx", "--input-type=module", "-e",
      `import { homeDaemonLockHeld } from ${JSON.stringify(lockModule)};
       process.stdout.write(String(homeDaemonLockHeld(${JSON.stringify(home)})));`,
    ], { cwd: packageRoot, env: { ...process.env, RERAIL_ENGINE_PATH: join(home, "no-engine.node") }, stdio: ["ignore", "pipe", "pipe"] });
    children.push(asking);
    let answer = "";
    asking.stdout.setEncoding("utf8").on("data", (chunk: string) => { answer += chunk; });
    await once(asking, "exit");
    expect(answer).toBe("true");
  }, 30_000);

  // The daemon holds its lock on a file no path reaches now, and a lock taken at the path would be on another file.
  it("tells nothing of a home whose lock file was deleted under the daemon that holds it", async () => {
    const home = await temporaryHome();
    await holdInAnotherProcess(home);
    expect(homeDaemonLockHeld(home)).toBe(true);
    await rm(join(home, LOCKS_DIR, "daemon.lock"));
    expect(homeDaemonLockHeld(home)).toBeUndefined();
  }, 30_000);

  it("goes on without the lock when its file does not open, as where the engine does not load", async () => {
    const home = await temporaryHome();
    // A folder where the lock file goes: opening it fails on every platform, for any user.
    await mkdir(join(home, LOCKS_DIR, "daemon.lock"), { recursive: true });
    expect(homeDaemonLockHeld(home)).toBeUndefined();
    // The daemon runs as on a machine without the lock, kept to one by its engine socket or pipe, never refused.
    expect(await holdHomeDaemonLock(home)).toBeUndefined();
  });
  // Windows keeps a folder that holds an open file from being moved, so a home there cannot go while its daemon runs.
  it.skipIf(process.platform === "win32")("holds the home only while the lock file it locked is at its path: not once the home is moved or made again", async () => {
    const home = await temporaryHome();
    const lease = await holdHomeDaemonLock(home);
    expect(lease).toBeDefined();
    leases.push(lease!);
    expect(lease!.holdsHome()).toBe(true);
    const moved = `${home}-moved`;
    homes.push(moved);
    await rename(home, moved);
    expect(lease!.holdsHome()).toBe(false);
    // A home made again at the path, a daemon's lock file in it or not, is another home.
    await mkdir(join(home, LOCKS_DIR), { recursive: true });
    expect(lease!.holdsHome()).toBe(false);
    const other = await holdInAnotherProcess(home);
    expect(homeDaemonHolder(home)).toBe(other.pid);
    expect(lease!.holdsHome()).toBe(false);
    // The end of its start leaves the record of the daemon that holds the home at the path now.
    lease!.started();
    expect(homeDaemonHolder(home)).toBe(other.pid);
    // Letting go, as its process exits, leaves the record of the daemon that holds the home at the path now.
    lease!.release();
    expect(homeDaemonHolder(home)).toBe(other.pid);
  }, 30_000);
});
