import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import { withSessionLock } from "../../src/harness/hook-adapter.js";
import { removeFolder } from "../support/platform/remove-folder.js";

vi.mock("node:fs/promises", { spy: true });

/** A pid no live process can hold on the platforms Rerail supports. */
const DEAD_PID = 2 ** 22 - 1;

const roots: string[] = [];
afterEach(async () => {
  vi.mocked(writeFile).mockReset();
  vi.mocked(rm).mockReset();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

async function lockRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-session-lock-"));
  roots.push(root);
  return root;
}

/** The next lock file create fails with `code`, as Windows fails it while another hook of the session deletes that file. */
function failNextLockCreate(code: string): void {
  vi.mocked(writeFile).mockImplementationOnce(async path => {
    throw Object.assign(new Error(`${code}: operation not permitted, open '${String(path)}'`), { code, syscall: "open" });
  });
}

function lockPath(root: string): string {
  return join(root, "claude-code-locks", `${createHash("sha256").update("session-1").digest("hex")}.lock`);
}

/** A lock file at `root` for session-1 holding `text`, as a hook that ended holding the lock left it. */
async function leftLock(root: string, text: string): Promise<void> {
  const lock = lockPath(root);
  await mkdir(dirname(lock), { recursive: true });
  await writeFile(lock, text);
}

/** The next removal fails with `code`, as Windows refuses it while a scanner holds the file this hook just wrote. */
function refuseNextRemoval(code: string): void {
  vi.mocked(rm).mockImplementationOnce(async path => {
    throw Object.assign(new Error(`${code}: operation not permitted, unlink '${String(path)}'`), { code, syscall: "unlink" });
  });
}

/** Every removal fails with `code`, which no retry waits out. */
function refuseRemovals(code: string): void {
  vi.mocked(rm).mockImplementation(async path => {
    throw Object.assign(new Error(`${code}: permission denied, unlink '${String(path)}'`), { code, syscall: "unlink" });
  });
}

describe("a hook taking its session's lock", () => {
  it.each(["EPERM", "EACCES", "EBUSY"])("waits through %s on Windows, where another hook is deleting the lock file, then takes it", async code => {
    const root = await lockRoot();
    failNextLockCreate(code);
    await expect(withSessionLock(root, "claude-code", "session-1", async () => "ran", { platform: "win32" })).resolves.toBe("ran");
  });

  it("fails at once on EPERM elsewhere, where it means the folder refuses the hook", async () => {
    const root = await lockRoot();
    failNextLockCreate("EPERM");
    await expect(withSessionLock(root, "claude-code", "session-1", async () => "ran", { platform: "linux" })).rejects.toMatchObject({ code: "EPERM" });
  });

  it("waits on Windows when another hook is already deleting the lock a dead hook left, instead of failing to remove it", async () => {
    const root = await lockRoot();
    await leftLock(root, JSON.stringify({ pid: DEAD_PID, token: "dead-hook" }));
    refuseNextRemoval("EPERM");

    await expect(withSessionLock(root, "claude-code", "session-1", async () => "ran", { platform: "win32" })).resolves.toBe("ran");
  });

  it("takes a lock whose holder's pid another live process has since been given, once the lock has gone silent", async () => {
    const root = await lockRoot();
    // This test's own process runs, and holds no lock: the pid Windows gave on to the agent's next shell.
    await leftLock(root, JSON.stringify({ pid: process.pid, token: "ended-hook" }));

    await expect(withSessionLock(root, "claude-code", "session-1", async () => "ran", { heartbeatMs: 20 })).resolves.toBe("ran");
  });

  it("takes a lock file its holder ended before writing itself into, once it has gone silent", async () => {
    const root = await lockRoot();
    await leftLock(root, "");

    await expect(withSessionLock(root, "claude-code", "session-1", async () => "ran", { heartbeatMs: 20 })).resolves.toBe("ran");
  });

  it("never takes the lock from a holder that still beats, however long it holds it", async () => {
    const root = await lockRoot();
    const order: string[] = [];
    const holderHasIt = Promise.withResolvers<void>();
    const holder = withSessionLock(root, "claude-code", "session-1", async () => {
      holderHasIt.resolve();
      // Real time, as the beat is the holder's real timer touching a real file: thirty beats, three times the silence
      // after which a waiter takes a lock.
      await new Promise(resolve_ => setTimeout(resolve_, 600));
      order.push("holder done");
    }, { heartbeatMs: 20 });
    await holderHasIt.promise;
    const waiter = withSessionLock(root, "claude-code", "session-1", async () => {
      order.push("waiter ran");
    }, { heartbeatMs: 20 });

    await Promise.all([holder, waiter]);
    expect(order).toEqual(["holder done", "waiter ran"]);
  });

  it("leaves no lock behind when its process exits holding it, as the parent watch ends a hook on Windows", async () => {
    const root = await lockRoot();
    const script = join(root, "exit-holding.mts");
    const adapter = pathToFileURL(resolve("src/harness/hook-adapter.ts")).href;
    await writeFile(script, [
      `import { withSessionLock } from ${JSON.stringify(adapter)};`,
      `await withSessionLock(${JSON.stringify(root)}, "claude-code", "session-1", async () => process.exit(0));`,
    ].join("\n"));

    const run = spawnSync(process.execPath, ["--import", "tsx", script], { encoding: "utf8" });

    expect(run).toMatchObject({ status: 0, stderr: "" });
    expect(existsSync(dirname(lockPath(root)))).toBe(true);
    expect(existsSync(lockPath(root))).toBe(false);
  });
});

describe("a hook letting go of its session's lock", () => {
  it("waits on Windows through a removal a scanner refuses, and keeps the hook's answer", async () => {
    const root = await lockRoot();
    refuseNextRemoval("EPERM");

    await expect(withSessionLock(root, "claude-code", "session-1", async () => "answer", { platform: "win32" })).resolves.toBe("answer");
    expect(existsSync(lockPath(root))).toBe(false);
  });

  it("waits on Windows through a refused removal before the daemon's check, and lets another hook in during it", async () => {
    const root = await lockRoot();
    refuseNextRemoval("EBUSY");

    const ingested = withSessionLock(root, "claude-code", "session-1", async lock => await lock.released(async () => {
      // Another hook of the session takes the lock while this one waits for the daemon.
      await withSessionLock(root, "claude-code", "session-1", async () => undefined, { platform: "win32" });
      return "ingested";
    }), { platform: "win32" });

    await expect(ingested).resolves.toBe("ingested");
    expect(existsSync(lockPath(root))).toBe(false);
  });

  it("keeps the hook's answer when the lock file will not go at all", async () => {
    const root = await lockRoot();
    refuseRemovals("EACCES");

    await expect(withSessionLock(root, "claude-code", "session-1", async () => "answer", { platform: "win32" })).resolves.toBe("answer");
  });

  it("still sends the event, holding the lock, when the lock file will not go before the daemon's check", async () => {
    const root = await lockRoot();
    refuseRemovals("EACCES");

    const ingested = withSessionLock(root, "claude-code", "session-1", async lock => await lock.released(async () => "ingested"), { platform: "win32" });

    await expect(ingested).resolves.toBe("ingested");
  });
});
