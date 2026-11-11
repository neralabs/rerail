import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs/promises";
import { lstat, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { WorktreeWalks } from "../../../../src/engine/code-rule/worktree-walks.js";

/** How many times each file was read whole through `node:fs/promises`, by the path it was read at. */
const reads = vi.hoisted(() => new Map<string, number>());
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    readFile: async (...args: Parameters<typeof actual.readFile>) => {
      const path = String(args[0]);
      reads.set(path, (reads.get(path) ?? 0) + 1);
      return await actual.readFile(...args);
    },
  };
});

const bridges: CodeRuleBridge[] = [];
const directories: string[] = [];
afterEach(async () => {
  await Promise.all(bridges.splice(0).map(bridge => bridge.close()));
  reads.clear();
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** Waits until the filesystem clock has passed the stamp of `path`, so a later write is told from it. */
async function awaitClockAfter(path: string): Promise<void> {
  const info = await lstat(path);
  const since = Math.max(info.mtimeMs, info.ctimeMs);
  const probe = await mkdtemp(join(tmpdir(), "rerail-walks-clock-"));
  directories.push(probe);
  for (;;) {
    await writeFile(join(probe, "tick"), "");
    const tick = await lstat(join(probe, "tick"));
    if (Math.max(tick.mtimeMs, tick.ctimeMs) > since) return;
  }
}

/** A repository at `main` holding `src/a.py`, with worktrees `w1` to `w3` beside it; physical paths. */
async function repository(): Promise<{ main: string; worktrees: string[]; checkouts: string[] }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-walks-")));
  directories.push(root);
  const main = join(root, "main");
  await mkdir(join(main, "src"), { recursive: true });
  await writeFile(join(main, "src", "a.py"), "old\n");
  git(main, "init", "--quiet");
  // The worktrees hold the blob's bytes as they are, whatever line breaks the host's Git writes.
  git(main, "config", "core.autocrlf", "false");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  const worktrees = ["w1", "w2", "w3"].map(name => join(root, name));
  for (const worktree of worktrees) git(main, "worktree", "add", "--quiet", "--detach", worktree);
  await awaitClockAfter(join(worktrees[2]!, "src", "a.py"));
  return { main, worktrees, checkouts: [main, ...worktrees] };
}

/** A session in `folder`, one of `checkouts`, watching the others through `walks`, and its auditor's calls. */
async function session(folder: string, checkouts: readonly string[], walks: WorktreeWalks) {
  const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => ({
    revision: batch.revision, disposition: "continue" as const, violations: [], failures: [],
  }));
  const auditor: CodeRuleAuditor = {
    openWorkspace: async () => undefined,
    acceptsPath: () => true,
    auditGeneratedEdits,
    close: async () => undefined,
    sourceRoot: path => checkouts.find(checkout => resolve(folder, path).startsWith(`${checkout}${sep}`)),
    worktreeRoots: () => [...checkouts],
  };
  const bridge = new CodeRuleBridge(folder, auditor, { worktreeWalks: walks });
  bridges.push(bridge);
  await bridge.openWorkspace(`session-${bridges.length}`);
  await bridge.checkpoint("turn");
  await bridge.settleWorktreeWalk();
  return { bridge, auditGeneratedEdits };
}

describe("WorktreeWalks", () => {
  it("holds one observation of each file of the worktrees its sessions watch, however many sessions watch them", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    await session(worktrees[0]!, checkouts, walks);
    expect(walks.observations()).toBe(3);
    await session(worktrees[1]!, checkouts, walks);
    // Two sessions watch `main` and `w3`: each of their files is held once, and read once.
    expect(walks.observations()).toBe(4);
    expect(reads.get(join(worktrees[2]!, "src", "a.py"))).toBe(1);
  });

  it("hands a shell write in a worktree to every session watching it, read once, and recorded by each in Log", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    const path = join(worktrees[2]!, "src", "a.py");
    await first.bridge.begin("shell-1", "shell", { command: "./write.sh" });
    await writeFile(path, "old\nmine\n");
    await first.bridge.finish("shell-1");
    await first.bridge.settleWorktreeWalk();
    await second.bridge.settleWorktreeWalk();
    for (const { auditGeneratedEdits } of [first, second]) {
      expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
        [[{ path: "../w3/src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" }], "log"],
      ]);
    }
    expect(reads.get(path)).toBe(2);
  });

  it("reads through another session once the one it read through closes, and drops what no session watches", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    await first.bridge.close();
    const path = join(worktrees[2]!, "src", "a.py");
    await writeFile(path, "old\nmine\n");
    await second.bridge.checkpoint("turn");
    await second.bridge.settleWorktreeWalk();
    expect(second.auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => edit.path))).toEqual(["../w3/src/a.py"]);
    await second.bridge.close();
    expect(walks.observations()).toBe(0);
  });
});
