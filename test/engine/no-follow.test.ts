import { closeSync, constants, readFileSync, symlinkSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { openNoFollow, openNoFollowSync } from "../../src/engine/no-follow.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

/** Whether this machine lets the test make a file symlink: Windows needs Developer Mode or admin rights. */
function linked(target: string, link: string): boolean {
  try {
    symlinkSync(target, link, "file");
    return true;
  } catch {
    return false;
  }
}

describe("openNoFollow", () => {
  it("refuses a symlink at the name without O_NOFOLLOW, as on Windows, and opens a regular file", async (context) => {
    const root = await mkdtemp(join(tmpdir(), "rerail-no-follow-"));
    roots.push(root);
    const file = join(root, "record.json");
    await writeFile(file, "{}");
    const handle = await openNoFollow(file, constants.O_RDONLY, undefined, undefined);
    expect(await handle.readFile("utf8")).toBe("{}");
    await handle.close();
    const link = join(root, "planted.json");
    if (!linked(file, link)) {
      context.skip();
      return;
    }
    await expect(openNoFollow(link, constants.O_RDONLY, undefined, undefined)).rejects.toMatchObject({ code: "ELOOP" });
  });
});

describe("openNoFollowSync", () => {
  it("refuses a symlink at the name without O_NOFOLLOW, as on Windows, and opens a regular file", async (context) => {
    const root = await mkdtemp(join(tmpdir(), "rerail-no-follow-"));
    roots.push(root);
    const file = join(root, "rerail-repository-id");
    await writeFile(file, "clone-1");
    const descriptor = openNoFollowSync(file, constants.O_RDONLY, undefined, undefined);
    expect(readFileSync(descriptor, "utf8")).toBe("clone-1");
    closeSync(descriptor);
    const link = join(root, "planted-id");
    if (!linked(file, link)) {
      context.skip();
      return;
    }
    expect(() => openNoFollowSync(link, constants.O_RDONLY, undefined, undefined)).toThrow(expect.objectContaining({ code: "ELOOP" }));
  });
});
