import { symlinkSync } from "node:fs";
import { mkdir, mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { isHomeFolder } from "../../src/cli/repository-setup.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

describe("set-up's home folder check", () => {
  it("names the home folder by any path that leads to it, so set-up refuses it under another spelling", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-home-folder-")));
    roots.push(root);
    const home = join(root, "home");
    await mkdir(home);
    // A junction needs no privilege on Windows; elsewhere the type is ignored and this is a symlink.
    const alias = join(root, "alias");
    symlinkSync(home, alias, "junction");
    expect(isHomeFolder(alias, home)).toBe(true);
    expect(isHomeFolder(root, home)).toBe(false);
  });
});
