import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { realpathSync, type PathLike } from "node:fs";
import { tmpdir } from "node:os";
import { join, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { physicalPath, storedPath } from "../../../../src/engine/catalog/checkout-control.js";
import { linkFolder } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

async function temporaryRoot(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-refused-path-")));
  roots.push(root);
  return root;
}

/**
 * Makes the native realpath fail with `code` for `refused` and every path below it, as Windows's does with `EPERM` for a
 * file or folder the user is denied, or whose delete is pending, while `lstat` and `existsSync` still answer for it.
 */
function refuseNativeRealpath(refused: string, code = "EPERM"): void {
  const native = realpathSync.native;
  // The cast drops the overloads that take an encoding: the paths asked here are strings.
  vi.spyOn(realpathSync, "native").mockImplementation(((path: PathLike) => {
    const named = String(path);
    if (named === refused || named.startsWith(`${refused}${sep}`)) throw Object.assign(new Error(`${code}: realpath '${named}'`), { code });
    return native(path);
  }) as typeof realpathSync.native);
}

describe("a path whose real path the system refuses", () => {
  it("is the real path of the folder holding it, plus its name, for a file the user is denied", async () => {
    const root = await temporaryRoot();
    await mkdir(join(root, "app"));
    const secret = join(root, "app", "secret.py");
    await writeFile(secret, "SECRET = 1\n");
    refuseNativeRealpath(secret);
    // Each lookup a walk makes of the file asks this; a throw here failed the whole walk.
    expect(physicalPath(secret, "win32")).toBe(join(realpathSync.native(join(root, "app")), "secret.py"));
    expect(storedPath(secret)).toBe(join(realpathSync.native(join(root, "app")), "secret.py"));
  });

  it("is the real path of the folder above it, plus each name below, for a folder the user is denied and a file in it", async () => {
    const root = await temporaryRoot();
    await mkdir(join(root, "app", "locked"), { recursive: true });
    await writeFile(join(root, "app", "locked", "hidden.py"), "x = 1\n");
    refuseNativeRealpath(join(root, "app", "locked"));
    const above = realpathSync.native(join(root, "app"));
    expect(physicalPath(join(root, "app", "locked"), "win32")).toBe(join(above, "locked"));
    expect(physicalPath(join(root, "app", "locked", "hidden.py"), "win32")).toBe(join(above, "locked", "hidden.py"));
  });

  it("is the real path of the folder that held it, plus its name, for a path gone between the look and the real path", async () => {
    const root = await temporaryRoot();
    const going = join(root, "going.py");
    await writeFile(going, "x = 1\n");
    refuseNativeRealpath(going, "ENOENT");
    expect(physicalPath(going, "win32")).toBe(join(realpathSync.native(root), "going.py"));
  });

  it("stays refused for a link, whose target the refusal hides", async () => {
    const root = await temporaryRoot();
    await mkdir(join(root, "elsewhere"));
    const link = join(root, "link");
    linkFolder(join(root, "elsewhere"), link);
    refuseNativeRealpath(link);
    expect(() => physicalPath(link, "win32")).toThrow(expect.objectContaining({ code: "EPERM" }));
    expect(() => physicalPath(join(link, "a.py"), "win32")).toThrow(expect.objectContaining({ code: "EPERM" }));
  });

  it("stays refused for any other failure", async () => {
    const root = await temporaryRoot();
    const busy = join(root, "busy.py");
    await writeFile(busy, "x = 1\n");
    refuseNativeRealpath(busy, "EIO");
    expect(() => physicalPath(busy, "win32")).toThrow(expect.objectContaining({ code: "EIO" }));
  });
});
