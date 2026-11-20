import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { SqliteSessionReader, SqliteSessionStore } from "../../../src/runtime/session-store.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { withExperiments } from "../../support/environment/experiments.js";

/** The paths read through symlinks, to tell whether the HUD looked for the session's checkout. */
const physicalPaths = vi.hoisted(() => [] as string[]);
vi.mock("../../../src/engine/catalog/checkout-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../../src/engine/catalog/checkout-control.js")>("../../../src/engine/catalog/checkout-control.js");
  return {
    ...actual,
    physicalPath: (path: string, platform?: NodeJS.Platform) => {
      physicalPaths.push(path);
      return actual.physicalPath(path, platform);
    },
  };
});

/** The paths the HUD of a session started in a folder read through symlinks. */
async function hudReads(): Promise<string[]> {
  const root = await mkdtemp(join(tmpdir(), "rerail-session-hud-"));
  const store = new SqliteSessionStore(join(root, "rerail.sqlite"));
  const reader = new SqliteSessionReader(join(root, "rerail.sqlite"));
  try {
    store.openSession({ host: "claude-code", sessionId: "session", cwd: root }, Date.now());
    physicalPaths.length = 0;
    expect(reader.hud("session").mergeAssist).toBeUndefined();
    return [...physicalPaths];
  } finally {
    store.close();
    reader.close();
    await removeFolder(root);
  }
}

describe("the HUD's Merge Assist segment", () => {
  it("looks for no checkout while the experiment is off", async () => {
    expect(await hudReads()).toEqual([]);
  });

  describe("with the experiment on", () => {
    withExperiments("MERGE_ASSIST");

    it("looks for the session's checkout", async () => {
      expect(await hudReads()).not.toEqual([]);
    });
  });
});
