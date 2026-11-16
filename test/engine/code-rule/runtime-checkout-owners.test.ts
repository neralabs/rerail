import { execFileSync } from "node:child_process";
import { mkdir, realpath, writeFile } from "node:fs/promises";
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

const printRule = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:print", revision: 1, mode: "steer", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "print", violation: { code: "P", anchor: "witness.span", message: "No print" }, forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
});

/** A started repository at `main` with three linked worktrees beside it, each holding `FOLDERS`; physical paths. */
async function repositoryWithWorktrees(root: string, databasePath: string): Promise<{ main: string; worktrees: string[] }> {
  const main = join(root, "main");
  await mkdir(join(main, ...FOLDERS.at(-1)!), { recursive: true });
  await writeFile(join(main, ...FOLDERS.at(-1)!, "x.py"), "x = 1\n");
  git(main, "init", "--quiet");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  setUpTestRepository(main, { library: [{ kind: "code", rule: printRule }] }, databasePath);
  const worktrees = ["w1", "w2", "w3"].map(name => join(root, name));
  for (const worktree of worktrees) git(main, "worktree", "add", "--quiet", "--detach", worktree);
  return { main, worktrees };
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
});
