import { execFileSync } from "node:child_process";
import { appendFileSync, mkdirSync, realpathSync, renameSync, utimesSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
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
    statSync: ((...parameters: Parameters<typeof original.statSync>) => {
      if (reads.counting && parameters[1]?.bigint === true) reads.stamps += 1;
      return original.statSync(...parameters);
    }) as typeof original.statSync,
  };
});

/** Git locations placed, remotes read, processes started and files stamped (one `stat` in `bigint`) while `counting` is on. */
const reads = vi.hoisted(() => ({ counting: false, placements: 0, remotes: 0, starts: 0, stamps: 0 }));
vi.mock("../../src/engine/catalog/repository-identity.js", async () => {
  const original = await vi.importActual<typeof import("../../src/engine/catalog/repository-identity.js")>("../../src/engine/catalog/repository-identity.js");
  return {
    ...original,
    repositoryIdentityLocation: (...parameters: Parameters<typeof original.repositoryIdentityLocation>) => {
      if (reads.counting) reads.placements += 1;
      return original.repositoryIdentityLocation(...parameters);
    },
    readRemoteAddress: (...parameters: Parameters<typeof original.readRemoteAddress>) => {
      if (reads.counting) reads.remotes += 1;
      return original.readRemoteAddress(...parameters);
    },
  };
});
vi.mock("../../src/engine/child-process.js", async () => {
  const original = await vi.importActual<typeof import("../../src/engine/child-process.js")>("../../src/engine/child-process.js");
  const spawn = ((...parameters: Parameters<typeof original.spawn>) => {
    if (reads.counting) reads.starts += 1;
    return original.spawn(...parameters);
  }) as typeof original.spawn;
  const spawnSync = ((...parameters: Parameters<typeof original.spawnSync>) => {
    if (reads.counting) reads.starts += 1;
    return original.spawnSync(...parameters);
  }) as typeof original.spawnSync;
  return { ...original, spawn, spawnSync };
});

import Database from "better-sqlite3";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { checkouts, lookupCheckout, recordCheckout, repositories } from "../../src/engine/index.js";
import { loopInPlace } from "../support/platform/links.js";
import { removeFolder } from "../support/platform/remove-folder.js";

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

/** A new folder with the daemon's stores over a database in it, and the API the daemon serves from them. */
async function daemonStores() {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-context-cost-")));
  cleanups.push(() => removeFolder(root));
  const databasePath = join(root, "rerail.sqlite");
  const stores = new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    origin: () => "http://127.0.0.1:4737",
  });
  cleanups.push(async () => stores.close());
  return { root, stores, db: stores.database(), app: createRerailApi(stores, operations, { version: "0.0.0-test" }) };
}

/**
 * A started, set-up repository whose main checkout is a Git checkout, with `rows` more checkouts recorded under the
 * same clone, as `git worktree add` and the walk record a repository's worktrees.
 */
async function repositoryWith(rows: ReadonlyArray<{ readonly path: (main: string, root: string) => string; readonly started: boolean | null }>) {
  const { root, stores, db, app } = await daemonStores();
  const main = join(root, "factory");
  mkdirSync(main, { recursive: true });
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: main });
  execFileSync("git", ["remote", "add", "origin", "git@github.com:acme/factory.git"], { cwd: main });
  const { repositoryKey, cloneId } = recordCheckout(db, main);
  db.update(repositories).set({ started: true, configuredAt: 1 }).where(eq(repositories.repositoryKey, repositoryKey)).run();
  if (rows.length > 0) {
    db.insert(checkouts).values(rows.map(row => ({
      checkoutPath: row.path(main, root), repositoryKey, cloneId, started: row.started, createdAt: 1, lastSeenAt: 1,
    }))).run();
  }
  return { stores, db, main, repositoryKey, cloneId, app };
}

/** A Git worktree of `main` at `path`, which no row records yet, as `git worktree add` leaves one. */
function addWorktree(main: string, path: string): void {
  const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", ...args], { cwd: main, stdio: "ignore" });
  git("commit", "-q", "--allow-empty", "-m", "first");
  git("worktree", "add", "-q", "--detach", path);
}

