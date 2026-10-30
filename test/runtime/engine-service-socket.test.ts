import { existsSync } from "node:fs";
import { chmod, lstat, mkdir, mkdtemp, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { preparePrivateDirectory, startEngineService } from "../../src/runtime/engine-service.js";

describe.runIf(process.platform !== "win32")("engine service socket directory", () => {
  const roots: string[] = [];

  async function temporaryRoot(): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), "rerail-socket-"));
    roots.push(root);
    return root;
  }

  afterEach(async () => {
    await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
  });

  it("listens inside a directory it makes for this user alone", async () => {
    const directory = join(await temporaryRoot(), "rerail-socket", "nested");
    const socketPath = join(directory, "engine.sock");
    const service = await startEngineService({ socketPath, storageRoot: join(directory, "..", "state") });
    try {
      const info = await lstat(directory);
      expect(info.isDirectory()).toBe(true);
      expect(info.uid).toBe(process.getuid!());
      expect(info.mode & 0o777).toBe(0o700);
      expect((await lstat(socketPath)).isSocket()).toBe(true);
    } finally {
      await service.close();
    }
  });

  it("refuses a directory that others may write, and listens nowhere", async () => {
    const directory = join(await temporaryRoot(), "shared");
    await mkdir(directory);
    await chmod(directory, 0o777);
    const socketPath = join(directory, "engine.sock");
    await expect(startEngineService({ socketPath, storageRoot: join(directory, "state") })).rejects.toThrow(/has mode 777, not 700/u);
    expect(existsSync(socketPath)).toBe(false);
  });

  it("refuses a directory another user owns, and a link standing in for a directory", async () => {
    const root = await temporaryRoot();
    const directory = join(root, "owned");
    await mkdir(directory, { mode: 0o700 });
    await expect(preparePrivateDirectory(directory, process.getuid!() + 1)).rejects.toThrow(/is owned by uid/u);
    await expect(preparePrivateDirectory(directory)).resolves.toBeUndefined();

    const link = join(root, "link");
    await symlink(directory, link);
    const socketPath = join(link, "engine.sock");
    await expect(startEngineService({ socketPath, storageRoot: join(root, "state") })).rejects.toThrow(/is a link or not a directory/u);
    expect(existsSync(join(directory, "engine.sock"))).toBe(false);
  });
});
