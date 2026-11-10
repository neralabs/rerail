import { existsSync } from "node:fs";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { START_LOCK_DIR, withStartLock } from "../../src/runtime/daemon-control.js";
import { fakeMachine } from "../support/platform/daemon-machine.js";
import { removeFolder } from "../support/platform/remove-folder.js";

vi.mock("node:fs/promises", { spy: true });

const homes: string[] = [];
afterEach(async () => {
  vi.mocked(mkdir).mockReset();
  vi.mocked(rm).mockReset();
  await Promise.all(homes.splice(0).map(home => removeFolder(home)));
});

async function temporaryHome(): Promise<string> {
  const home = await mkdtemp(join(tmpdir(), "rerail-start-lock-"));
  homes.push(home);
  return home;
}

const lease = { leaseMs: 60_000, renewMs: 20_000 };

/** An error as Windows raises it for a folder another process is deleting. */
const deleting = (code: string, syscall: string, path: unknown): Error =>
  Object.assign(new Error(`${code}: operation not permitted, ${syscall} '${String(path)}'`), { code, syscall });

/** The next lock folder create fails with `code`, as Windows fails it while another start deletes that folder. */
function failNextLockCreate(code: string): void {
  vi.mocked(mkdir).mockImplementationOnce(async path => { throw deleting(code, "mkdir", path); });
}

describe("a daemon start taking the start lock", () => {
  it.each(["EPERM", "EACCES", "EBUSY"])("waits through %s on Windows, where another start is deleting the lock folder, then takes it", async code => {
    const home = await temporaryHome();
    failNextLockCreate(code);

    await expect(withStartLock(home, async () => "ran", lease, fakeMachine("win32"))).resolves.toBe("ran");
    expect(existsSync(join(home, START_LOCK_DIR))).toBe(false);
  });

  it("waits on Windows when the folder it took for stale is one another start is still deleting", async () => {
    const home = await temporaryHome();
    failNextLockCreate("EPERM");
    // The folder has no lease to read, so it looks stale, and Windows refuses to remove a folder it is deleting.
    vi.mocked(rm).mockImplementationOnce(async path => { throw deleting("EPERM", "rmdir", path); });

    await expect(withStartLock(home, async () => "ran", lease, fakeMachine("win32"))).resolves.toBe("ran");
  });

  it("fails at once on EPERM elsewhere, where it means the home refuses this process", async () => {
    const home = await temporaryHome();
    failNextLockCreate("EPERM");

    await expect(withStartLock(home, async () => "ran", lease, fakeMachine("linux"))).rejects.toMatchObject({ code: "EPERM" });
  });
});