/** A Git clone at `path` with one commit and `origin` at `remote`; Rerail records nothing there, so it has no clone id yet. */
function cloneAt(path: string, remote: string): string {
  mkdirSync(path, { recursive: true });
  const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", ...args], { cwd: path, stdio: "ignore" });
  git("init", "-q", "-b", "main");
  git("remote", "add", "origin", remote);
  git("commit", "-q", "--allow-empty", "-m", "first");
  return path;
}

/**
 * The repository each read of `checkout` names, as the status line (GET /api/presentation) and the dashboard's
 * repository page and rules list read it; null where a read names none.
 */
async function namedRepositories(app: ReturnType<typeof createRerailApi>, checkout: string) {
  const read = async (path: string, query: Record<string, string>) => {
    const response = await app.request(`${path}?${new URLSearchParams(query)}`);
    expect(response.status, path).toBe(200);
    return await response.json() as { scope?: { repositoryKey: string } | null; repository?: { repositoryKey: string }; repositoryKey?: string };
  };
  const presentation = await read("/api/presentation", { repository: checkout, host: "claude-code" });
  const page = await read("/api/repository", { checkout });
  const rules = await read("/api/repository/rules", { checkout });
  return { statusLine: presentation.scope?.repositoryKey ?? null, page: page.repository?.repositoryKey ?? null, rules: rules.repositoryKey ?? null };
}

/** What {@link namedRepositories} gives when every read names `repositoryKey`. */
const allNaming = (repositoryKey: string) => ({ statusLine: repositoryKey, page: repositoryKey, rules: repositoryKey });

/** The state the dashboard gives `checkout` (GET /api/repository) and the one the status line and the runtime give it. */
async function shownStates(app: ReturnType<typeof createRerailApi>, checkout: string) {
  const context = await (await app.request(`/api/repository?${new URLSearchParams({ checkout })}`)).json() as {
    clones: Array<{ checkouts: Array<{ checkoutPath: string; state: string; parentCheckoutPath: string | null }> }>;
    checkout: { state: string } | null;
  };
  const presentation = await (await app.request(`/api/presentation?${new URLSearchParams({ repository: checkout, host: "claude-code" })}`)).json() as {
    repositoryState: string;
  };
  const listed = context.clones.flatMap(clone => clone.checkouts).find(entry => entry.checkoutPath === checkout);
  return { listed: listed === undefined ? null : [listed.state, listed.parentCheckoutPath], dashboard: context.checkout?.state, statusLine: presentation.repositoryState };
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

  it("gives the asked checkout the state Rerail runs it in, from the row at its path even when another repository recorded it", async () => {
    const { db, main, app } = await repositoryWith([]);
    // A folder once recorded as another repository's checkout, stopped there, is now a worktree of this one, not yet recorded again.
    const reused = join(main, "..", "reused");
    const other = "remote:github.com/acme/other";
    db.insert(repositories).values({ repositoryKey: other, displayName: "other", configuredAt: 1, started: true, createdAt: 1, updatedAt: 1 }).run();
    db.insert(checkouts).values({ checkoutPath: reused, repositoryKey: other, cloneId: "old-clone", started: false, createdAt: 1, lastSeenAt: 1 }).run();
    addWorktree(main, reused);
    expect(await shownStates(app, reused)).toEqual({ listed: null, dashboard: "stopped", statusLine: "stopped" });
  });

  it("picks the same containing checkout as the runtime when two recorded ones differ only in letter case, the one recorded first", async () => {
    for (const order of [["wt", "WT"], ["WT", "wt"]] as const) {
      const { db, main, repositoryKey, cloneId, app } = await repositoryWith([]);
      // One folder on Windows, recorded under two spellings a millisecond apart: stopped as wt, started as WT. Elsewhere two folders.
      for (const [index, segment] of order.entries()) {
        db.insert(checkouts).values({ checkoutPath: join(main, segment), repositoryKey, cloneId, started: segment === "WT", createdAt: index + 1, lastSeenAt: 1 }).run();
      }
      const inner = join(main, "wt", "inner");
      addWorktree(main, inner);
      db.insert(checkouts).values({ checkoutPath: inner, repositoryKey, cloneId, started: null, createdAt: 3, lastSeenAt: 1 }).run();
      // Of one folder's spellings the one recorded first holds the checkout, as it did before the readers shared an order.
      const holder = process.platform === "win32" ? order[0] : "wt";
      const state = holder === "WT" ? "started" : "stopped";
      expect(await shownStates(app, inner), order.join(" then ")).toEqual({ listed: [state, join(main, holder)], dashboard: state, statusLine: state });
    }
  });
});

