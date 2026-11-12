import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join, relative, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * Rerail's calls of the `node:fs` functions a checkout lookup makes, and the paths it resolved; the test's own go
 * through `node:fs/promises`.
 */
const fsCalls = vi.hoisted(() => ({ realpath: 0, resolved: [] as string[] }));
vi.mock("node:fs", async importOriginal => {
  const original = await importOriginal<typeof import("node:fs")>();
  const realpathSync = Object.assign(
    (...args: Parameters<typeof original.realpathSync>) => {
      fsCalls.realpath += 1;
      fsCalls.resolved.push(String(args[0]));
      return original.realpathSync(...args);
    },
    {
      native: (...args: Parameters<typeof original.realpathSync.native>) => {
        fsCalls.realpath += 1;
        fsCalls.resolved.push(String(args[0]));
        return original.realpathSync.native(...args);
      },
    },
  );
  return { ...original, default: { ...original, realpathSync }, realpathSync };
});

import Database from "better-sqlite3";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../src/engine/code-rule/code-rule-bridge.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

/** Statements run while `counting`, on any connection, prepared once or for one run. */
const statements = { counting: false, run: 0 };
{
  const probe = new Database(":memory:");
  const prototype = Object.getPrototypeOf(probe.prepare("SELECT 1")) as Record<string, (...parameters: unknown[]) => unknown>;
  probe.close();
  for (const method of ["run", "get", "all", "iterate"]) {
    const run = prototype[method]!;
    prototype[method] = function (this: unknown, ...parameters: unknown[]) {
      if (statements.counting) statements.run += 1;
      return run.apply(this, parameters);
    };
  }
}

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  vi.restoreAllMocks();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

const RULE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

const FOLDERS = ["src", "src/a", "src/a/b", "docs"];
const WORKTREES = 2;

/** Writes `files` sources into each of `folders` below `checkout`. */
async function sources(checkout: string, folders: readonly string[], files: number): Promise<void> {
  for (const folder of folders) {
    await mkdir(join(checkout, folder), { recursive: true });
    for (let index = 0; index < files; index += 1) await writeFile(join(checkout, folder, `f${index}.py`), `x = ${index}\n`);
  }
}

/**
 * A session in a started repository's main checkout with `WORKTREES` worktrees beside it, each holding `folders` of
 * `files` sources, over the checkout lookups and catalogs a runtime gives its sessions.
 */
