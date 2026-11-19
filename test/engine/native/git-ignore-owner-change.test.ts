import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import type * as GitAccessModule from "../../../src/engine/native/git-access.js";
import type { NativeGitAccess, NativeGitIgnoreMatcher } from "../../../src/engine/native/native.js";
import { NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** The owner `nativeGitAccess` hands out: the test swaps it, as a process does once Git answers an ask at last. */
const owner = vi.hoisted(() => ({ current: undefined as NativeGitAccess | undefined }));
vi.mock("../../../src/engine/native/git-access.js", async importOriginal => ({
  ...(await importOriginal<typeof GitAccessModule>()),
  nativeGitAccess: () => owner.current!,
}));

function access(open: () => NativeGitIgnoreMatcher | null): NativeGitAccess & { openGitIgnoreMatcher: Mock<(root: string) => NativeGitIgnoreMatcher | null> } {
  return { systemConfig: null, installationConfig: null, openGitIgnoreMatcher: vi.fn(open) };
}

const roots: string[] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await removeFolder(root);
});

describe("a lane's Git ignore matcher and the process's Git owner", () => {
  it("opens the matcher again once the owner changes, even where the old owner's open was refused", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-owner-change-"));
    roots.push(root);
    // Opened while Git had not said which system config it reads: no system safe.directory, so refused.
    const fallback = access(() => { throw new Error("detected dubious ownership"); });
    const answered = access(() => ({ ignored: (path: string) => path.endsWith(".log"), ignoredBy: () => null, sourcesUnchanged: () => true }));
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog([]), {
      workspaceRoot: root,
      steerDelivery: { requestSteer: async () => undefined },
      followGitignore: true,
    });
    try {
      owner.current = fallback;
      expect([enforcer.ignoresPath("a.log", false), enforcer.ignoresPath("b.log", false)]).toEqual([false, false]);
      expect(fallback.openGitIgnoreMatcher).toHaveBeenCalledTimes(1);
      owner.current = answered;
      expect([enforcer.ignoresPath("a.log", false), enforcer.ignoresPath("c.py", false)]).toEqual([true, false]);
      expect(answered.openGitIgnoreMatcher).toHaveBeenCalledTimes(1);
    } finally {
      stderr.mockRestore();
      await enforcer.close();
    }
  });
});
