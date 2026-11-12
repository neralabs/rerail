import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/** Git processes Rerail started (`spawnSync`, `execFile` or `spawn`); the test's own `git` uses `execFileSync`. */
const gitProcesses = vi.hoisted(() => ({ started: 0 }));
vi.mock("node:child_process", async importOriginal => {
  const original = await importOriginal<typeof import("node:child_process")>();
  const counted = <F extends (file: string, ...rest: never[]) => unknown>(start: F): F => ((file: string, ...rest: never[]) => {
    if (/(^|[\\/])git(\.exe)?$/iu.test(file)) gitProcesses.started += 1;
    return start(file, ...rest);
  }) as F;
  return { ...original, spawnSync: counted(original.spawnSync), execFile: counted(original.execFile), spawn: counted(original.spawn) };
});

/** Reads of the installed packs' catalog, each a check of the packs' files on disk. */
const ruleCatalogReads = vi.hoisted(() => ({ count: 0 }));
vi.mock("../../../src/engine/catalog/rule-catalog.js", async importOriginal => {
  const original = await importOriginal<typeof import("../../../src/engine/catalog/rule-catalog.js")>();
  return {
    ...original,
    loadRuleCatalog: (...parameters: Parameters<typeof original.loadRuleCatalog>) => {
      ruleCatalogReads.count += 1;
      return original.loadRuleCatalog(...parameters);
    },
  };
});

import Database from "better-sqlite3";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { setRuleSetting } from "../../../src/engine/catalog/rule-configuration.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { RerailRuntime, setCheckoutStarted } from "../../../src/runtime/index.js";
import { testEmbedder } from "../../support/hook-events.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** Statements run on the connection `counting` names, prepared once or for one run. */
const statements = { counting: undefined as unknown, run: 0 };
{
  const probe = new Database(":memory:");
  const prototype = Object.getPrototypeOf(probe.prepare("SELECT 1")) as Record<string, (...parameters: unknown[]) => unknown>;
  probe.close();
  for (const method of ["run", "get", "all", "iterate"]) {
    const run = prototype[method]!;
    prototype[method] = function (this: { database: unknown }, ...parameters: unknown[]) {
      if (this.database === statements.counting) statements.run += 1;
      return run.apply(this, parameters);
    };
  }
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

const RULE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

/**
 * A session in a started repository's main checkout with `count` Git worktrees beside it, as a repository many agents
 * work in has. The session knows every worktree, as its first walk of them leaves it (`worktreeRoots`), and has a lane
 * open in each, as that walk opens one wherever it asks whether a file is read.
 */
async function sessionWithWorktrees(count: number) {
  const base = await realpath(await mkdtemp(join(tmpdir(), "rerail-router-cost-")));
  cleanups.push(() => rm(base, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 }));
  const main = join(base, "main");
  await mkdir(join(main, "api"), { recursive: true });
  await writeFile(join(main, "api", "x.py"), "x = 1\n");
  git(base, "init", "--quiet", "-b", "main", main);
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  const databasePath = join(base, "rerail.sqlite");
  setUpTestRepository(main, { library: [{ kind: "code", rule: RULE }] }, databasePath);
  const worktrees: string[] = [];
  for (let index = 0; index < count; index += 1) {
    const worktree = join(base, "worktrees", `w${index}`);
    git(main, "worktree", "add", "--quiet", "--detach", worktree, "HEAD");
    worktrees.push(await realpath(worktree));
  }
  const catalogs = new CheckoutCatalogs(databasePath);
  const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
  const auditor = await openRouter(main, { owners, catalogs }, "session");
  cleanups.push(async () => {
    await auditor.close();
    catalogs.close();
  });
  expect(auditor.worktreeRoots()).toEqual(worktrees);
  for (const worktree of worktrees) expect(auditor.acceptsPath(join(worktree, "api", "x.py"))).toBe(true);
  // The first walk records each worktree it finds; the next one finds them recorded.
  auditor.worktreeRoots();
  return { base, main, databasePath, worktrees, catalogs, owners, auditor };
}

