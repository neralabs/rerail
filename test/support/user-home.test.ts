import { realpathSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { stubUserHome } from "./user-home.js";

const under = (path: string, root: string): boolean => realpathSync(path).startsWith(realpathSync(root));

describe("the user home a test sees", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("is a folder of the test file's own under the temp folder, so no test writes the person's real profile", () => {
    // os.homedir() reads HOME on POSIX and USERPROFILE on Windows; both must point away from the real profile.
    expect(under(homedir(), tmpdir())).toBe(true);
    expect(process.env.HOME).toBe(homedir());
    expect(process.env.USERPROFILE).toBe(homedir());
  });

  it("moves with stubUserHome on every platform, not only where os.homedir() reads HOME", () => {
    const moved = join(homedir(), "moved");
    stubUserHome(moved);
    expect(homedir()).toBe(moved);
    expect(process.env.HOME).toBe(moved);
  });
});
