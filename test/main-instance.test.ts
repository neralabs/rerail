import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync, readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import { startDaemon, stopDaemon } from "../src/daemon-control.js";
import {
  acquireMainInstance,
  assertContentStoreOwner,
  MAIN_INSTANCE_HOLDER_FILE,
  MainInstanceConflictError,
  type MainInstanceLease,
  type MainRerailBuild,
} from "../src/main-instance.js";
import { mainContentStoreDirectory } from "../src/rerail-build.js";

const packageRoot = resolve(import.meta.dirname, "..");
const mainInstanceModule = pathToFileURL(join(packageRoot, "src", "main-instance.ts")).href;
const build: MainRerailBuild = {
  kind: "main",
  home: "/managed/.rerail",
  version: "2.0.0",
  packageRoot: "/managed/.rerail/versions/2.0.0/node_modules/@neralabs/rerail",
};

const children: ChildProcess[] = [];
const leases: MainInstanceLease[] = [];
const directories: string[] = [];

afterEach(async () => {
  for (const lease of leases.splice(0)) lease.release();
  for (const child of children.splice(0)) {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      child.kill("SIGKILL");
      await exited;
    }
  }
  vi.unstubAllEnvs();
  for (const directory of directories.splice(0)) await rm(directory, { recursive: true, force: true });
});

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), "rerail-main-instance-"));
  directories.push(directory);
  return directory;
}

/** A separate process that becomes the main instance in `directory` and stays so until it is killed. */
async function holdInAnotherProcess(directory: string, version: string): Promise<ChildProcess> {
  const child = spawn(process.execPath, [
    "--import", "tsx", "--input-type=module", "-e",
    `import { acquireMainInstance } from ${JSON.stringify(mainInstanceModule)};
     acquireMainInstance(${JSON.stringify({ ...build, version })}, ${JSON.stringify(directory)});
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

/** The refusal a second main instance in `directory` meets. */
function conflict(directory: string): MainInstanceConflictError {
  let refusal: unknown;
  try {
    leases.push(acquireMainInstance(build, directory));
  } catch (error) {
    refusal = error;
  }
  expect(refusal, "a second main instance was allowed to run").toBeInstanceOf(MainInstanceConflictError);
  return refusal as MainInstanceConflictError;
}

describe("the single main instance", () => {
  it("refuses a second main instance in another process and names the holder", async () => {
    const directory = await temporaryDirectory();
    const holder = await holdInAnotherProcess(directory, "1.4.0");

    const refused = conflict(directory);

    expect(refused.holder).toEqual(expect.objectContaining({ pid: holder.pid, version: "1.4.0", packageRoot: build.packageRoot }));
    expect(refused.message).toContain(`pid ${holder.pid}, version 1.4.0`);
  }, 30_000);

  it("is free again once the holder is killed, with no stale state to reclaim", async () => {
    const directory = await temporaryDirectory();
    const holder = await holdInAnotherProcess(directory, "1.4.0");
    conflict(directory);

    const exited = once(holder, "exit");
    holder.kill("SIGKILL");
    await exited;
    const lease = acquireMainInstance(build, directory);
    leases.push(lease);

    expect(JSON.parse(readFileSync(join(directory, MAIN_INSTANCE_HOLDER_FILE), "utf8"))).toEqual(
      expect.objectContaining({ pid: process.pid, version: build.version }),
    );
    lease.release();
    expect(existsSync(join(directory, MAIN_INSTANCE_HOLDER_FILE))).toBe(false);
  }, 30_000);

  it("lets only the main instance open a store in the machine-wide store directory", async () => {
    const home = await temporaryDirectory();
    vi.stubEnv("HOME", home);
    vi.stubEnv("USERPROFILE", home);
    const mainStore = join(mainContentStoreDirectory(), "content-v1.sqlite");
    const checkoutStore = join(home, "checkout", ".rerail", "content-v1.sqlite");

    expect(() => assertContentStoreOwner(mainStore)).toThrow(/belongs to the main Rerail daemon/);
    assertContentStoreOwner(checkoutStore);

    const lease = acquireMainInstance(build);
    leases.push(lease);
    assertContentStoreOwner(mainStore);
    lease.release();
    expect(() => assertContentStoreOwner(mainStore)).toThrow(/belongs to the main Rerail daemon/);
  });

  it("starts a checkout's daemon while the main instance runs, without touching its lock", async () => {
    const root = await temporaryDirectory();
    const machineHome = join(root, "user");
    const rerailHome = join(root, "rerail-home");
    const mainDirectory = join(machineHome, ".cache", "rerail");
    const holder = await holdInAnotherProcess(mainDirectory, "1.4.0");

    try {
      const started = await startDaemon({ home: rerailHome, packageRoot }, {
        dev: true,
        port: 0,
        env: { ...process.env, HOME: machineHome, USERPROFILE: machineHome },
      });
      expect(started.started).toBe(true);
    } finally {
      await stopDaemon(rerailHome);
    }

    expect(JSON.parse(readFileSync(join(mainDirectory, MAIN_INSTANCE_HOLDER_FILE), "utf8"))).toEqual(
      expect.objectContaining({ pid: holder.pid }),
    );
    expect(conflict(mainDirectory).holder?.pid).toBe(holder.pid);
  }, 30_000);
});
