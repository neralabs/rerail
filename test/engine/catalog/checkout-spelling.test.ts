import { realpathSync, symlinkSync, type PathLike } from "node:fs";
import { mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { isHomeFolder } from "../../../src/cli/repository-setup.js";
import { physicalPath } from "../../../src/engine/catalog/checkout-control.js";

const roots: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

async function folder(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-spelling-")));
  roots.push(root);
  return root;
}

describe("a checkout path's spelling", () => {
  it("is the native realpath's on Windows, which spells each name as stored, and the JS realpath's elsewhere", async () => {
    const root = await folder();
    // On Windows the native realpath turns `c:\work\app` into `C:\work\app`; the JS one keeps the caller's spelling.
    const native = vi.spyOn(realpathSync, "native").mockImplementation(((path: PathLike) => `${String(path)}-as-stored`) as typeof realpathSync.native);
    expect(physicalPath(join(root, "not-yet"), "win32")).toBe(join(`${root}-as-stored`, "not-yet"));
    expect(native).toHaveBeenCalled();
    native.mockClear();
    expect(physicalPath(join(root, "not-yet"), "linux")).toBe(join(root, "not-yet"));
    expect(native).not.toHaveBeenCalled();
  });

  it("names the home folder by any path that leads to it, so set-up refuses it under another spelling", async () => {
    const root = await folder();
    const home = join(root, "home");
    await mkdir(home);
    // A junction needs no privilege on Windows; elsewhere the type is ignored and this is a symlink.
    const alias = join(root, "alias");
    symlinkSync(home, alias, "junction");
    expect(isHomeFolder(alias, home)).toBe(true);
    expect(isHomeFolder(root, home)).toBe(false);
  });
});
