import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs/promises";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { startEngineService } from "../../../../src/runtime/index.js";
import { testEmbedder } from "../../../support/hook-events.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

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

const directories: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  reads.clear();
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

const printRule = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:print", revision: 1, mode: "steer", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "print", violation: { code: "P", anchor: "witness.span", message: "No print" }, forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
});

describe("the engine service and the other worktrees of a repository", () => {
  it("reads a worktree's files once for all the live sessions watching it, each in a worktree of its own", { timeout: 60_000 }, async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-shared-walk-")));
    directories.push(root);
    const databasePath = join(root, "rerail.sqlite");
    const main = join(root, "main");
    await mkdir(join(main, "src"), { recursive: true });
    await writeFile(join(main, "src", "x.py"), "x = 1\n");
    git(main, "init", "--quiet");
    git(main, "add", ".");
    git(main, "commit", "--quiet", "-m", "seed");
    setUpTestRepository(main, { library: [{ kind: "code", rule: printRule }] }, databasePath);
    const worktrees = ["w1", "w2", "w3", "w4"].map(name => join(root, name));
    for (const worktree of worktrees) git(main, "worktree", "add", "--quiet", "--detach", worktree);
    const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-shared-walk-${process.pid}` : join(root, "engine.sock");
    const service = await startEngineService({ socketPath, storageRoot: join(root, "sessions"), databasePath, embedder: testEmbedder });
    const sessions = worktrees.slice(0, 3).map((cwd, index) => ({ host: "claude-code" as const, sessionId: `session-${index + 1}`, cwd }));
    try {
      for (const session of sessions) await service.ingest({ kind: "session-start", eventId: `${session.sessionId}:start`, session });
      await vi.waitFor(() => expect(opens.mock.results).toHaveLength(sessions.length), { timeout: 30_000, interval: 20 });
      await Promise.all(opens.mock.results.map(result => result.value));
      // A turn waits for the session folder's walk, which then asks for a walk of the other worktrees that runs behind it.
      for (const bridge of opens.mock.contexts as CodeRuleBridge[]) {
        await bridge.checkpoint("turn");
        await bridge.settleWorktreeWalk();
      }

      // `w4` is no session's folder, so every session watches it: each session's own walk of it read its file once per
      // session, and the daemon held an observation of it per session.
      expect(reads.get(join(worktrees[3]!, "src", "x.py"))).toBe(1);
    } finally {
      await service.close();
    }
  });
});
