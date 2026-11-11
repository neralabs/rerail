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

import Database from "better-sqlite3";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { setRuleSetting } from "../../../src/engine/catalog/rule-configuration.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { setCheckoutStarted } from "../../../src/runtime/index.js";
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
  const auditor = new CheckoutCodeRuleRouter({
    sessionRoot: main, startingCheckout: startingCheckout(owners, main).checkoutRoot, owners, catalogs,
    steerDelivery: { requestSteer: async () => undefined },
    audits: { start: () => "audit", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
  });
  cleanups.push(async () => {
    await auditor.close();
    catalogs.close();
  });
  await auditor.openWorkspace("session");
  expect(auditor.worktreeRoots()).toEqual(worktrees);
  for (const worktree of worktrees) expect(auditor.acceptsPath(join(worktree, "api", "x.py"))).toBe(true);
  // The first walk records each worktree it finds; the next one finds them recorded.
  auditor.worktreeRoots();
  return { base, main, databasePath, worktrees, catalogs, auditor };
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
