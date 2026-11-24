import * as childProcess from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });

/** What each placement's submodule read waits for first, by the checkout it reads: nothing unless a case sets it. */
const submoduleReads = vi.hoisted(() => ({ before: (_checkout: string): Promise<void> | undefined => undefined }));
vi.mock("../../../src/engine/catalog/checkouts/declared-submodules.js", async () => {
  const actual = await vi.importActual<typeof import("../../../src/engine/catalog/checkouts/declared-submodules.js")>(
    "../../../src/engine/catalog/checkouts/declared-submodules.js",
  );
  return {
    ...actual,
    learnDeclaredSubmodules: async (...args: Parameters<typeof actual.learnDeclaredSubmodules>) => {
      await submoduleReads.before(args[0]);
      return await actual.learnDeclaredSubmodules(...args);
    },
  };
});

/** An edit batch a session's router audited, as far as a case reads it. */
interface AuditedBatch {
  readonly owner?: string | undefined;
  readonly edits: ReadonlyArray<{ readonly path: string }>;
}
/** Each batch a router audited (`auditGeneratedEdits`), with the session whose evaluator ran it. */
const audited = vi.hoisted(() => ({ calls: [] as Array<readonly [string, AuditedBatch]> }));
vi.mock("../../../src/engine/code-rule/checkout-code-rule-router.js", async () => {
  const actual = await vi.importActual<typeof import("../../../src/engine/code-rule/checkout-code-rule-router.js")>(
    "../../../src/engine/code-rule/checkout-code-rule-router.js",
  );
  const { prototype } = actual.CheckoutCodeRuleRouter;
  const audit = prototype.auditGeneratedEdits;
  prototype.auditGeneratedEdits = async function (this: typeof prototype, ...args: Parameters<typeof audit>) {
    audited.calls.push([args[0], args[1]]);
    return await audit.apply(this, args);
  };
  return actual;
});

import { codeRuleSchema } from "../../../src/engine/index.js";
import { RerailRuntime, SqliteSessionReader } from "../../../src/runtime/index.js";
import { testEmbedder } from "../../support/hook-events.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const printRule = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:print", revision: 1, mode: "steer", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "print", violation: { code: "P", anchor: "witness.span", message: "No print" }, forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
});

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

/** The null device as Rerail names it to Git (`--git-dir`) to ask it outside any repository. */
const NULL_DEVICE = process.platform === "win32" ? "nul" : "/dev/null";

/**
 * The arguments of each Git process the caller's thread waited on (`spawnSync`) while `act` ran. With `installation`
 * false, Git asked outside any repository (`--git-dir` the null device) is left out: a process reads its Git's
 * installation that way once, at its first engine read, and under GIT_CONFIG_NOSYSTEM the remote does not need it.
 */
async function blockingGitStarts(act: () => Promise<unknown>, installation: boolean): Promise<string[][]> {
  const calls = vi.mocked(childProcess.spawnSync).mock.calls;
  const before = calls.length;
  await act();
  const counted = (call: readonly unknown[]): boolean => isGit(call) && (installation || !(call[1] as string[]).includes(`--git-dir=${NULL_DEVICE}`));
  return calls.slice(before).filter(counted).map(([, args]) => (args as string[]).filter((arg, i, all) => arg !== "-c" && all[i - 1] !== "-c"));
}

