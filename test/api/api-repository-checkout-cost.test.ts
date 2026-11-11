import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { afterEach, describe, expect, it, vi } from "vitest";

/** Filesystem checks Rerail made while `counting` is on. */
const fileChecks = vi.hoisted(() => ({ counting: false, existsSync: 0 }));
vi.mock("node:fs", async importOriginal => {
  const original = await importOriginal<typeof import("node:fs")>();
  return {
    ...original,
    existsSync: (...parameters: Parameters<typeof original.existsSync>) => {
      if (fileChecks.counting) fileChecks.existsSync += 1;
      return original.existsSync(...parameters);
    },
  };
});

import Database from "better-sqlite3";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { checkouts, recordCheckout, repositories } from "../../src/engine/index.js";

/** Statements run on any connection while `counting` is on, prepared once or for one run. */
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
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/**
 * A started, set-up repository whose main checkout is a Git checkout, with `rows` more checkouts recorded under the
 * same clone, as `git worktree add` and the walk record a repository's worktrees.
 */
async function repositoryWith(rows: ReadonlyArray<{ readonly path: (main: string, root: string) => string; readonly started: boolean | null }>) {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-checkout-cost-")));
  cleanups.push(() => rm(root, { recursive: true, force: true }));
  const main = join(root, "factory");
  mkdirSync(main, { recursive: true });
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: main });
  execFileSync("git", ["remote", "add", "origin", "git@github.com:acme/factory.git"], { cwd: main });
  const databasePath = join(root, "rerail.sqlite");
  const stores = new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    origin: () => "http://127.0.0.1:4737",
  });
  cleanups.push(async () => stores.close());
  const db = stores.database();
  const { repositoryKey, cloneId } = recordCheckout(db, main);
  db.update(repositories).set({ started: true, configuredAt: 1 }).where(eq(repositories.repositoryKey, repositoryKey)).run();
  if (rows.length > 0) {
    db.insert(checkouts).values(rows.map(row => ({
      checkoutPath: row.path(main, root), repositoryKey, cloneId, started: row.started, createdAt: 1, lastSeenAt: 1,
    }))).run();
  }
  return { stores, db, main, repositoryKey, app: createRerailApi(stores, operations, { version: "0.0.0-test" }) };
}

/** A repository with `count` checkouts: its main checkout and worktrees in a folder inside it, as the field has them. */
const repositoryOf = (count: number) => repositoryWith(Array.from({ length: count - 1 }, (_, index) => ({
  path: (main: string) => join(main, "wt", `wt-${index}`), started: null,
})));

/** What one request costs once the daemon has answered it before: statements run and `existsSync` calls. */
async function cost(app: ReturnType<typeof createRerailApi>, path: string): Promise<{ status: number; statements: number; existsSync: number; body: unknown }> {
  await app.request(path);
  statements.run = 0;
  fileChecks.existsSync = 0;
  statements.counting = fileChecks.counting = true;
  let response: Response;
  try {
    response = await app.request(path);
  } finally {
    statements.counting = fileChecks.counting = false;
  }
  return { status: response.status, statements: statements.run, existsSync: fileChecks.existsSync, body: await response.json() };
}

describe("the dashboard link's read", () => {
  it("names a checkout's repository and top folder at the same cost however many checkouts the repository has", async () => {
    const answers = [];
    for (const count of [10, 190]) {
      const { main, repositoryKey, app } = await repositoryOf(count);
      mkdirSync(join(main, "src"));
      const answer = await cost(app, `/api/repository/checkout?${new URLSearchParams({ checkout: join(main, "src") })}`);
      expect(answer.status).toBe(200);
      expect(answer.body).toEqual({ repositoryKey, checkoutPath: main });
      answers.push(answer);
    }
    const [few, many] = answers;
    expect({ statements: many!.statements, existsSync: many!.existsSync }).toEqual({ statements: few!.statements, existsSync: few!.existsSync });
  });

  it("answers for a path Rerail has not seen as for one that does not exist", async () => {
    const { main, app } = await repositoryOf(1);
    const unseen = join(main, "..", "unseen");
    mkdirSync(unseen);
    for (const checkout of [unseen, join(main, "..", "missing")]) {
      const response = await app.request(`/api/repository/checkout?${new URLSearchParams({ checkout })}`);
      expect(response.status).toBe(404);
      expect(await response.json()).toMatchObject({ error: "checkout-not-found" });
    }
  });
});

describe("a repository's context", () => {
  it("runs the same statements however many checkouts the repository has, and checks each checkout's folder once", async () => {
    const answers = [];
    for (const count of [10, 190]) {
      const { main, app } = await repositoryOf(count);
      const answer = await cost(app, `/api/repository?${new URLSearchParams({ checkout: main })}`);
      expect(answer.status).toBe(200);
      answers.push(answer);
    }
    const [few, many] = answers;
    expect(many!.statements).toBe(few!.statements);
    // `onDisk` is one existsSync per checkout; 180 more checkouts cost 180 more.
    expect(many!.existsSync - few!.existsSync).toBe(180);
  });

  it("gives each checkout its state, what decided it and its nearest containing checkout", async () => {
    const { db, main, repositoryKey, app } = await repositoryWith([
      { path: main => join(main, "wt", "a"), started: false },
      { path: main => join(main, "wt", "a", "inner"), started: null },
      { path: main => join(main, "wt", "b"), started: null },
      { path: main => join(main, "x"), started: true },
      { path: main => join(main, "x", "y"), started: null },
      { path: main => join(main, "x", "y", "z"), started: false },
      // Beside the main checkout, not inside it: only the repository decides.
      { path: (_main, root) => join(root, "beside"), started: null },
      // The main checkout's folder spelled in upper case: the same folder on Windows, another one elsewhere.
      { path: main => join(main, "..", "FACTORY", "wt", "c"), started: null },
    ]);
    db.update(repositories).set({ started: false }).where(eq(repositories.repositoryKey, repositoryKey)).run();
    db.update(checkouts).set({ started: true }).where(eq(checkouts.checkoutPath, main)).run();
    const response = await app.request(`/api/repository?${new URLSearchParams({ checkout: main })}`);
    const body = await response.json() as {
      clones: Array<{ checkouts: Array<{ checkoutPath: string; state: string; startedBy: string; parentCheckoutPath: string | null }> }>;
      checkout: unknown;
    };
    const listed = new Map(body.clones.flatMap(clone => clone.checkouts)
      .map(entry => [entry.checkoutPath, [entry.state, entry.startedBy, entry.parentCheckoutPath]]));
    // path.relative, which says whether one checkout holds another, ignores letter case on Windows only.
    const upper = join(main, "..", "FACTORY", "wt", "c");
    const upperHeld = process.platform === "win32";
    expect(listed).toEqual(new Map([
      [main, ["started", "checkout", null]],
      [join(main, "wt", "a"), ["stopped", "checkout", main]],
      [join(main, "wt", "a", "inner"), ["stopped", "parent", join(main, "wt", "a")]],
      [join(main, "wt", "b"), ["started", "parent", main]],
      [join(main, "x"), ["started", "checkout", main]],
      [join(main, "x", "y"), ["started", "parent", join(main, "x")]],
      [join(main, "x", "y", "z"), ["stopped", "checkout", join(main, "x", "y")]],
      [join(main, "..", "beside"), ["stopped", "repository", null]],
      [upper, upperHeld ? ["started", "parent", main] : ["stopped", "repository", null]],
    ]));
    expect(body.checkout).toEqual({ checkoutPath: main, folder: "", state: "started" });
  });
});
