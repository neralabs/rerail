import { createHash } from "node:crypto";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
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
    const lock = join(root, "claude-code-locks", `${createHash("sha256").update("session-1").digest("hex")}.lock`);
    await mkdir(dirname(lock), { recursive: true });
    await writeFile(lock, JSON.stringify({ pid: DEAD_PID, token: "dead-hook" }));
    vi.mocked(rm).mockImplementationOnce(async path => {
      throw Object.assign(new Error(`EPERM: operation not permitted, unlink '${String(path)}'`), { code: "EPERM", syscall: "unlink" });
    });

    await expect(withSessionLock(root, "claude-code", "session-1", async () => "ran", { platform: "win32" })).resolves.toBe("ran");
  });
});
