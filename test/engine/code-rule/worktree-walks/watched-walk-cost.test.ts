/**
 * The engine service's walk of the other worktrees, with the engine's own tree watch (FSEvents, inotify or
 * ReadDirectoryChangesW): a walk lists the folders of a worktree only when something in it changed that a session could
 * be audited for. Idle turns, churn in a folder Git ignores, Git's own files and Git commands that change no file of a
 * worktree list nothing; a write lists only the worktree it is in.
 */
import { execFileSync } from "node:child_process";
import { statSync } from "node:fs";
import type * as FileSystem from "node:fs/promises";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, sep } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { nativeTreeWatch } from "../../../../src/engine/code-rule/tree-watch.js";
import { startEngineService } from "../../../../src/runtime/index.js";
import { testEmbedder } from "../../../support/hook-events.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

/** Every folder a walk listed, by the path it listed. */
const listed = vi.hoisted(() => ({ paths: [] as string[] }));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      listed.paths.push(String(args[0]));
      return await actual.opendir(...args);
    },
  };
});

const directories: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(directories.splice(0).map(async path => await removeFolder(path)));
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

const printRule = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:print", revision: 1, mode: "steer", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "print", violation: { code: "P", anchor: "witness.span", message: "No print" }, forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
});

/** `count` files in `folder`, one folder each, under `root`. */
async function files(root: string, folder: string, count: number): Promise<void> {
  for (let index = 0; index < count; index++) {
    await mkdir(join(root, folder, `d${index}`), { recursive: true });
    await writeFile(join(root, folder, `d${index}`, "f.py"), `x = ${index}\n`);
  }
}

describe("the engine service's walk of the other worktrees, with the engine's tree watch", () => {
  it.runIf(nativeTreeWatch() !== undefined)(
    "lists a worktree's folders only after a change in it that a session could be audited for",
    { timeout: 300_000 },
    async () => {
      const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-watched-walk-")));
      directories.push(root);
      const main = join(root, "main");
      await mkdir(main);
      await writeFile(join(main, ".gitignore"), "node_modules/\n");
      await files(main, "src", 20);
      git(main, "init", "--quiet", "--initial-branch=main");
      git(main, "add", ".");
      git(main, "commit", "--quiet", "-m", "seed");
      const databasePath = join(root, "rerail.sqlite");
      setUpTestRepository(main, { library: [{ kind: "code", rule: printRule }] }, databasePath);
      // The session works in `w0`, so the main checkout, which holds the repository's `.git` folder, is one of the
      // worktrees its walk watches.
      const [w0, w1, w2] = ["w0", "w1", "w2"].map(name => join(root, name)) as [string, string, string];
      for (const worktree of [w0, w1, w2]) git(main, "worktree", "add", "--quiet", "--detach", worktree);
      // Installed packages, which Git ignores, are there before the session, as in a checkout someone works in.
      await files(w1, "node_modules/installed", 5);
      const others = { main, w1, w2 };
      const listings = () => Object.fromEntries(Object.entries(others).map(([name, worktree]) =>
        [name, listed.paths.filter(path => path === worktree || path.startsWith(`${worktree}${sep}`)).length]));

      const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
      // A Windows engine service listens on a named pipe, elsewhere on a socket file.
      const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-watched-walk-${process.pid}` : join(root, "engine.sock");
      const service = await startEngineService({ socketPath, storageRoot: join(root, "sessions"), databasePath, embedder: testEmbedder });
      try {
        await service.ingest({ kind: "session-start", eventId: "s:start", session: { host: "claude-code", sessionId: "s", cwd: w0 } });
        await vi.waitFor(() => expect(opens.mock.results).toHaveLength(1), { timeout: 60_000, interval: 50 });
        await opens.mock.results[0]!.value;
        const bridge = opens.mock.contexts[0] as CodeRuleBridge;
        /** Lets the engine's notices in, then runs two walks of the other worktrees, a gap apart. */
        const walks = async (): Promise<void> => {
          await sleep(1_000);
          for (let walk = 0; walk < 2; walk++) {
            await bridge.checkpoint("turn");
            await bridge.settleWorktreeWalk();
          }
        };
        /** What `change` had the walks list, by worktree. */
        const after = async (change: () => Promise<void> | void): Promise<Record<string, number>> => {
          listed.paths.length = 0;
          await change();
          await walks();
          return listings();
        };
        // The first walks read every worktree and watch it; on Linux the next read follows the folders the first walked.
        await walks();
        await walks();

        const idle = await after(() => {});
        const ignored = await after(() => files(w1, "node_modules/added", 300));
        const gitFiles = await after(() => git(main, "gc", "--quiet"));
        // Editors and agents run `git status` all the time, and it writes the index again whenever it refreshes the files'
        // stat data. `git read-tree HEAD` writes it again with the same entries and no stat data, so the `git status`
        // after it writes it again too, whatever the clock; nothing in the worktree changes.
        const index = (): string => { const { ino, mtimeMs } = statSync(join(main, ".git", "index")); return `${ino}:${mtimeMs}`; };
        const indexBefore = index();
        const status = await after(() => {
          for (const checkout of [w1, main]) {
            git(checkout, "read-tree", "HEAD");
            git(checkout, "status", "--porcelain");
          }
        });
        expect(index()).not.toBe(indexBefore);
        const written = await after(() => writeFile(join(w2, "src", "d0", "f.py"), "print(1)\n"));
        expect({ idle, ignored, gitFiles, status }).toEqual({
          idle: { main: 0, w1: 0, w2: 0 },
          ignored: { main: 0, w1: 0, w2: 0 },
          gitFiles: { main: 0, w1: 0, w2: 0 },
          status: { main: 0, w1: 0, w2: 0 },
        });
        expect(written).toEqual({ main: 0, w1: 0, w2: expect.any(Number) });
        expect(written.w2).toBeGreaterThan(0);
      } finally {
        await service.close();
      }
    },
  );
});
