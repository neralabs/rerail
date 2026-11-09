// npm runs in a scratch folder Windows may still hold when it ends (npm's cache in it): the command's result stands when
// removing that folder fails. node:fs/promises is spied on so the test can make the removal fail.
import { rm } from "node:fs/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import { outsideAnyProject } from "../../src/runtime/npm.js";

vi.mock("node:fs/promises", { spy: true });

const left: string[] = [];
afterEach(async () => {
  vi.mocked(rm).mockReset();
  await Promise.all(left.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

describe("npm outside any project", () => {
  it("answers with the operation's result when its scratch folder cannot be removed", async () => {
    vi.mocked(rm).mockImplementationOnce(async (path) => {
      left.push(String(path));
      throw Object.assign(new Error(`EBUSY: resource busy or locked, rmdir '${String(path)}'`), { code: "EBUSY" });
    });
    await expect(outsideAnyProject(async () => "done")).resolves.toBe("done");
    expect(left).toHaveLength(1);
  });
});
