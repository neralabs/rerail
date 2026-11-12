import { execFileSync } from "node:child_process";
import { mkdir, realpath, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CodeRuleBridge } from "../../../src/engine/code-rule/code-rule-bridge.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { RerailRuntime } from "../../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../../support/hook-events.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

const temporaryDirectory = temporaryDirectories("rerail-runtime-checkout-owners-");

afterEach(() => {
  vi.restoreAllMocks();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** The folders below each checkout's top level; the deepest holds the one source. */
const FOLDERS = [["src"], ["src", "a"], ["src", "a", "b"], ["src", "a", "b", "c"]];

const SESSION = "session-1";

const printRule = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:print", revision: 1, mode: "steer", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "print", violation: { code: "P", anchor: "witness.span", message: "No print" }, forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
});

/** A rule `print(1)` does not break: a checkout set up with it alone leaves a write of it unsteered. */
const evalRule = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:eval", revision: 1, mode: "steer", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "eval", violation: { code: "E", anchor: "witness.span", message: "No eval" }, forbid: { ast: { language: "python", rule: { pattern: "eval($ARG)" } } } }],
});

/** A started repository at `path` holding `FOLDERS`, set up with `rule` (the print rule by default); returns `path`. */
async function startedRepository(path: string, databasePath: string, rule = printRule): Promise<string> {
  await mkdir(join(path, ...FOLDERS.at(-1)!), { recursive: true });
  await writeFile(join(path, ...FOLDERS.at(-1)!, "x.py"), "x = 1\n");
  git(path, "init", "--quiet");
  git(path, "add", ".");
  git(path, "commit", "--quiet", "-m", "seed");
  setUpTestRepository(path, { library: [{ kind: "code", rule }] }, databasePath);
  return path;
}

/** A started repository at `main` with three linked worktrees beside it, each holding `FOLDERS`; physical paths. */
async function repositoryWithWorktrees(root: string, databasePath: string): Promise<{ main: string; worktrees: string[] }> {
  const main = await startedRepository(join(root, "main"), databasePath);
  const worktrees = ["w1", "w2", "w3"].map(name => join(root, name));
  for (const worktree of worktrees) git(main, "worktree", "add", "--quiet", "--detach", worktree);
  return { main, worktrees };
}

/** A runtime for `main` with `SESSION` open there, after its first turn, whose end waits for the session's first walk. */
async function openedSession(root: string, main: string, databasePath: string): Promise<RerailRuntime> {
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: main, embedder: testEmbedder, autostart: false, startGate: true,
  });
  await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: SESSION, cwd: main } });
  await runtime.codeRulesOpened(SESSION);
  await runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: SESSION, text: "" });
  return runtime;
}

/** Writes `text` to `path` in a Write tool call of `sessionId`; the lines of the steers its result brings that name a file. */
async function write(runtime: RerailRuntime, sessionId: string, path: string, text: string): Promise<string[]> {
  const toolCallId = `write-${path}`;
  await runtime.ingest({ kind: "call", eventId: `call-${path}`, sessionId, toolCallId, toolName: "Write", input: { file_path: path, content: text } });
  await mkdir(join(path, ".."), { recursive: true });
  await writeFile(path, text);
  const deliveries = await runtime.ingest({ kind: "result", eventId: `result-${path}`, sessionId, toolCallId, toolName: "Write", output: "ok" });
  return deliveries.flatMap(delivery => delivery.content.split("\n").filter(line => line.includes("No print")));
}

/** Writes `text` to `path` from a shell command of `sessionId`, which the session's walk finds at the command's result. */
async function shellWrite(runtime: RerailRuntime, sessionId: string, path: string, text: string): Promise<string[]> {
  const toolCallId = `shell-${path}`;
  await runtime.ingest({ kind: "call", eventId: `call-${toolCallId}`, sessionId, toolCallId, toolName: "Bash", input: { command: "./write.sh" } });
  await writeFile(path, text);
  const deliveries = await runtime.ingest({ kind: "result", eventId: `result-${toolCallId}`, sessionId, toolCallId, toolName: "Bash", output: "" });
  return deliveries.flatMap(delivery => delivery.content.split("\n").filter(line => line.includes("No print")));
}

/** The one object of `type` among a spy's `this` values: the runtime's own. */
function instanceOf<T>(contexts: readonly unknown[], type: abstract new (...args: never[]) => T): T {
  const found = contexts.filter((context): context is T => context instanceof type);
  expect(new Set(found).size).toBe(1);
  return found[0]!;
}