async function sessionWithWorktrees(files: number, folders: readonly string[] = FOLDERS) {
  const base = await realpath(await mkdtemp(join(tmpdir(), "rerail-owner-walk-")));
  cleanups.push(() => removeFolder(base));
  const main = join(base, "main");
  await sources(main, folders, files);
  git(base, "init", "--quiet", "-b", "main", main);
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  const databasePath = join(base, "rerail.sqlite");
  setUpTestRepository(main, { library: [{ kind: "code", rule: RULE }] }, databasePath);
  const worktrees: string[] = [];
  for (let index = 0; index < WORKTREES; index += 1) {
    const worktree = join(base, `w${index}`);
    git(main, "worktree", "add", "--quiet", "--detach", worktree, "HEAD");
    worktrees.push(worktree);
  }
  const catalogs = new CheckoutCatalogs(databasePath);
  const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
  const auditor = new CheckoutCodeRuleRouter({
    sessionRoot: main, startingCheckout: startingCheckout(owners, main).checkoutRoot, owners, catalogs,
    steerDelivery: { requestSteer: async () => undefined },
    audits: { start: () => "audit", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
  });
  const bridge = new CodeRuleBridge(main, auditor, { worktreeWalkGapMs: 0, discoveryWaitMs: 60_000 });
  cleanups.push(async () => {
    await bridge.close();
    catalogs.close();
  });
  return { base, main, worktrees, bridge, catalogs };
}

/** A walk of the session folder, then one of the other worktrees, as a turn's end starts them. */
async function walks(bridge: CodeRuleBridge): Promise<void> {
  await bridge.checkpoint("turn");
  await bridge.settleWorktreeWalk();
}

/** What `work` costs: its checkout lookups, the realpath calls Rerail makes and the statements it runs. */
async function cost(work: () => Promise<void>): Promise<{ lookups: number; realpaths: number; statements: number }> {
  const lookups = vi.spyOn(CheckoutOwners.prototype, "ownerOf");
  const realpaths = fsCalls.realpath;
  statements.run = 0;
  statements.counting = true;
  try {
    await work();
  } finally {
    statements.counting = false;
  }
  const looked = lookups.mock.calls.length;
  lookups.mockRestore();
  return { lookups: looked, realpaths: fsCalls.realpath - realpaths, statements: statements.run };
}

describe("checkout lookups of a session's walks", () => {
  it("cost the same however many files each folder holds, the first sight of a worktree included", { timeout: 120_000 }, async () => {
    const costs = [];
    for (const files of [1, 6]) {
      const { bridge } = await sessionWithWorktrees(files);
      // The session folder's first walk, and the first of the worktrees, see every file first.
      const first = await cost(async () => {
        await bridge.openWorkspace("session");
        await walks(bridge);
      });
      costs.push({ files, first, next: await cost(() => walks(bridge)) });
    }
    // Each file of a folder is owned by the folder's checkout: the walk found it by listing the folder, never through a link.
    expect(costs[1]).toEqual({ ...costs[0], files: 6 });
  });

  it("cost the same however many folders each checkout holds, also once an unrelated checkout is recorded", { timeout: 120_000 }, async () => {
    const costs = [];
    for (const folders of [FOLDERS, [...FOLDERS, "lib", "lib/x", "lib/x/y", "lib/x/y/z", "tools", "tools/bin"]]) {
      const { base, bridge, catalogs } = await sessionWithWorktrees(1, folders);
      const first = await cost(async () => {
        await bridge.openWorkspace("session");
        await walks(bridge);
      });
      const next = await cost(() => walks(bridge));
      // Another repository's checkout is recorded in the same database, as a session that starts in it records it.
      const other = join(base, "other");
      await sources(other, ["src"], 1);
      git(base, "init", "--quiet", "-b", "main", other);
      git(other, "add", ".");
      git(other, "commit", "--quiet", "-m", "seed");
      recordCheckout(catalogs.database, other);
      costs.push({ first, next, recorded: await cost(() => walks(bridge)) });
    }
    expect(costs[1]).toEqual(costs[0]);
  });

  it("look up no changed file's checkout but to route its audit, however many the walks find", { timeout: 120_000 }, async () => {
    const costs = [];
    for (const files of [1, 6]) {
      const { main, worktrees, bridge } = await sessionWithWorktrees(files);
      await bridge.openWorkspace("session");
      await walks(bridge);
      await walks(bridge);
      // Every file of every checkout changes, as a branch switch or a pull leaves them.
      for (const checkout of [main, ...worktrees]) {
        for (const folder of FOLDERS) {
          for (let index = 0; index < files; index += 1) await writeFile(join(checkout, folder, `f${index}.py`), `y = ${index}\n`);
        }
      }
      const audits = vi.spyOn(CheckoutCodeRuleRouter.prototype, "auditGeneratedEdits");
      const { lookups } = await cost(() => walks(bridge));
      // The router routes each edit it audits to its checkout's lane with a lookup of its own, as for a tool call's edit.
      const routed = audits.mock.calls.reduce((sum, [, batch]) => sum + batch.edits.length, 0);
      audits.mockRestore();
      costs.push({ files, routed, others: lookups - routed });
    }
    expect(costs[1]!.routed).toBeGreaterThan(costs[0]!.routed);
    expect(costs[1]!.others).toBe(costs[0]!.others);
  });
});

describe("a walk's checkout lookups", () => {
  it("resolve no real path again once the recorded checkouts change", async () => {
    const base = await realpath(await mkdtemp(join(tmpdir(), "rerail-owner-walk-")));
    cleanups.push(() => removeFolder(base));
    const main = join(base, "main");
    await sources(main, FOLDERS, 1);
    git(base, "init", "--quiet", "-b", "main", main);
    let version = "1";
    const owners = new CheckoutOwners(() => version);
    const walk = (): void => {
      const lookUp = owners.walk();
      for (const folder of ["", ...FOLDERS]) lookUp(join(main, folder));
    };
    walk();

    version = "2";
    const before = fsCalls.resolved.length;
    walk();

    // Git may be asked again whether the checkout's top level is one, which resolves that folder, but no folder below it.
    const inside = (path: string): boolean => {
      const rest = relative(main, path);
      return rest !== "" && !isAbsolute(rest) && rest !== ".." && !rest.startsWith(`..${sep}`);
    };
    expect(fsCalls.resolved.slice(before).filter(inside)).toEqual([]);
  });

  it("resolve the real path of no folder but one whose `.git` entry came, to ask Git about it", async () => {
    const base = await realpath(await mkdtemp(join(tmpdir(), "rerail-owner-walk-")));
    cleanups.push(() => removeFolder(base));
    const main = join(base, "main");
    await sources(main, FOLDERS, 1);
    git(base, "init", "--quiet", "-b", "main", main);
    const owners = new CheckoutOwners();
    const walk = (): void => {
      const lookUp = owners.walk();
      for (const folder of ["", ...FOLDERS]) lookUp(join(main, folder));
    };
    walk();

    // `src/a/` becomes a checkout of its own: Git confirms it, and the folder below it takes it from its folder's answer.
    const nested = join(main, "src", "a");
    git(nested, "init", "--quiet");
    const before = fsCalls.resolved.length;
    walk();

    expect(fsCalls.resolved.slice(before).filter(path => relative(nested, path) !== "")).toEqual([]);
  });
});
