/**
 * What a walk of the session folder works out, on the daemon's thread, to name each file it finds as the auditor names
 * it (relative to the session folder, `/`-separated): each `path.relative` naming a file of a checkout of 200 folders,
 * over the walk at a turn's end, and the paths the auditor is asked about.
 */
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import type * as Path from "node:path";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/** While `on`: the path each `path.relative` was asked to reach. */
const arithmetic = vi.hoisted(() => ({ on: false, reached: [] as string[] }));
vi.mock("node:path", async importOriginal => {
  const actual = await importOriginal<typeof Path>();
  return {
    ...actual,
    relative: (from: string, to: string): string => {
      if (arithmetic.on) arithmetic.reached.push(to);
      return actual.relative(from, to);
    },
  };
});

import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  arithmetic.on = false;
  vi.restoreAllMocks();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** The tree's folders, d0 to d9 each holding e0 to e18: 200 folders, each holding one Python file. */
const FOLDERS = Array.from({ length: 10 }, (_, top) => [`d${top}`, ...Array.from({ length: 19 }, (_, leaf) => `d${top}/e${leaf}`)]).flat();
const FILES = FOLDERS.map(folder => `${folder}/x.py`);

function response(batch: CodeRuleEditBatch) {
  return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
}

describe("a walk of a checkout of many folders", () => {
  it("names each file it finds without working out its path from the session folder's again", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-session-paths-")));
    cleanups.push(() => removeFolder(root));
    for (const folder of FOLDERS) {
      await mkdir(join(root, folder), { recursive: true });
      await writeFile(join(root, folder, "x.py"), "x = 1\n");
    }
    git(root, "init", "--quiet", "-b", "main");
    git(root, "add", "-A");
    git(root, "commit", "--quiet", "-m", "seed");
    // Each path the walk asks Git's ignore question of: every file it finds, as the auditor names it.
    const asked: string[] = [];
    const auditor: CodeRuleAuditor = {
      openWorkspace: async () => undefined,
      acceptsPath: path => path.endsWith(".py"),
      ignoresPath: (path, directory) => {
        if (!directory) asked.push(path);
        return false;
      },
      auditGeneratedEdits: async (_sessionId, batch) => response(batch),
      close: async () => undefined,
    };
    const bridge = new CodeRuleBridge(root, auditor, { discoveryWaitMs: Infinity });
    cleanups.push(() => bridge.close());
    await bridge.openWorkspace("paths");
    await bridge.settleWalk();
    asked.length = 0;
    arithmetic.reached = [];

    arithmetic.on = true;
    try {
      await bridge.checkpoint("turn");
    } finally {
      arithmetic.on = false;
    }

    const files = new Set(FILES.map(path => join(root, path)));
    // `path.relative` from the session folder to each file it found: 200 a walk, each resolving both paths again.
    expect(arithmetic.reached.filter(path => files.has(path))).toHaveLength(0);
    expect(asked.toSorted()).toEqual(FILES.toSorted());
  }, 60_000);
});
