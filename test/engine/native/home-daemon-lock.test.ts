import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  holdHomeDaemonLock,
  HomeDaemonConflictError,
  homeDaemonHolder,
  homeDaemonLockHeld,
  type HomeDaemonLease,
} from "../../../src/engine/native/home-daemon-lock.js";
import { LOCKS_DIR } from "../../../src/engine/native/file-lock.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const packageRoot = resolve(import.meta.dirname, "..", "..", "..");
const lockModule = pathToFileURL(join(packageRoot, "src", "engine", "native", "home-daemon-lock.ts")).href;

const children: ChildProcess[] = [];
const leases: HomeDaemonLease[] = [];
const homes: string[] = [];

afterEach(async () => {
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

  it("waits out a process that holds the lock for an instant, as a start asking whether a daemon runs does", async () => {
    const home = await temporaryHome();
    expect(homeDaemonLockHeld(home)).toBe(false);
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
});
