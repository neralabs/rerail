import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import { mkdtempSync, realpathSync, renameSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { recordCheckout } from "../../../../src/engine/catalog/repository-registry.js";
import { openRerailDatabase } from "../../../../src/engine/storage/database.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";

// Every registry file read and every stored path resolved goes through these, so a test can count them.
vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof import("node:fs")>();
  const realpathSync = Object.assign(vi.fn(actual.realpathSync), { native: vi.fn(actual.realpathSync.native) });
  return { ...actual, readFileSync: vi.fn(actual.readFileSync), realpathSync };
});

const folders: string[] = [];
afterEach(() => {
  for (const folder of folders.splice(0)) removeFolderSync(folder);
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "commit.gpgsign=false", ...args], { cwd, stdio: "ignore" });
}

const RULE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

/** A started, set-up clone's main checkout with `count` linked worktrees beside it, none of them recorded, and its catalogs. */
function startedClone(count: number) {
  const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-registry-pass-")));
  folders.push(root);
  const main = join(root, "main");
  fs.mkdirSync(main);
  git(main, "init", "--quiet", "-b", "main");
  git(main, "commit", "--quiet", "--allow-empty", "-m", "seed");
  const databasePath = join(root, "rerail.sqlite");
  setUpTestRepository(main, { library: [{ kind: "code", rule: RULE }] }, databasePath);
  const worktrees = Array.from({ length: count }, (_, index) => {
    const worktree = join(root, `w${index}`);
    git(main, "worktree", "add", "--quiet", "--no-checkout", "-b", `w${index}`, worktree);
    return worktree;
  });
  const catalogs = new CheckoutCatalogs(databasePath);
  return { root, main, databasePath, worktrees, catalogs, close: () => catalogs.close() };
}

