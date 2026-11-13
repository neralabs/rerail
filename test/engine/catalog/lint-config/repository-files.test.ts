import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { repositoryFiles, withRepositoryFiles } from "../../../../src/engine/catalog/lint-config/repository-files.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-repository-files-")));
  temporaryDirectories.push(root);
  mkdirSync(join(root, "sub"));
  mkdirSync(join(root, "node_modules"));
  writeFileSync(join(root, "sub", "ruff.toml"), "");
  writeFileSync(join(root, "node_modules", "ruff.toml"), "");
  return root;
}

describe("repositoryFiles", () => {
  it("lists the files with `/`, leaving out the folders the pack matcher leaves out, afresh on every call", async () => {
    const root = await repository();
    expect(repositoryFiles(root)).toEqual(["sub/ruff.toml"]);
    writeFileSync(join(root, "a.py"), "");
    expect(repositoryFiles(root)).toEqual(["a.py", "sub/ruff.toml"]);
  });

  it("lists each root once while withRepositoryFiles runs, nested runs included, and afresh after it", async () => {
    const [root, other] = [await repository(), await repository()];
    const listed = withRepositoryFiles(() => {
      const first = repositoryFiles(root);
      writeFileSync(join(root, "a.py"), "");
      return { first, again: withRepositoryFiles(() => repositoryFiles(root)), other: repositoryFiles(other) };
    });
    expect(listed).toEqual({ first: ["sub/ruff.toml"], again: ["sub/ruff.toml"], other: ["sub/ruff.toml"] });
    expect(repositoryFiles(root)).toEqual(["a.py", "sub/ruff.toml"]);
  });
});
