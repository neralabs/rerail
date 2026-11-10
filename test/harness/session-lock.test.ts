import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { withSessionLock } from "../../src/harness/hook-adapter.js";
import { removeFolder } from "../support/platform/remove-folder.js";

vi.mock("node:fs/promises", { spy: true });

const roots: string[] = [];
afterEach(async () => {
  vi.mocked(writeFile).mockReset();
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
});
