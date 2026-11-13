import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { SqlitePresentationPreferenceStore } from "../../src/runtime/presentation-preference-store.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** `path` with its drive letter in the other case: VS Code's `c:\` for a terminal's `C:\`. */
const otherDriveCase = (path: string): string => path.replace(/^[A-Za-z]:/u, drive => (drive === drive.toUpperCase() ? drive.toLowerCase() : drive.toUpperCase()));

describe("SqlitePresentationPreferenceStore", () => {
  it.runIf(process.platform === "win32")("finds a repository's preference under any spelling of its Windows path", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-presentation-")));
    cleanups.push(() => removeFolder(root));
    const store = new SqlitePresentationPreferenceStore(join(root, "rerail.sqlite"));
    cleanups.push(async () => store.close());
    store.upsert({ repository: root, host: "claude-code", presence: "silent" });
    expect(store.effective(otherDriveCase(root), "claude-code")).toBe("silent");
    expect(store.effective(root.toUpperCase(), "claude-code")).toBe("silent");
  });
});