describe("a runtime's checkout lookups", () => {
  it("keep one answer per checkout once its last session closes, not every folder of every worktree its walks looked up", { timeout: 60_000 }, async () => {
    const root = await realpath(await temporaryDirectory({ started: false }));
    const databasePath = join(root, "rerail.sqlite");
    const { main, worktrees } = await repositoryWithWorktrees(root, databasePath);
    const lookups = vi.spyOn(CheckoutOwners.prototype, "ownerOf");
    const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: main, embedder: testEmbedder, autostart: false, startGate: true,
    });
    const session = { host: "claude-code" as const, sessionId: "session-1", cwd: main };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      const owners = instanceOf(lookups.mock.contexts, CheckoutOwners);
      const checkouts = 1 + worktrees.length;
      // The walk of the other worktrees runs behind the session folder's own walk; once it has looked up every folder of
      // every worktree, it is waited for whole, so no lookup of its lands after the session closes.
      await vi.waitFor(() => expect(owners.cachedFolders()).toBeGreaterThanOrEqual(checkouts * (1 + FOLDERS.length)), { timeout: 30_000, interval: 20 });
      await instanceOf(opens.mock.contexts, CodeRuleBridge).settleWorktreeWalk();
      expect(owners.cachedFolders()).toBeGreaterThanOrEqual(checkouts * (1 + FOLDERS.length));

      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId });

      // The runtime serves its folder long after its sessions, so a folder each walk looked up stayed for as long: every
      // folder of every worktree, in each runtime a session had walked them from.
      expect(owners.cachedFolders()).toBe(checkouts);
    } finally {
      await runtime.dispose();
    }
  });

  it("find a worktree Git made, while a session is open, where the session had written to a plain folder", { timeout: 120_000 }, async () => {
    const root = await realpath(await temporaryDirectory({ started: false }));
    const databasePath = join(root, "rerail.sqlite");
    const { main } = await repositoryWithWorktrees(root, databasePath);
    const later = join(root, "later");
    const runtime = await openedSession(root, main, databasePath);
    try {
      const plain = await write(runtime, SESSION, join(later, ...FOLDERS.at(-1)!, "a.py"), "print(1)\n");
      await rm(later, { recursive: true, force: true });
      git(main, "worktree", "add", "--quiet", "--detach", later);

      const worktree = await write(runtime, SESSION, join(later, ...FOLDERS.at(-1)!, "b.py"), "print(1)\n");

      // The first write looked up each folder from the file up while `later` was a plain one. Those answers named no
      // checkout and stayed for as long as the session was open, so its writes in the worktree were not checked.
      expect({ plain, worktree }).toEqual({ plain: [], worktree: [expect.stringMatching(/later\/src\/a\/b\/c\/b\.py:1:1/u)] });
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: SESSION });
    } finally {
      await runtime.dispose();
    }
  });

  it("find a worktree another started repository made where the session had written to a plain folder", { timeout: 120_000 }, async () => {
    const root = await realpath(await temporaryDirectory({ started: false }));
    const databasePath = join(root, "rerail.sqlite");
    const main = await startedRepository(join(root, "main"), databasePath);
    const other = await startedRepository(join(root, "other"), databasePath);
    const later = join(root, "later");
    const runtime = await openedSession(root, main, databasePath);
    try {
      const plain = await write(runtime, SESSION, join(later, ...FOLDERS.at(-1)!, "a.py"), "print(1)\n");
      await rm(later, { recursive: true, force: true });
      git(other, "worktree", "add", "--quiet", "--detach", later);

      const worktree = await write(runtime, SESSION, join(later, ...FOLDERS.at(-1)!, "b.py"), "print(1)\n");

      // The session's repository lists no such worktree, so reading its registry dropped none of the answers the first
      // write left, and the session's writes in the other repository's worktree were not checked.
      expect({ plain, worktree }).toEqual({ plain: [], worktree: [expect.stringMatching(/later\/src\/a\/b\/c\/b\.py:1:1/u)] });
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: SESSION });
    } finally {
      await runtime.dispose();
    }
  });

  it("find a worktree whose .git file Git wrote after an event of the session found it listed", { timeout: 120_000 }, async () => {
    const root = await realpath(await temporaryDirectory({ started: false }));
    const databasePath = join(root, "rerail.sqlite");
    const main = await startedRepository(join(root, "main"), databasePath);
    const later = join(root, "later");
    const runtime = await openedSession(root, main, databasePath);
    try {
      const plain = await write(runtime, SESSION, join(later, ...FOLDERS.at(-1)!, "a.py"), "print(1)\n");
      await rm(later, { recursive: true, force: true });
      git(main, "worktree", "add", "--quiet", "--detach", later);
      // `git worktree add` lists the worktree before it writes the worktree's .git file; with the file hidden, one event
      // lands between the two.
      await rename(join(later, ".git"), join(later, ".git-pending"));
      const between = await write(runtime, SESSION, join(main, "between.py"), "x = 2\n");
      await rename(join(later, ".git-pending"), join(later, ".git"));

      const worktree = await write(runtime, SESSION, join(later, ...FOLDERS.at(-1)!, "b.py"), "print(1)\n");

      // That event saw the registry change and looked again at a folder with no .git entry yet; nothing looked again
      // once Git wrote it, as the registry did not change again.
      expect({ plain, between, worktree }).toEqual({ plain: [], between: [], worktree: [expect.stringMatching(/later\/src\/a\/b\/c\/b\.py:1:1/u)] });
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: SESSION });
    } finally {
      await runtime.dispose();
    }
  });

  it("judge a shell write by the rules of a worktree another started repository made in the session's folder over a folder its walk had found plain", { timeout: 120_000 }, async () => {
    const root = await realpath(await temporaryDirectory({ started: false }));
    const databasePath = join(root, "rerail.sqlite");
    // The session's checkout has no rule against print; the other repository has.
    const main = await startedRepository(join(root, "main"), databasePath, evalRule);
    const other = await startedRepository(join(root, "other"), databasePath);
    const nested = join(main, "nested");
    await mkdir(join(nested, ...FOLDERS.at(-1)!), { recursive: true });
    await writeFile(join(nested, ...FOLDERS.at(-1)!, "x.py"), "x = 1\n");
    const runtime = await openedSession(root, main, databasePath);
    try {
      await rm(nested, { recursive: true, force: true });
      git(other, "worktree", "add", "--quiet", "--detach", nested);

      const shell = await shellWrite(runtime, SESSION, join(nested, ...FOLDERS.at(-1)!, "x.py"), "print(1)\n");

      // The session's first walk looked up the folders of `nested` while it was plain. Those answers named the session's
      // checkout, so its walks judged the worktree's files by that checkout's rules for as long as the session was open.
      expect(shell).toEqual([expect.stringMatching(/nested\/src\/a\/b\/c\/x\.py:1:1/u)]);
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: SESSION });
    } finally {
      await runtime.dispose();
    }
  });
});