/** A session's router in `folder`, open, over `shared`: the runtime gives every session the same catalogs and owners. */
async function openRouter(
  folder: string,
  shared: { readonly owners: CheckoutOwners; readonly catalogs: CheckoutCatalogs },
  sessionId: string,
): Promise<CheckoutCodeRuleRouter> {
  const auditor = new CheckoutCodeRuleRouter({
    sessionRoot: folder, startingCheckout: startingCheckout(shared.owners, folder).checkoutRoot, ...shared,
    steerDelivery: { requestSteer: async () => undefined },
    audits: { start: () => "audit", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
  });
  await auditor.openWorkspace(sessionId);
  return auditor;
}

/** What `work` costs: the statements it runs on `database`'s connection and the Git processes it starts. */
async function cost(database: RerailDatabase, work: () => Promise<void> | void): Promise<{ statements: number; gitProcesses: number }> {
  statements.counting = database.$client;
  statements.run = 0;
  const started = gitProcesses.started;
  try {
    await work();
  } finally {
    statements.counting = undefined;
  }
  return { statements: statements.run, gitProcesses: gitProcesses.started - started };
}

function withDatabase(databasePath: string, write: (database: RerailDatabase) => void): void {
  const database = openRerailDatabase(databasePath);
  try {
    write(database);
  } finally {
    database.$client.close();
  }
}

/** Turns the rule on or off in `checkout` by a setting of the checkout's own, as `rerail rules` does from another process. */
function turnRule(database: RerailDatabase, checkout: string, on: boolean): void {
  setRuleSetting(database, { owner: { kind: "checkout", checkoutPath: checkout }, folder: "", targetKind: "rule", targetId: RULE.id, on, mode: null });
}

describe("CheckoutCodeRuleRouter in a repository with many worktrees", () => {
  /** An edit tool call as the runtime and the bridge drive the router: an event, its begin, its result, its finish. */
  async function editToolCall(auditor: CheckoutCodeRuleRouter, revision: number): Promise<void> {
    auditor.refreshCodeRules();
    auditor.refreshCodeRules();
    auditor.acceptsPath("api/x.py");
    auditor.refreshCodeRules();
    auditor.refreshCodeRules();
    auditor.ignoredEdit("api/x.py");
    auditor.ignoreRuleRoots();
    await auditor.auditGeneratedEdits("session", { revision, checkpoints: ["edit"], edits: [{ path: "api/x.py", language: "python", generatedText: "x = 2\n" }] });
    await auditor.settleGeneratedEdits();
  }

  it("costs one tool call the same statements and Git processes however many worktrees the session knows", async () => {
    const costOf = async (count: number) => {
      const { catalogs, auditor } = await sessionWithWorktrees(count);
      await editToolCall(auditor, 1);
      return await cost(catalogs.database, () => editToolCall(auditor, 2));
    };
    const few = await costOf(1);
    expect(await costOf(6)).toEqual(few);
    // The checkout declares no submodules, so its ignore rules ask Git nothing.
    expect(few.gitProcesses).toBe(0);
  }, 60_000);

  it("asks Git again only about the new worktree when another process records a checkout", async () => {
    const gitOf = async (count: number) => {
      const { base, main, databasePath, catalogs, auditor } = await sessionWithWorktrees(count);
      const late = join(base, "worktrees", "late");
      git(main, "worktree", "add", "--quiet", "--detach", late, "HEAD");
      withDatabase(databasePath, database => recordCheckout(database, late));
      const { gitProcesses: started } = await cost(catalogs.database, () => {
        expect(auditor.worktreeRoots()).toContain(late);
      });
      return started;
    };
    expect(await gitOf(6)).toBe(await gitOf(1));
  }, 60_000);

  it("reads the session checkout's rules once in an event, however many times the router, its lane and the conversation rules ask", async () => {
    const { main, catalogs, auditor } = await sessionWithWorktrees(1);
    const conversation = catalogs.conversationRules(main, "");
    const toolCall = async (revision: number, inEvent: boolean) => {
      const reads = ruleCatalogReads.count;
      const closeEvent = inEvent ? catalogs.openEvent() : () => undefined;
      try {
        const { statements } = await cost(catalogs.database, async () => {
          conversation.signature();
          await editToolCall(auditor, revision);
        });
        return { statements, ruleCatalogReads: ruleCatalogReads.count - reads };
      } finally {
        closeEvent();
      }
    };
    await toolCall(1, false);
    // With no event open, each of the six refreshes reads the checkout's record, its rule configuration and the installed
    // packs; in one event the first does, and the router still reads every known checkout's record at each of its four.
    expect(await toolCall(2, false)).toEqual({ statements: 20, ruleCatalogReads: 6 });
    expect(await toolCall(3, true)).toEqual({ statements: 10, ruleCatalogReads: 1 });
  }, 60_000);

  it("reads the session checkout's rules at every event, so what its catalog reports is current before any tool call", async () => {
    const { main, databasePath, catalogs, auditor } = await sessionWithWorktrees(1);
    const catalog = catalogs.catalog(main);
    expect(catalog.status.ruleCount).toBe(1);
    const repositoryKey = catalogs.checkout(main)!.repositoryKey;
    withDatabase(databasePath, database => {
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: RULE.id, on: false, mode: null });
    });
    // An event with no tool call, as a message's end is: the runtime then reports the session's rule coverage from it.
    auditor.refreshCodeRules();
    expect(catalog.status.ruleCount).toBe(0);
  }, 60_000);

  it("reads a worktree's rules when its lane opens again, after events that did not use the lane before it closed", async () => {
    const { databasePath, worktrees: [worktree], auditor } = await sessionWithWorktrees(1);
    const path = join(worktree!, "api", "x.py");
    withDatabase(databasePath, database => turnRule(database, worktree!, false));
    auditor.refreshCodeRules();
    expect(auditor.acceptsPath(path)).toBe(false);
    withDatabase(databasePath, database => turnRule(database, worktree!, true));
    auditor.refreshCodeRules();
    auditor.refreshCodeRules();
    withDatabase(databasePath, database => setCheckoutStarted(database, recordCheckout(database, worktree!), false));
    auditor.refreshCodeRules();
    withDatabase(databasePath, database => setCheckoutStarted(database, recordCheckout(database, worktree!), true));
    auditor.refreshCodeRules();
    expect(auditor.acceptsPath(path)).toBe(true);
  }, 60_000);

  it("opens a lane with a worktree's rules as they are, though another session's lane read them before a change", async () => {
    const { main, databasePath, worktrees: [worktree], catalogs, owners, auditor } = await sessionWithWorktrees(1);
    withDatabase(databasePath, database => turnRule(database, worktree!, false));
    // The first session's event leaves its lane in the worktree as it was until it next uses it.
    auditor.refreshCodeRules();
    const second = await openRouter(main, { owners, catalogs }, "second");
    cleanups.push(() => second.close());
    second.refreshCodeRules();
    expect(second.acceptsPath(join(worktree!, "api", "x.py"))).toBe(false);
  }, 60_000);

  it("walks a worktree Git creates again at the path it still lists, though the walks worked the list out while it was gone", async () => {
    const { base, main, worktrees: [worktree], auditor } = await sessionWithWorktrees(1);
    await rm(worktree!, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
    // Another worktree changes Git's registry, so the next walk works the list out again without the removed one.
    git(main, "worktree", "add", "--quiet", "--detach", join(base, "worktrees", "other"), "HEAD");
    expect(auditor.worktreeRoots()).not.toContain(worktree);
    expect(auditor.worktreeRoots()).not.toContain(worktree);
    // Over the entry Git still lists, `add -f` writes the same name and Git folder: the registry reads as it did.
    git(main, "worktree", "add", "--quiet", "-f", "--detach", worktree!, "HEAD");
    auditor.refreshCodeRules();
    expect(auditor.worktreeRoots()).toContain(worktree);
  }, 60_000);

  it("closes the lane of a worktree another process stops at the next event, and audits with a setting changed elsewhere", async () => {
    const { databasePath, worktrees: [stopped, turnedOff], auditor } = await sessionWithWorktrees(2);
    const needle = (worktree: string) => [{ path: join(worktree, "api", "x.py"), language: "python" as const, generatedText: "NEEDLE\n" }];
    await auditor.auditGeneratedEdits("session", { revision: 1, checkpoints: ["edit"], edits: needle(stopped!) });
    await auditor.settleGeneratedEdits();
    const steered = await auditor.auditGeneratedEdits("session", { revision: 2, checkpoints: ["edit"], edits: [] });
    expect(steered.violations.map(violation => violation.code)).toEqual(["NEEDLE"]);

    withDatabase(databasePath, database => {
      setCheckoutStarted(database, recordCheckout(database, stopped!), false);
      setRuleSetting(database, { owner: { kind: "checkout", checkoutPath: turnedOff! }, folder: "", targetKind: "rule", targetId: RULE.id, on: false, mode: null });
    });
    auditor.refreshCodeRules();

    await auditor.auditGeneratedEdits("session", { revision: 3, checkpoints: ["edit"], edits: needle(turnedOff!) });
    await auditor.settleGeneratedEdits();
    const next = await auditor.auditGeneratedEdits("session", { revision: 4, checkpoints: ["edit"], edits: [] });
    expect(next.violations).toEqual([]);
    expect(auditor.acceptsPath(join(stopped!, "api", "x.py"))).toBe(false);
    expect(auditor.acceptsPath("api/x.py")).toBe(true);
  }, 60_000);
});

