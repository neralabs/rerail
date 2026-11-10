import { afterEach, describe, expect, it } from "vitest";
import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pinnedNodePath } from "../../src/setup/node-path.js";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

/** A Homebrew prefix holding `<formula>` at `version` in its Cellar; the keg's node is returned. */
async function keg(prefix: string, formula: string, version: string): Promise<string> {
  const node = join(prefix, "Cellar", formula, version, "bin", "node");
  await mkdir(dirname(node), { recursive: true });
  await writeFile(node, "", { mode: 0o755 });
  await mkdir(join(prefix, "opt"), { recursive: true });
  await symlink(join("..", "Cellar", formula, version), join(prefix, "opt", formula));
  return node;
}

async function homebrew(): Promise<string> {
  const prefix = await realpath(await mkdtemp(join(tmpdir(), "rerail-node-path-")));
  directories.push(prefix);
  await mkdir(join(prefix, "bin"));
  return prefix;
}

describe("pinnedNodePath", () => {
  it("records Homebrew's node by the link brew upgrade keeps, not its versioned keg", async () => {
    const prefix = await homebrew();
    const node = await keg(prefix, "node", "26.5.0");
    await symlink(join("..", "Cellar", "node", "26.5.0", "bin", "node"), join(prefix, "bin", "node"));
    expect(pinnedNodePath(node)).toBe(join(prefix, "bin", "node"));
  });

  it("records a formula Homebrew does not link into bin by its opt link", async () => {
    const prefix = await homebrew();
    const linked = await keg(prefix, "node", "26.5.0");
    await symlink(linked, join(prefix, "bin", "node"));
    const node = await keg(prefix, "node@24", "24.22.1");
    expect(pinnedNodePath(node)).toBe(join(prefix, "opt", "node@24", "bin", "node"));
  });

  it("keeps the keg where no Homebrew link reaches it, and any other node as it is", async () => {
    const prefix = await homebrew();
    const node = join(prefix, "Cellar", "node", "26.5.0", "bin", "node");
    await mkdir(dirname(node), { recursive: true });
    await writeFile(node, "", { mode: 0o755 });
    expect(pinnedNodePath(node)).toBe(node);
    const nvm = join(prefix, ".nvm", "versions", "node", "v24.22.1", "bin", "node");
    expect(pinnedNodePath(nvm)).toBe(nvm);
  });
});