describe("a checkout replaced at its path while the daemon runs", () => {
  /** Two known repositories, each recorded at a clone of its own: `app` (acme/app) and `other` (acme/other). */
  async function twoRepositories() {
    const { root, stores, db, app } = await daemonStores();
    const first = recordCheckout(db, cloneAt(join(root, "app"), "git@github.com:acme/app.git"));
    const second = recordCheckout(db, cloneAt(join(root, "other"), "git@github.com:acme/other.git"));
    return { root, stores, db, api: app, first, second };
  }
  const worktreeAdd = (clone: string, path: string, branch: string) => execFileSync("git", ["worktree", "add", "-q", "-b", branch, path], { cwd: clone, stdio: "ignore" });

  it("names the other clone's repository at once when that clone's worktree replaces the checkout", async () => {
    const { root, api, first, second } = await twoRepositories();
    const replaced = join(root, "app");
    expect(await namedRepositories(api, replaced)).toEqual(allNaming(first.repositoryKey));

    await removeFolder(replaced);
    worktreeAdd(second.checkoutPath, replaced, "side");

    expect(await namedRepositories(api, replaced)).toEqual(allNaming(second.repositoryKey));
  });

  it("names the other clone's repository at once when that clone's worktree replaces a worktree, whose clone id stays", async () => {
    const { root, db, api, first, second } = await twoRepositories();
    const replaced = join(root, "feature");
    worktreeAdd(first.checkoutPath, replaced, "feature");
    recordCheckout(db, replaced);
    expect(await namedRepositories(api, replaced)).toEqual(allNaming(first.repositoryKey));

    await removeFolder(replaced);
    worktreeAdd(second.checkoutPath, replaced, "side");

    expect(await namedRepositories(api, replaced)).toEqual(allNaming(second.repositoryKey));
  });

  it("names the repository a new clone's remote names, not the one the clone it replaced named, before either is recorded", async () => {
    const { root, api, first, second } = await twoRepositories();
    const replaced = join(root, "fresh");
    cloneAt(replaced, "git@github.com:acme/app.git");
    expect(await namedRepositories(api, replaced)).toEqual(allNaming(first.repositoryKey));

    await removeFolder(replaced);
    cloneAt(replaced, "git@github.com:acme/other.git");

    expect(await namedRepositories(api, replaced)).toEqual(allNaming(second.repositoryKey));
  });

  it("names the repository a new clone's remote names, not the removed clone's recorded there, in the daemon and in a lookup", async () => {
    const { root, db, api, first, second } = await twoRepositories();
    const replaced = first.checkoutPath;
    expect(await namedRepositories(api, replaced)).toEqual(allNaming(first.repositoryKey));

    // A new clone has no clone id until Rerail records it; the row at its path is the removed clone's.
    await removeFolder(replaced);
    cloneAt(replaced, "git@github.com:acme/other.git");

    expect(await namedRepositories(api, replaced)).toEqual(allNaming(second.repositoryKey));
    expect(lookupCheckout(db, replaced)?.repositoryKey).toBe(second.repositoryKey);
  });

  /** Checkouts whose Git folder, which holds the clone id, is not their own `.git` and outlives them, made in `root`. */
  const keptApart: Record<string, (root: string, source: string) => string> = {
    "a submodule, whose Git folder is in its parent's": (root, source) => {
      const parent = cloneAt(join(root, "parent"), "git@github.com:acme/parent.git");
      execFileSync("git", ["-c", "protocol.file.allow=always", "submodule", "add", "-q", source, "lib"], { cwd: parent, stdio: "ignore" });
      return join(parent, "lib");
    },
    "a clone made with --separate-git-dir": root => {
      const path = join(root, "separate");
      execFileSync("git", ["init", "-q", "-b", "main", `--separate-git-dir=${join(root, "separate.git")}`, path], { cwd: root, stdio: "ignore" });
      return path;
    },
  };
  for (const [layout, make] of Object.entries(keptApart)) {
    it(`names the new clone's repository at once when a clone replaces ${layout}`, async () => {
      const { root, db, api, first, second } = await twoRepositories();
      const replaced = make(root, first.checkoutPath);
      const recorded = recordCheckout(db, replaced).repositoryKey;
      expect(await namedRepositories(api, replaced)).toEqual(allNaming(recorded));

      await removeFolder(replaced);
      cloneAt(replaced, "git@github.com:acme/other.git");

      expect(await namedRepositories(api, replaced)).toEqual(allNaming(second.repositoryKey));
    });
  }

  it("keeps an unchanged checkout's location and remote: its second read starts no process, places nothing, reads no remote", async () => {
    const { root, stores, db, first } = await twoRepositories();
    const worktree = join(root, "feature");
    worktreeAdd(first.checkoutPath, worktree, "feature");
    recordCheckout(db, worktree);
    // Not recorded: the resolver names its repository by its remote.
    const unrecorded = cloneAt(join(root, "fresh"), "git@github.com:acme/app.git");
    // The daemon's own resolver, which its reads of a folder go through.
    const resolveCheckout = stores.checkouts();
    for (const checkout of [first.checkoutPath, worktree, unrecorded]) {
      expect(resolveCheckout(checkout)?.repositoryKey, checkout).toBe(first.repositoryKey);
      Object.assign(reads, { placements: 0, remotes: 0, starts: 0, counting: true });
      const again = resolveCheckout(checkout)?.repositoryKey;
      reads.counting = false;
      expect({ again, placements: reads.placements, remotes: reads.remotes, starts: reads.starts }, checkout)
        .toEqual({ again: first.repositoryKey, placements: 0, remotes: 0, starts: 0 });
    }
  });
});