describe("a tool call through the runtime", () => {
  it("reads the installed packs' catalog once in each of its events, however many of their steps refresh the rules", async () => {
    const base = await realpath(await mkdtemp(join(tmpdir(), "rerail-runtime-cost-")));
    cleanups.push(() => rm(base, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 }));
    const main = join(base, "main");
    await mkdir(join(main, "api"), { recursive: true });
    await writeFile(join(main, "api", "x.py"), "x = 1\n");
    git(base, "init", "--quiet", "-b", "main", main);
    git(main, "add", ".");
    git(main, "commit", "--quiet", "-m", "seed");
    const databasePath = join(base, "rerail.sqlite");
    setUpTestRepository(main, { library: [{ kind: "code", rule: RULE }] }, databasePath);
    const runtime = new RerailRuntime({ storageRoot: join(base, "sessions"), databasePath, workspaceRoot: main, embedder: testEmbedder, autostart: false });
    cleanups.push(() => runtime.dispose());
    await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "session", cwd: main } });
    await runtime.codeRulesOpened("session");

    const reads = ruleCatalogReads.count;
    const write = { sessionId: "session", toolCallId: "write", toolName: "Write" };
    await runtime.ingest({ kind: "call", eventId: "write", ...write, input: { file_path: join(main, "api", "x.py"), content: "x = 2\n" } });
    await writeFile(join(main, "api", "x.py"), "x = 2\n");
    await runtime.ingest({ kind: "result", eventId: "written", ...write, output: "ok" });
    expect(ruleCatalogReads.count - reads).toBe(2);
  }, 60_000);
});
