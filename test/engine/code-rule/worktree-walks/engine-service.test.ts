import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs/promises";
import { lstat, mkdir, mkdtemp, realpath, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { startEngineService } from "../../../../src/runtime/index.js";
import { testEmbedder } from "../../../support/hook-events.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

/**
 * How many times each file was read whole through `node:fs/promises`, by the path it was read at, and the listings held
 * back until a test lets them go, by folder.
 */
const counted = vi.hoisted(() => ({ reads: new Map<string, number>(), held: new Map<string, () => Promise<void>>() }));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    readFile: async (...args: Parameters<typeof actual.readFile>) => {
      const path = String(args[0]);
      counted.reads.set(path, (counted.reads.get(path) ?? 0) + 1);
      return await actual.readFile(...args);
    },
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      const hold = counted.held.get(String(args[0]));
      counted.held.delete(String(args[0]));
      await hold?.();
      return await actual.opendir(...args);
    },
  };
});
const { reads } = counted;

const directories: string[] = [];
const releases: Array<() => void> = [];
afterEach(async () => {
  releases.splice(0).forEach(release => release());
  counted.held.clear();
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

/** A set-up repository at `<root>/main` holding `src/x.py`, with worktrees named `names` beside it; physical paths. */
function repository(root: string, databasePath: string, names: readonly string[]): { main: string; worktrees: string[] } {
  const main = join(root, "main");
  git(main, "init", "--quiet");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  setUpTestRepository(main, { library: [{ kind: "code", rule: printRule }] }, databasePath);
  const worktrees = names.map(name => join(root, name));
  for (const worktree of worktrees) git(main, "worktree", "add", "--quiet", "--detach", worktree);
  return { main, worktrees };
}

async function folder(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-shared-walk-")));
  directories.push(root);
  await mkdir(join(root, "main", "src"), { recursive: true });
  await writeFile(join(root, "main", "src", "x.py"), "x = 1\n");
  return root;
}

function socketPathIn(root: string, name: string): string {
  return process.platform === "win32" ? `\\\\.\\pipe\\rerail-${name}-${process.pid}` : join(root, "engine.sock");
}

/** Waits until the filesystem clock has passed the stamp of `path`, so a walk after it reads the file as it stands. */
async function awaitClockAfter(path: string): Promise<void> {
  const info = await lstat(path);
  const since = Math.max(info.mtimeMs, info.ctimeMs);
  const probe = await mkdtemp(join(tmpdir(), "rerail-shared-walk-clock-"));
  directories.push(probe);
  for (;;) {
    await writeFile(join(probe, "tick"), "");
    const tick = await lstat(join(probe, "tick"));
    if (Math.max(tick.mtimeMs, tick.ctimeMs) > since) return;
  }
}

/**
 * One daemon's life: an engine service on `databasePath` with a session in `cwd`, which takes a turn and the walk of the
 * other worktrees after it, then whatever `then` does with its bridge, before the service closes.
 */
async function serviceLife(
  root: string,
  databasePath: string,
  { name, cwd, then }: { name: string; cwd: string; then?: (bridge: CodeRuleBridge) => Promise<void> },
): Promise<void> {
  const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
  opens.mockClear();
  const service = await startEngineService({ socketPath: socketPathIn(root, name), storageRoot: join(root, "sessions"), databasePath, embedder: testEmbedder });
  try {
    const session = { host: "claude-code" as const, sessionId: name, cwd };
    await service.ingest({ kind: "session-start", eventId: `${name}:start`, session });
    await vi.waitFor(() => expect(opens.mock.results).toHaveLength(1), { timeout: 30_000, interval: 20 });
    await opens.mock.results[0]!.value;
    const bridge = opens.mock.contexts[0] as CodeRuleBridge;
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    await then?.(bridge);
  } finally {
    await service.close();
  }
}

describe("the engine service and the other worktrees of a repository", () => {
  it("reads a worktree's files once for all the live sessions watching it, each in a worktree of its own", { timeout: 60_000 }, async () => {
    const root = await folder();
    const databasePath = join(root, "rerail.sqlite");
    const { worktrees } = repository(root, databasePath, ["w1", "w2", "w3", "w4"]);
    const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
    const service = await startEngineService({ socketPath: socketPathIn(root, "shared-walk"), storageRoot: join(root, "sessions"), databasePath, embedder: testEmbedder });
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

  it("records a shell write in another worktree of one repository while a walk of another repository's worktrees is held", { timeout: 60_000 }, async () => {
    const [held, quick] = [await folder(), await folder()];
    const databasePath = join(held, "rerail.sqlite");
    const slow = repository(held, databasePath, ["w1"]);
    const fast = repository(quick, databasePath, ["w1"]);
    await mkdir(join(slow.worktrees[0]!, "deep"));
    const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
    const audits = vi.spyOn(CheckoutCodeRuleRouter.prototype, "auditGeneratedEdits");
    const service = await startEngineService({ socketPath: socketPathIn(held, "two-repositories"), storageRoot: join(held, "sessions"), databasePath, embedder: testEmbedder });
    try {
      // One session in each repository's main checkout, opened one after the other so each bridge is known.
      for (const [index, cwd] of [slow.main, fast.main].entries()) {
        const session = { host: "claude-code" as const, sessionId: `session-${index + 1}`, cwd };
        await service.ingest({ kind: "session-start", eventId: `${session.sessionId}:start`, session });
        await vi.waitFor(() => expect(opens.mock.results).toHaveLength(index + 1), { timeout: 30_000, interval: 20 });
        await opens.mock.results[index]!.value;
      }
      const [slowBridge, fastBridge] = opens.mock.contexts as CodeRuleBridge[];
      for (const bridge of [slowBridge!, fastBridge!]) {
        await bridge.checkpoint("turn");
        await bridge.settleWorktreeWalk();
      }
      const entered = Promise.withResolvers<void>();
      const released = Promise.withResolvers<void>();
      releases.push(released.resolve);
      counted.held.set(join(slow.worktrees[0]!, "deep"), async () => {
        entered.resolve();
        await released.promise;
      });
      await slowBridge!.checkpoint("turn");
      await entered.promise;
      await fastBridge!.begin("shell-1", "shell", { command: "./run.sh" });
      await writeFile(join(fast.worktrees[0]!, "src", "x.py"), "x = 2\n");
      await fastBridge!.finish("shell-1");
      await fastBridge!.checkpoint("turn");
      // The walk gap is 5 s, so a walk of the second repository's worktree is due within it.
      const settled = await Promise.race([
        fastBridge!.settleWorktreeWalk().then(() => true),
        new Promise<boolean>(wake => { setTimeout(() => wake(false), 20_000); }),
      ]);
      const logged = audits.mock.calls.filter(([, batch]) => batch.mode === "log").flatMap(([, batch]) => batch.edits.map(edit => edit.path));
      expect({ settled, logged }).toEqual({ settled: true, logged: ["../w1/src/x.py"] });
    } finally {
      releases.splice(0).forEach(release => release());
      await service.close();
    }
  });
});

describe("the engine service started again", () => {
  /** A repository whose checkouts each hold `SOURCES`, with three worktrees; the filesystem clock is past every stamp. */
  async function restartRepository(): Promise<{ root: string; databasePath: string; main: string; worktrees: string[] }> {
    const root = await folder();
    await mkdir(join(root, "main", "lib"));
    await writeFile(join(root, "main", "src", "y.py"), "y = 1\n");
    await writeFile(join(root, "main", "lib", "z.py"), "z = 1\n");
    await writeFile(join(root, "main", "README.md"), "# notes\n");
    const databasePath = join(root, "rerail.sqlite");
    const { main, worktrees } = repository(root, databasePath, ["w1", "w2", "w3"]);
    await awaitClockAfter(join(worktrees.at(-1)!, "README.md"));
    return { root, databasePath, main, worktrees };
  }
  const SOURCES = ["src/x.py", "src/y.py", "lib/z.py", "README.md"];

  it("reads none of the checkouts' files whose stamps held while no daemon ran", { timeout: 90_000 }, async () => {
    const { root, databasePath, main, worktrees } = await restartRepository();
    const files = [main, ...worktrees].flatMap(checkout => SOURCES.map(path => join(checkout, ...path.split("/"))));
    await serviceLife(root, databasePath, { name: "before-restart", cwd: worktrees[0]! });
    const readBefore = files.filter(file => reads.has(file));
    reads.clear();
    await serviceLife(root, databasePath, { name: "after-restart", cwd: worktrees[0]! });
    // The session folder's walk read `w1`, the walk of the other worktrees `main`, `w2` and `w3`: each source a rule
    // can match, once. Nothing changed while no daemon ran, so the walks after the restart read none of them again.
    expect(readBefore.length).toBeGreaterThanOrEqual(12);
    expect(files.filter(file => reads.has(file)).map(file => relative(root, file).split(sep).join("/"))).toEqual([]);
  });

  it("reads again a file rewritten while no daemon ran, its modification time put back, and records a later write from what it read", { timeout: 90_000 }, async () => {
    const { root, databasePath, worktrees } = await restartRepository();
    const path = join(worktrees[1]!, "src", "x.py");
    // Text `HEAD` does not hold: a write back to `HEAD`'s text would be Git's (`git restore`), and recorded by no walk.
    await writeFile(path, "x = 2\n");
    // A modification time in whole seconds, which `utimes` can put back exactly.
    const modified = Math.floor(Date.now() / 1000) - 60;
    await utimes(path, modified, modified);
    await awaitClockAfter(path);
    await serviceLife(root, databasePath, { name: "before-rewrite", cwd: worktrees[0]! });
    // The same size, with the times a copy that keeps them would leave: only the change time moves.
    const before = await lstat(path);
    await writeFile(path, "x = 9\n");
    await utimes(path, modified, modified);
    const after = await lstat(path);
    expect([after.ino, after.size, after.mtimeMs]).toEqual([before.ino, before.size, before.mtimeMs]);
    reads.clear();
    const audits = vi.spyOn(CheckoutCodeRuleRouter.prototype, "auditGeneratedEdits");
    let readByFirstWalk: number | undefined;
    await serviceLife(root, databasePath, {
      name: "after-rewrite", cwd: worktrees[0]!,
      then: async bridge => {
        readByFirstWalk = reads.get(path);
        // Back to the text the last daemon read: a change from what this one read, which a kept digest would hide.
        await bridge.begin("shell-1", "shell", { command: "./run.sh" });
        await writeFile(path, "x = 2\n");
        await bridge.finish("shell-1");
        await bridge.checkpoint("turn");
        await bridge.settleWorktreeWalk();
      },
    });
    const logged = audits.mock.calls.filter(([, batch]) => batch.mode === "log").flatMap(([, batch]) => batch.edits.map(edit => [edit.path, edit.generatedText]));
    expect({ readByFirstWalk, logged }).toEqual({ readByFirstWalk: 1, logged: [["../w2/src/x.py", "x = 2\n"]] });
  });
});
