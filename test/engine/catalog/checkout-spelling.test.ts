import { realpathSync, type PathLike } from "node:fs";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { checkoutRealpath, physicalPath } from "../../../src/engine/catalog/checkout-control.js";

const roots: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

describe("a checkout path's spelling", () => {
  it("is the native realpath's on Windows, which spells each name as stored, and the JS realpath's elsewhere", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-spelling-")));
    roots.push(root);
    // On Windows the native realpath turns `c:\work\app` into `C:\work\app`; the JS one keeps the caller's spelling.
    const native = vi.spyOn(realpathSync, "native").mockImplementation(((path: PathLike) => `${String(path)}-as-stored`) as typeof realpathSync.native);
    expect(physicalPath(join(root, "not-yet"), "win32")).toBe(join(`${root}-as-stored`, "not-yet"));
    expect(native).toHaveBeenCalled();
    native.mockClear();
    expect(physicalPath(join(root, "not-yet"), "linux")).toBe(join(root, "not-yet"));
    expect(native).not.toHaveBeenCalled();
  });

  it.runIf(process.platform === "win32")("is Git's for a checkout typed with a lower-case drive letter, as VS Code types a session's folder", async () => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-checkout-spelling-")));
    roots.push(root);
    const typed = root.replace(/^[A-Z]:/u, drive => drive.toLowerCase());
    expect(typed).not.toBe(root);
    expect(checkoutRealpath(typed)).toBe(root);
  });
});
