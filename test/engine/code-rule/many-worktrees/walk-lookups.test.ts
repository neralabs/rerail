import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// Every real path resolved goes through these, so a test can count the look-ups on disk.
vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof import("node:fs")>();
  const realpathSync = Object.assign(vi.fn(actual.realpathSync), { native: vi.fn(actual.realpathSync.native) });
  return { ...actual, realpathSync };
});

import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { recordedCheckoutsVersionReader } from "../../../../src/engine/catalog/repository-registry.js";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { openRerailDatabase, type RerailDatabase } from "../../../../src/engine/storage/database.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";

const cleanups: Array<() => void> = [];
afterEach(() => {
  for (const cleanup of cleanups.splice(0).reverse()) cleanup();
});

const RULE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

/** A started, set-up checkout at its real path with `count` folders, and the router of a session opened there. */
async function sessionIn(count: number): Promise<{ readonly root: string; readonly folders: readonly string[]; readonly router: CheckoutCodeRuleRouter }> {
  const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-walk-lookups-")));
  cleanups.push(() => removeFolderSync(root));
  execFileSync("git", ["init", "--quiet", "-b", "main", root]);
  const databasePath = join(root, ".rerail-test.sqlite");
  setUpTestRepository(root, { library: [{ kind: "code", rule: RULE }] }, databasePath);
  const folders = Array.from({ length: count }, (_, index) => `d${index}`);
  for (const folder of folders) fs.mkdirSync(join(root, folder));
  const catalogs = new CheckoutCatalogs(databasePath);
  const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
  const router = new CheckoutCodeRuleRouter({
    sessionRoot: root, startingCheckout: startingCheckout(owners, root).checkoutRoot, owners, catalogs,
    steerDelivery: { requestSteer: async () => undefined },
    audits: { start: () => "audit", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
  });
  await router.openWorkspace("session");
  cleanups.push(() => catalogs.close());
  return { root, folders, router };
}

/** How many times the disk was asked for the real path of one of `paths`. */
function realPathsOf(paths: readonly string[]): number {
  return [vi.mocked(fs.realpathSync), vi.mocked(fs.realpathSync.native)]
    .flatMap(spy => spy.mock.calls)
    .filter(([path]) => paths.includes(String(path))).length;
}

describe("the look-ups of a walk of the session folder", () => {
  it("asks the disk for no real path of a folder the walk reached through folders alone, however many there are", async () => {
    const { root, folders, router } = await sessionIn(12);
    try {
      const absolute = folders.map(folder => join(root, folder));
      const before = realPathsOf(absolute);
      const view = router.walkView(root);

      const accepted = folders.map(folder => view.acceptsPath(`${folder}/x.py`));

      expect(accepted).toEqual(folders.map(() => true));
      // Each folder below the walk's start is its own real path: a look-up per folder asked the disk 12 times.
      expect(realPathsOf(absolute) - before).toBe(0);
    } finally {
      await router.close();
    }
  });
});

/** The runs of statements on `database` whose text names `table` and not `except`, counted from now on. */
function runsOf(database: RerailDatabase, table: string, except: string): () => number {
  const prototype = Object.getPrototypeOf(database.$client.prepare("SELECT 1")) as { get: (...args: unknown[]) => unknown };
  const get = vi.spyOn(prototype, "get");
  cleanups.push(() => get.mockRestore());
  return () => get.mock.contexts.filter(context => {
    const source = String((context as { source?: unknown }).source);
    return source.includes(table) && !source.includes(except);
  }).length;
}

describe("the recorded checkouts' version a walk's look-ups read", () => {
  it("reads the recorded checkouts once for any number of look-ups with no write between them", () => {
    const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-walk-version-")));
    cleanups.push(() => removeFolderSync(root));
    const database = openRerailDatabase(join(root, "rerail.sqlite"));
    cleanups.push(() => database.$client.close());
    const version = recordedCheckoutsVersionReader(database);
    const runs = runsOf(database, "checkouts", "data_version");

    const versions = new Set(Array.from({ length: 50 }, () => version()));

    expect(versions.size).toBe(1);
    // A look-up per folder of a walk read every row of the recorded checkouts again: 50 reads.
    expect(runs()).toBe(1);
  });
});