describe("a remote changed in place in a checkout not recorded, while the daemon runs", () => {
  const APP = "git@github.com:acme/app.git";
  const OTHER = "git@github.com:acme/other.git";
  const gitIn = (cwd: string, ...args: string[]) => execFileSync("git", args, { cwd, stdio: "ignore" });

  /** Two known repositories, acme/app and acme/other, each recorded at a clone of its own, and the daemon's own resolver. */
  async function twoRepositories() {
    const { root, stores, db, app } = await daemonStores();
    const first = recordCheckout(db, cloneAt(join(root, "app"), APP)).repositoryKey;
    const second = recordCheckout(db, cloneAt(join(root, "other"), OTHER)).repositoryKey;
    return { root, api: app, first, second, resolveCheckout: stores.checkouts() };
  }

  /** One read of `checkout` through `resolveCheckout`: the repository it names, the remotes it read, the processes it started and the files it stamped. */
  function counted(resolveCheckout: (workspace: string) => { repositoryKey: string } | undefined, checkout: string) {
    Object.assign(reads, { remotes: 0, starts: 0, stamps: 0, counting: true });
    try {
      const named = resolveCheckout(checkout)?.repositoryKey;
      return { named, remotes: reads.remotes, starts: reads.starts, stamps: reads.stamps };
    } finally {
      reads.counting = false;
    }
  }

  it("names the new remote's repository at the next read, and asks Git for that read alone", async () => {
    const { root, api, first, second, resolveCheckout } = await twoRepositories();
    const checkout = cloneAt(join(root, "fresh"), APP);
    expect(await namedRepositories(api, checkout)).toEqual(allNaming(first));
    expect(counted(resolveCheckout, checkout)).toEqual({ named: first, remotes: 0, starts: 0, stamps: 0 });

    gitIn(checkout, "remote", "set-url", "origin", OTHER);

    expect(counted(resolveCheckout, checkout)).toEqual({ named: second, remotes: 1, starts: 1, stamps: 0 });
    expect(counted(resolveCheckout, checkout)).toEqual({ named: second, remotes: 0, starts: 0, stamps: 0 });
    expect(await namedRepositories(api, checkout)).toEqual(allNaming(second));
  });

  /** Checkouts whose remotes are in a Git folder that is not their own `.git`, made in `root`, with no remote yet. */
  const layouts: Record<string, (root: string) => string> = {
    "a linked worktree, whose remotes are in its clone's common config": root => {
      gitIn(cloneAt(join(root, "clone"), "https://example.invalid/clone.git"), "worktree", "add", "-q", "-b", "side", join(root, "linked"));
      gitIn(join(root, "linked"), "remote", "remove", "origin");
      return join(root, "linked");
    },
    "a submodule, whose Git folder is in its parent's": root => {
      const parent = cloneAt(join(root, "parent"), "https://example.invalid/parent.git");
      const source = cloneAt(join(root, "source"), "https://example.invalid/source.git");
      gitIn(parent, "-c", "protocol.file.allow=always", "submodule", "add", "-q", source, "lib");
      gitIn(join(parent, "lib"), "remote", "remove", "origin");
      return join(parent, "lib");
    },
    "a clone made with --separate-git-dir": root => {
      gitIn(root, "init", "-q", "-b", "main", `--separate-git-dir=${join(root, "separate.git")}`, join(root, "separate"));
      return join(root, "separate");
    },
  };
  for (const [layout, make] of Object.entries(layouts)) {
    it(`names the new remote's repository at the next read in ${layout}`, async () => {
      const { root, api, first, second } = await twoRepositories();
      const checkout = make(root);
      gitIn(checkout, "remote", "add", "origin", APP);
      expect(await namedRepositories(api, checkout)).toEqual(allNaming(first));

      gitIn(checkout, "remote", "set-url", "origin", OTHER);

      expect(await namedRepositories(api, checkout)).toEqual(allNaming(second));
    });
  }

  it("reads the remote again when only a linked worktree's own config.worktree changed, where extensions.worktreeConfig is on", async () => {
    const { root, api, first, second } = await twoRepositories();
    const clone = cloneAt(join(root, "clone"), APP);
    gitIn(clone, "remote", "remove", "origin");
    gitIn(clone, "config", "extensions.worktreeConfig", "true");
    const linked = join(root, "linked");
    gitIn(clone, "worktree", "add", "-q", "-b", "side", linked);
    gitIn(linked, "config", "--worktree", "remote.origin.url", APP);
    expect(await namedRepositories(api, linked)).toEqual(allNaming(first));

    gitIn(linked, "config", "--worktree", "remote.origin.url", OTHER);

    expect(await namedRepositories(api, linked)).toEqual(allNaming(second));
  });

  it("keeps the remote while a config file whose text cannot be read keeps its size, time and file id, and reads it again once one changes", async () => {
    const { root, first, resolveCheckout } = await twoRepositories();
    const checkout = cloneAt(join(root, "fresh"), APP);
    // Larger than Rerail reads a config file, where Git, with extensions.worktreeConfig off, reads no file.
    const file = join(checkout, ".git", "config.worktree");
    const large = "#".repeat(2 ** 20 + 1);
    writeFileSync(file, large);
    expect(resolveCheckout(checkout)?.repositoryKey).toBe(first);
    const kept = { named: first, remotes: 0, starts: 0, stamps: 1 };
    const readAgain = { named: first, remotes: 1, starts: 1, stamps: 1 };
    const twice = () => [counted(resolveCheckout, checkout), counted(resolveCheckout, checkout)];
    expect(twice()).toEqual([kept, kept]);

    utimesSync(file, 1000, 1000); // Its time alone.
    expect(twice()).toEqual([readAgain, kept]);
    appendFileSync(file, "#"); // Its size alone.
    utimesSync(file, 1000, 1000);
    expect(twice()).toEqual([readAgain, kept]);
    writeFileSync(`${file}.new`, `${large}#`); // Its file id alone.
    utimesSync(`${file}.new`, 1000, 1000);
    renameSync(`${file}.new`, file);
    expect(twice()).toEqual([readAgain, kept]);
    loopInPlace(file); // A file that cannot even be looked at.
    expect(twice()).toEqual([readAgain, kept]);
  });
});
