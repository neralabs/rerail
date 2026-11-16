/**
 * A pruning pass over a checkout on a share that does not answer. The filesystem is faked under one folder, the share:
 * an asynchronous call there never answers, as a request to a share that is down can wait for the system's whole
 * timeout or for ever, and a blocking call there holds its thread {@link HANG_MS} before it fails. Git listing the
 * clone's worktrees, which looks at each one, takes as long. The tests measure how long the real thread is held, so they
 * run on the real clock: fake timers would not see a held thread.
 */
import { execFileSync } from "node:child_process";
import type * as Fs from "node:fs";
import { mkdtempSync, realpathSync } from "node:fs";
import type * as FsPromises from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as Git from "../../../../src/engine/git.js";
import { listRepositories, pruneCheckouts, recordCheckout } from "../../../../src/engine/catalog/repository-registry.js";
import { openRerailDatabase } from "../../../../src/engine/storage/database.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const HANG_MS = 1_000;
/** The longest a pass may hold the thread it runs on, well below {@link HANG_MS}. */
const HELD_BOUND_MS = 250;

const share = vi.hoisted(() => ({ root: "\0", waiting: 0 }));

function onShare(path: unknown): boolean {
  return String(path).startsWith(share.root);
}

/** Holds the thread {@link HANG_MS}, as a blocking call to a share that does not answer does. */
function holdThread(): void {
  const until = Date.now() + HANG_MS;
  while (Date.now() < until) { /* the thread is held */ }
}

/** {@link holdThread}, then the error a share that timed out gives. */
function holdThenFail(): never {
  holdThread();
  throw Object.assign(new Error("share does not answer"), { code: "ETIMEDOUT" });
}

vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FsPromises>();
  // Each call to the share takes one of the pool's threads and never gives it back.
  const unanswered = <A extends unknown[], R>(call: (path: unknown, ...rest: A) => Promise<R>) =>
    (path: unknown, ...rest: A): Promise<R> => {
      if (!onShare(path)) return call(path, ...rest);
      share.waiting += 1;
      return Promise.withResolvers<R>().promise;
    };
  return {
    ...actual,
    stat: unanswered(actual.stat as (path: unknown) => Promise<unknown>),
    readFile: unanswered(actual.readFile as (path: unknown, options?: unknown) => Promise<unknown>),
    realpath: unanswered(actual.realpath as (path: unknown) => Promise<unknown>),
  };
});

vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof Fs>();
  const held = <A extends unknown[], R>(call: (path: unknown, ...rest: A) => R) =>
    (path: unknown, ...rest: A): R => (onShare(path) ? holdThenFail() : call(path, ...rest));
  const realpathSync = Object.assign(held(actual.realpathSync as (path: unknown) => string), {
    native: held(actual.realpathSync.native as (path: unknown) => string),
  });
  return {
    ...actual,
    existsSync: (path: unknown): boolean => {
      if (!onShare(path)) return actual.existsSync(path as string);
      holdThread();
      return false;
    },
    statSync: held(actual.statSync as (path: unknown) => unknown),
    readFileSync: held(actual.readFileSync as (path: unknown, options?: unknown) => unknown),
    realpathSync,
  };
});

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof Git>();
  // Git lists the clone's worktrees as it looks at each, the one on the share too.
  const listing = (args: readonly string[]): boolean => share.root !== "\0" && args[0] === "worktree" && args[1] === "list";
  return {
    ...actual,
    runGit: (cwd: string, args: readonly string[], options?: Git.RunGitOptions) => {
      if (listing(args)) holdThread();
      return actual.runGit(cwd, args, options);
    },
    runGitAsync: async (cwd: string, args: readonly string[], options?: Git.RunGitOptions & { input?: string }) => {
      if (listing(args)) await sleep(HANG_MS);
      return await actual.runGitAsync(cwd, args, options);
    },
  };
});

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  share.root = "\0";
  share.waiting = 0;
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** A folder with a clone at `main`, its database, and a worktree of it at `gone` that Git removed. */
function cloneWithRemovedWorktree() {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-prune-share-")));
  cleanups.push(() => removeFolder(root));
  const main = join(root, "main");
  git(root, "init", "-q", "-b", "main", main);
  git(main, "commit", "-q", "--allow-empty", "-m", "base");
  const database = openRerailDatabase(join(root, "rerail.sqlite"));
  cleanups.push(() => { database.$client.close(); });
  const gone = join(root, "gone");
  git(main, "worktree", "add", "-q", "-b", "gone", gone);
  recordCheckout(database, main);
  recordCheckout(database, gone);
  git(main, "worktree", "remove", gone);
  const recorded = (): string[] => listRepositories(database).flatMap(repository => repository.checkouts.map(row => row.checkoutPath)).sort();
  return { root, main, gone, database, recorded };
}

/** The longest the thread went without running a timer, from now until the returned function is called. */
function longestHold(): () => number {
  let last = performance.now();
  let longest = 0;
  const timer = setInterval(() => {
    const now = performance.now();
    longest = Math.max(longest, now - last);
    last = now;
  }, 10);
  return () => {
    clearInterval(timer);
    return Math.max(longest, performance.now() - last);
  };
}

async function settledWithin<T>(work: Promise<T>, ms: number): Promise<T | "still waiting"> {
  return await Promise.race([work, sleep(ms, "still waiting" as const)]);
}

describe("a pruning pass over a share that does not answer", () => {
  it("holds no thread while Git lists the clone's worktrees and a worktree it names does not answer", async () => {
    const { root, main, database, recorded } = cloneWithRemovedWorktree();
    git(main, "worktree", "add", "-q", "-b", "away", join(root, "share", "away"));
    const before = recorded();
    share.root = join(root, "share");
    const held = longestHold();
    const abort = new AbortController();

    const pass = pruneCheckouts(database, { signal: abort.signal });
    await sleep(HANG_MS + 300);
    abort.abort();

    const forgotten = await settledWithin(pass, HELD_BOUND_MS);
    expect(held()).toBeLessThan(HELD_BOUND_MS);
    expect(forgotten).toEqual([]);
    // Git answered, and the pass then waited on the worktree it names on the share, one call at a time.
    expect(share.waiting).toBe(1);
    expect(recorded()).toEqual(before);
  }, 10_000);

  it("waits for one recorded folder on the share at a time, and ends at once when it is stopped", async () => {
    const { root, main, database, recorded } = cloneWithRemovedWorktree();
    const away = join(root, "share", "away");
    git(main, "worktree", "add", "-q", "-b", "away", away);
    recordCheckout(database, away);
    const before = recorded();
    share.root = join(root, "share");
    const held = longestHold();
    const abort = new AbortController();

    const pass = pruneCheckouts(database, { signal: abort.signal });
    await sleep(200);
    expect(share.waiting).toBe(1);
    abort.abort();

    const forgotten = await settledWithin(pass, HELD_BOUND_MS);
    expect(held()).toBeLessThan(HELD_BOUND_MS);
    expect(forgotten).toEqual([]);
    expect(recorded()).toEqual(before);
  }, 10_000);

  it("forgets the removed worktree once the share answers again", async () => {
    const { root, main, gone, database, recorded } = cloneWithRemovedWorktree();
    git(main, "worktree", "add", "-q", "-b", "away", join(root, "share", "away"));

    expect(await pruneCheckouts(database)).toEqual([gone]);
    expect(recorded()).toEqual([main]);
  }, 10_000);
});
