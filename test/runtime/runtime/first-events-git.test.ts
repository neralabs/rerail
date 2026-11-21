import * as childProcess from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });

import { RerailRuntime, SqliteSessionReader } from "../../../src/runtime/index.js";
import { testEmbedder } from "../../support/hook-events.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const folders: string[] = [];
afterEach(() => {
  for (const folder of folders.splice(0)) removeFolderSync(folder);
});

function git(cwd: string, args: readonly string[]): void {
  childProcess.execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], { cwd, stdio: "ignore" });
}

function repository(path: string): void {
  mkdirSync(path, { recursive: true });
  git(path, ["init", "--quiet", "-b", "main"]);
  writeFileSync(join(path, "app.py"), "def total(value):\n    return value * 2\n");
  git(path, ["add", "app.py"]);
  git(path, ["commit", "--quiet", "-m", "one"]);
}

/**
 * A started repository with no remote, a declared submodule, and `count` linked worktrees beside its main checkout,
 * and the database it is recorded in.
 */
function startedRepository(count: number): { root: string; checkouts: string[]; databasePath: string } {
  const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-first-events-")));
  folders.push(root);
  repository(join(root, "library"));
  const main = join(root, "main");
  repository(main);
  git(main, ["submodule", "add", "--quiet", join(root, "library"), "lib"]);
  git(main, ["commit", "--quiet", "-m", "lib"]);
  const worktrees = Array.from({ length: count }, (_, i) => join(root, `wt-${i}`));
  for (const [i, worktree] of worktrees.entries()) git(main, ["worktree", "add", "--quiet", "-b", `wt-${i}`, worktree]);
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(main, databasePath);
  return { root, checkouts: [main, ...worktrees], databasePath };
}

const isGit = ([file]: readonly unknown[]): boolean => /^git(\.exe)?$/iu.test(basename(String(file)));

/** The arguments of each Git process the caller's thread waited on (`spawnSync`) while `act` ran. */
async function blockingGitStarts(act: () => Promise<unknown>): Promise<string[][]> {
  const calls = vi.mocked(childProcess.spawnSync).mock.calls;
  const before = calls.length;
  await act();
  return calls.slice(before).filter(isGit).map(([, args]) => (args as string[]).filter((arg, i, all) => arg !== "-c" && all[i - 1] !== "-c"));
}

describe("a repository's first events in a daemon", () => {
  describe("with six worktrees", () => {
    let started: ReturnType<typeof startedRepository>;
    // Built before the case, so the case's 5 s covers the sessions alone: built inside it, one run in 20 passed 5 s on a
    // busy Linux machine.
    beforeEach(() => {
      started = startedRepository(6);
    });

    it("open sessions in the main checkout and six worktrees, each writing a file in another, record each one's events in order, and start no Git the daemon waits on", async () => {
      const { root, checkouts, databasePath } = started;
      const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: checkouts[0]!, autostart: false, embedder: testEmbedder });
      const reader = new SqliteSessionReader(databasePath);
      // Eight sessions: two in the main checkout, one in each worktree; each writes a file in another worktree, which the
      // router finds the checkout of, and that checkout's submodules.
      const sessions = [checkouts[0]!, ...checkouts].slice(0, 8).map((cwd, i) => ({ sessionId: `first-${i}`, cwd, file: join(checkouts[(i + 1) % 7]!, `written_${i}.py`) }));
      try {
        const blocking = await blockingGitStarts(() => Promise.all(sessions.map(async ({ sessionId, cwd, file }) => {
          const call = { sessionId, toolCallId: `call-${sessionId}`, toolName: "Write", input: { file_path: file, content: "x = 1\n" } };
          await Promise.all([
            runtime.ingest({ kind: "session-start", eventId: `${sessionId}-start`, session: { host: "claude-code", sessionId, cwd } }),
            // Sent before the start is answered: it waits for its session, in order.
            runtime.ingest({ kind: "call", eventId: `${sessionId}-call`, ...call }),
          ]);
          writeFileSync(file, "x = 1\n");
          await runtime.ingest({ kind: "result", eventId: `${sessionId}-result`, ...call, output: "" });
        })));
        expect(blocking).toEqual([]);
        for (const { sessionId } of sessions) {
          const kinds = reader.events(sessionId).map(event => event.kind).filter(kind => ["session-start", "call", "result"].includes(kind));
          expect(kinds).toEqual(["session-start", "call", "result"]);
        }
      } finally {
        await runtime.dispose();
        reader.close();
      }
    });
  });

  it("starts no Git once it is disposed while a session's checkout is being placed", async () => {
    const { root, checkouts, databasePath } = startedRepository(12);
    // Git reads each worktree's own config once this is set, so placing the worktrees asks Git for each.
    git(checkouts[0]!, ["config", "extensions.worktreeConfig", "true"]);
    const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: checkouts[0]!, autostart: false, embedder: testEmbedder });
    const spawn = vi.mocked(childProcess.spawn);
    const { spawn: realSpawn } = await vi.importActual<typeof childProcess>("node:child_process");
    const placing = Promise.withResolvers<void>();
    spawn.mockImplementation((...args: Parameters<typeof childProcess.spawn>) => {
      if (isGit(args)) placing.resolve();
      return realSpawn(...args);
    });
    const opening = runtime.ingest({ kind: "session-start", eventId: "late-start", session: { host: "claude-code", sessionId: "late", cwd: checkouts[0]! } });
    try {
      await placing.promise;
    } finally {
      spawn.mockImplementation(realSpawn);
    }
    const disposing = runtime.dispose();
    const [spawned, blocked] = [spawn.mock.calls.length, vi.mocked(childProcess.spawnSync).mock.calls.length];
    await disposing;
    await opening.catch(() => undefined);
    expect({
      spawned: spawn.mock.calls.slice(spawned).filter(isGit).length,
      blocked: vi.mocked(childProcess.spawnSync).mock.calls.slice(blocked).filter(isGit).length,
    }).toEqual({ spawned: 0, blocked: 0 });
  });
});