describe("a repository's first events in a daemon", () => {
  describe("with six worktrees", () => {
    let started: ReturnType<typeof startedRepository>;
    // Built before the case, so the case's 5 s covers the sessions alone: built inside it, one run in 20 passed 5 s on a
    // busy Linux machine.
    beforeEach(() => {
      started = startedRepository(6);
    });
    afterEach(() => {
      vi.unstubAllEnvs();
    });

    // The first runs before any engine read of the process, as a daemon's first sessions do; the test home sets
    // GIT_CONFIG_NOSYSTEM, under which the remote names no system file.
    it.each([
      ["with the system file read, before the process read Git's installation", undefined],
      ["with GIT_CONFIG_NOSYSTEM set", "1"],
    ])("open sessions in the main checkout and six worktrees, each writing a file in another, record each one's events in order, and start no Git the daemon waits on, %s", async (_, noSystem) => {
      vi.stubEnv("GIT_CONFIG_NOSYSTEM", noSystem);
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
        })), noSystem === undefined);
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

  it("answers a session's start once its own checkout is placed, and places the other worktrees after it", { timeout: 30_000 }, async () => {
    const { root, checkouts, databasePath } = startedRepository(3);
    const [main] = checkouts as [string];
    const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: main, autostart: false, embedder: testEmbedder });
    // The pass over the other worktrees waits here, at each linked worktree, until the case lets it go.
    const pass = Promise.withResolvers<void>();
    const [learnt, held]: [string[], string[]] = [[], []];
    submoduleReads.before = async checkout => {
      if (basename(checkout).startsWith("wt-")) {
        held.push(basename(checkout));
        await pass.promise;
      }
      learnt.push(basename(checkout));
    };
    let timer: NodeJS.Timeout | undefined;
    try {
      const opening = runtime.ingest({ kind: "session-start", eventId: "own-start", session: { host: "claude-code", sessionId: "own", cwd: main } });
      const bound = new Promise<string>(resolve => (timer = setTimeout(() => resolve("still waiting after 10 s"), 10_000)));
      expect(await Promise.race([opening.then(() => "answered"), bound])).toBe("answered");
      // The pass reached the linked worktrees and waits there, the start answered.
      await vi.waitFor(() => expect(held.length).toBeGreaterThan(0), { timeout: 10_000, interval: 20 });
      pass.resolve();
      await vi.waitFor(() => expect(learnt.toSorted()).toEqual(["main", ...checkouts.map(checkout => basename(checkout))].toSorted()), { timeout: 10_000, interval: 20 });
    } finally {
      clearTimeout(timer);
      pass.resolve();
      submoduleReads.before = () => undefined;
      await runtime.dispose();
    }
  });

  // The walks join the pass that places the other worktrees; once the walk of them reads the worktree the shell
  // command ran in, the write there is that session's own (`cwd`), as a write by one of its calls is.
  it("takes a shell write in a worktree the pass is placing as the session's own whose command ran there, and starts no Git the daemon waits on", { timeout: 60_000 }, async () => {
    const { root, checkouts, databasePath } = startedRepository(2);
    const [main, worktree] = checkouts as [string, string];
    setUpTestRepository(main, { library: [{ kind: "code", rule: printRule }] }, databasePath);
    const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: main, autostart: false, embedder: testEmbedder });
    const pass = Promise.withResolvers<void>();
    const held: string[] = [];
    submoduleReads.before = async checkout => {
      if (!basename(checkout).startsWith("wt-")) return;
      held.push(basename(checkout));
      await pass.promise;
    };
    audited.calls.length = 0;
    const file = join(worktree, "shell_there.py");
    const writes = (batch: AuditedBatch): boolean => batch.edits.some(edit => edit.path.endsWith(`${basename(worktree)}/shell_there.py`));
    const call = { sessionId: "own", toolCallId: "call-shell", toolName: "Bash", input: { command: "python3 write.py" }, cwd: worktree };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "own-start", session: { host: "claude-code", sessionId: "own", cwd: main } });
      await vi.waitFor(() => expect(held.length).toBeGreaterThan(0), { timeout: 10_000, interval: 20 });
      const blocking = await blockingGitStarts(async () => {
        await runtime.ingest({ kind: "call", eventId: "own-call", ...call });
        writeFileSync(file, "print(1)\n");
        // The session folder's walk at the result joins the pass too, so the result is answered once the pass is let go.
        const result = runtime.ingest({ kind: "result", eventId: "own-result", ...call, output: "" });
        pass.resolve();
        await result;
        await vi.waitFor(() => expect(audited.calls.some(([, batch]) => writes(batch))).toBe(true), { timeout: 30_000, interval: 20 });
      }, false);
      const owners = audited.calls.filter(([, batch]) => writes(batch)).map(([sessionId, batch]) => [sessionId, batch.owner ?? "session"]);
      expect({ blocking, owners }).toEqual({ blocking: [], owners: [["own", "session"]] });
    } finally {
      pass.resolve();
      submoduleReads.before = () => undefined;
      await runtime.dispose();
    }
  });

  it("leaves none of its placement's Git running in the checkout when the session's open fails", async () => {
    const { checkouts, databasePath } = startedRepository(2);
    const checkout = checkouts[0]!;
    // Session records kept in the checkout, where a committed .gitignore folder makes the open fail.
    const storageRoot = join(checkout, ".rerail", "sessions");
    mkdirSync(join(storageRoot, ".gitignore"), { recursive: true });
    writeFileSync(join(storageRoot, ".gitignore", "README"), "committed\n");
    const runtime = new RerailRuntime({ storageRoot, databasePath, workspaceRoot: checkout, autostart: false, embedder: testEmbedder });
    const spawn = vi.mocked(childProcess.spawn);
    const before = spawn.mock.calls.length;
    try {
      await expect(runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "fails", cwd: checkout } })).rejects.toThrow();
      const started = spawn.mock.calls.slice(before).map((call, i) => ({ call, child: spawn.mock.results[before + i]?.value as childProcess.ChildProcess }));
      const running = started.filter(({ call, child }) => isGit(call) && child.exitCode === null && child.signalCode === null);
      expect(started.filter(({ call }) => isGit(call)).length).toBeGreaterThan(0);
      expect(running.map(({ call }) => call[1])).toEqual([]);
    } finally {
      await runtime.dispose();
    }
  });

  /**
   * Disposes a runtime over a repository with twelve worktrees as soon as `open`, given the runtime and the main checkout,
   * has led it to start a Git `picks` takes by its arguments. Resolves with the arguments of each Git started since `open`
   * began that still runs once dispose returned, then, `open` settled, how many Git started after the dispose began, those
   * the caller's thread waited on (`spawnSync`) apart.
   */
  async function disposedAtGit(open: (runtime: RerailRuntime, main: string) => Promise<unknown>, picks: (args: readonly string[]) => boolean): Promise<unknown> {
    const { root, checkouts, databasePath } = startedRepository(12);
    // Git reads each worktree's own config once this is set, so placing the worktrees asks Git for each.
    git(checkouts[0]!, ["config", "extensions.worktreeConfig", "true"]);
    const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: checkouts[0]!, autostart: false, embedder: testEmbedder });
    const spawn = vi.mocked(childProcess.spawn);
    const { spawn: realSpawn } = await vi.importActual<typeof childProcess>("node:child_process");
    const placing = Promise.withResolvers<void>();
    spawn.mockImplementation((...args: Parameters<typeof childProcess.spawn>) => {
      if (isGit(args) && picks(args[1])) placing.resolve();
      return realSpawn(...args);
    });
    const before = spawn.mock.calls.length;
    const opening = open(runtime, checkouts[0]!);
    try {
      await placing.promise;
    } finally {
      spawn.mockImplementation(realSpawn);
    }
    const disposing = runtime.dispose();
    const [spawned, blocked] = [spawn.mock.calls.length, vi.mocked(childProcess.spawnSync).mock.calls.length];
    await disposing;
    // Every Git the placement started has exited once dispose returns: none is left running in a checkout.
    const running = spawn.mock.calls.slice(before).filter((call, i) => {
      const child = spawn.mock.results[before + i]?.value as childProcess.ChildProcess;
      return isGit(call) && child.exitCode === null && child.signalCode === null;
    });
    await opening.catch(() => undefined);
    return {
      running: running.map(([, args]) => args),
      spawned: spawn.mock.calls.slice(spawned).filter(isGit).length,
      blocked: vi.mocked(childProcess.spawnSync).mock.calls.slice(blocked).filter(isGit).length,
    };
  }

  it("starts no Git once it is disposed while a session's checkout is being placed", async () => {
    const opened = (runtime: RerailRuntime, main: string): Promise<unknown> =>
      runtime.ingest({ kind: "session-start", eventId: "late-start", session: { host: "claude-code", sessionId: "late", cwd: main } });
    expect(await disposedAtGit(opened, () => true)).toEqual({ running: [], spawned: 0, blocked: 0 });
  });

  // The pass over the other worktrees waits at each linked worktree until the session's open has answered and the session
  // has ended, so the Git disposed at is one the pass started in a linked worktree after both. With no session left to
  // close, dispose returns only as soon as it has waited for the pass: a session's close outlasts a stopped Git's exit.
  it("ends the pass over the other worktrees, its Git exited, and starts no Git once it is disposed while that pass runs after a session's open and end", { timeout: 30_000 }, async () => {
    const pass = Promise.withResolvers<void>();
    submoduleReads.before = async checkout => {
      if (basename(checkout).startsWith("wt-")) await pass.promise;
    };
    let ended = false;
    const inWorktree = (args: readonly string[]): boolean => ended && args.some((arg, i) => args[i - 1] === "-C" && basename(arg).startsWith("wt-"));
    const opened = async (runtime: RerailRuntime, main: string): Promise<void> => {
      const session = { host: "claude-code", sessionId: "own", cwd: main } as const;
      await runtime.ingest({ kind: "session-start", eventId: "own-start", session });
      await runtime.ingest({ kind: "session-end", eventId: "own-end", sessionId: "own", session });
      ended = true;
      pass.resolve();
    };
    try {
      expect(await disposedAtGit(opened, inWorktree)).toEqual({ running: [], spawned: 0, blocked: 0 });
    } finally {
      pass.resolve();
      submoduleReads.before = () => undefined;
    }
  });
});
