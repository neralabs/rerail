import * as fs from "node:fs/promises";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("node:fs/promises", { spy: true });

import { writeTextFile } from "../../src/setup/harness-registration.js";

const roots: string[] = [];
const platform = Object.getOwnPropertyDescriptor(process, "platform")!;
afterEach(async () => {
  Object.defineProperty(process, "platform", platform);
  vi.restoreAllMocks();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

describe("writeTextFile on Windows", () => {
  it("replaces a coding agent's settings without removing them first, retrying a rename a scan refuses", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-write-text-"));
    roots.push(root);
    const settings = join(root, "settings.json");
    await writeFile(settings, "{\"old\":true}\n");
    Object.defineProperty(process, "platform", { ...platform, value: "win32" });
    vi.mocked(fs.rename).mockRejectedValueOnce(Object.assign(new Error("EPERM: operation not permitted, rename"), { code: "EPERM" }));

    await writeTextFile(settings, "{\"new\":true}\n", { exists: true, mode: 0o600 }, false);

    expect(await readFile(settings, "utf8")).toBe("{\"new\":true}\n");
    expect(vi.mocked(fs.rm)).not.toHaveBeenCalledWith(settings, expect.anything());
  });
});