/** A session's router in the clone's main checkout, open, as the runtime gives one to each session. */
async function sessionRouter(clone: { readonly main: string; readonly catalogs: CheckoutCatalogs }): Promise<CheckoutCodeRuleRouter> {
  const owners = new CheckoutOwners(() => clone.catalogs.recordedCheckoutsVersion());
  const router = new CheckoutCodeRuleRouter({
    sessionRoot: clone.main, startingCheckout: startingCheckout(owners, clone.main).checkoutRoot, owners, catalogs: clone.catalogs,
    steerDelivery: { requestSteer: async () => undefined },
    audits: { start: () => "audit", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
  });
  await router.openWorkspace("session");
  return router;
}

/** How many turns of the event loop pass while `work` runs, and how far `count` moved in each run of the thread between them. */
async function turnsDuring<T>(work: () => Promise<T>, count: () => number = () => 0): Promise<{ readonly result: T; readonly turns: number; readonly most: number }> {
  let turns = 0;
  let running = true;
  const marks = [count()];
  const tick = (): void => {
    if (!running) return;
    turns += 1;
    marks.push(count());
    setImmediate(tick);
  };
  setImmediate(tick);
  const result = await work();
  running = false;
  marks.push(count());
  return { result, turns, most: Math.max(...marks.slice(1).map((mark, index) => mark - marks[index]!)) };
}

/** The cases that add 12 to 40 worktrees with Git: on a busy Windows machine each add takes up to a second. */
const WORKTREES_TIMEOUT_MS = 120_000;

const registryReads = (): number => vi.mocked(fs.readFileSync).mock.calls
  .filter(([path]) => String(path).endsWith(`${sep}gitdir`)).length;
const realPaths = (paths: readonly string[]): number => vi.mocked(fs.realpathSync.native).mock.calls
  .filter(([path]) => paths.includes(String(path))).length;

describe("a read of the recorded checkouts that records many worktrees", () => {
  it("reads Git's registry once and resolves each listed worktree once, however many worktrees it records", () => {
    const perWorktree = (count: number): { readonly reads: number; readonly realPaths: number } => {
      const clone = startedClone(count);
      try {
        const reads = registryReads();
        const resolved = realPaths(clone.worktrees);
        const eligibility = clone.catalogs.refreshAll(clone.worktrees);
        expect([...eligibility.values()]).toEqual(clone.worktrees.map(() => "ok"));
        return { reads: registryReads() - reads, realPaths: (realPaths(clone.worktrees) - resolved) / count };
      } finally {
        clone.close();
      }
    };

    const few = perWorktree(8);
    const many = perWorktree(16);

    // One read of each registry file, where a read per worktree recorded read 64 and 256.
    expect([few.reads, many.reads]).toEqual([8, 16]);
    // The real paths of the listed worktrees per worktree recorded stay the same with twice the worktrees: a read per
    // worktree resolved the listed ones until it met its own, half of them on average.
    expect(many.realPaths - few.realPaths).toBeLessThan(1);
  }, WORKTREES_TIMEOUT_MS);

  it("reads it once for a session's first list of the other worktrees, which records each of them", async () => {
    const clone = startedClone(12);
    const router = await sessionRouter(clone);
    try {
      const reads = registryReads();

      expect(router.worktreeRoots().toSorted()).toEqual(clone.worktrees.toSorted());

      // The list's own read and one for the worktrees it records, where a read per worktree recorded made 13.
      expect(registryReads() - reads).toBe(2 * 12);
    } finally {
      await router.close();
      clone.close();
    }
  }, WORKTREES_TIMEOUT_MS);

  it("gives the event loop a turn after each 16 worktrees a session's first list records, and none once they are read", async () => {
    const clone = startedClone(40);
    const router = await sessionRouter(clone);
    const lookups = vi.spyOn(CheckoutOwners.prototype, "ownerOf");
    try {
      const first = await turnsDuring(() => router.worktreeRootsInTurns(), () => lookups.mock.calls.length);
      const again = await turnsDuring(() => router.worktreeRootsInTurns());

      expect(first.result?.toSorted()).toEqual(clone.worktrees.toSorted());
      // 40 worktrees first seen: 16, 16 and 8, with a turn between each, and no run of the thread looks up the owners of
      // more than one group (each worktree's owner before it is recorded and after): filtering all 40 after the last
      // turn looked up 48 at once.
      expect(first.turns).toBe(2);
      expect(first.most).toBeLessThanOrEqual(2 * 16);
      expect(again.result?.toSorted()).toEqual(clone.worktrees.toSorted());
      expect(again.turns).toBe(0);
    } finally {
      lookups.mockRestore();
      await router.close();
      clone.close();
    }
  }, WORKTREES_TIMEOUT_MS);

  it("records no group after the router is interrupted between two turns of a session's first list, and gives no list", async () => {
    const clone = startedClone(40);
    const router = await sessionRouter(clone);
    const refreshes = vi.spyOn(clone.catalogs, "refreshAll");
    try {
      // The first group is recorded before the first turn, in which the session closes.
      const listing = router.worktreeRootsInTurns();
      router.interrupt();
      const before = refreshes.mock.calls.length;

      expect(await listing).toBeUndefined();
      // The two groups left were recorded after the close, each in a read of the recorded checkouts.
      expect(refreshes.mock.calls.length - before).toBe(0);
    } finally {
      refreshes.mockRestore();
      await router.close();
      clone.close();
    }
  }, WORKTREES_TIMEOUT_MS);

  it("answers as a read per worktree at the next read: a worktree added, moved, removed or pruned since", () => {
    const clone = startedClone(2);
    try {
      const [first, second] = clone.worktrees as [string, string];
      expect([...clone.catalogs.refreshAll([first]).values()]).toEqual(["ok"]);

      const added = join(clone.root, "added");
      git(clone.main, "worktree", "add", "--quiet", "--no-checkout", "-b", "added", added);
      const moved = join(clone.root, "moved");
      git(clone.main, "worktree", "move", second, moved);
      const pruned = join(clone.root, "pruned");
      git(clone.main, "worktree", "add", "--quiet", "--no-checkout", "-b", "pruned", pruned);
      removeFolderSync(pruned);
      git(clone.main, "worktree", "prune");
      // The registry no longer lists the old path or the pruned one; a .git file naming the clone there is no worktree.
      for (const forged of [second, pruned]) {
        fs.mkdirSync(forged, { recursive: true });
        writeFileSync(join(forged, ".git"), `gitdir: ${join(clone.main, ".git")}\n`);
      }

      const eligibility = clone.catalogs.refreshAll([added, moved, second, pruned]);

      expect(Object.fromEntries(eligibility)).toEqual({ [added]: "ok", [moved]: "ok", [second]: "unrecorded", [pruned]: "unrecorded" });
    } finally {
      clone.close();
    }
  });

  it("records nothing for a listed worktree whose folder is gone, nor for a worktree of a clone not recorded", () => {
    const clone = startedClone(2);
    const other = startedClone(0);
    try {
      const [gone, kept] = clone.worktrees as [string, string];
      renameSync(gone, `${gone}-away`);
      const unrecordedMain = join(other.root, "unrecorded");
      fs.mkdirSync(unrecordedMain);
      git(unrecordedMain, "init", "--quiet", "-b", "main");
      git(unrecordedMain, "commit", "--quiet", "--allow-empty", "-m", "seed");
      const stray = join(other.root, "stray");
      git(unrecordedMain, "worktree", "add", "--quiet", "--no-checkout", "-b", "stray", stray);

      const eligibility = clone.catalogs.refreshAll([gone, kept, stray]);

      expect(Object.fromEntries(eligibility)).toEqual({ [gone]: "unrecorded", [kept]: "ok", [stray]: "unrecorded" });
    } finally {
      clone.close();
      other.close();
    }
  });

  it("takes a worktree another process recorded as recorded at the next read", () => {
    const clone = startedClone(1);
    try {
      const [worktree] = clone.worktrees as [string];
      expect(clone.catalogs.refresh(clone.main)).toBe("ok");
      const elsewhere = openRerailDatabase(clone.databasePath);
      try {
        recordCheckout(elsewhere, worktree);
      } finally {
        elsewhere.$client.close();
      }
      const reads = registryReads();

      expect([...clone.catalogs.refreshAll([worktree]).values()]).toEqual(["ok"]);
      // Recorded already, so Git's registry is not asked for it.
      expect(registryReads() - reads).toBe(0);
    } finally {
      clone.close();
    }
  });
});
