import * as childProcess from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });
vi.mock("../../../../src/engine/catalog/checkouts/declared-submodules.js", { spy: true });
vi.mock("../../../../src/engine/native/native.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/native/native.js")>();
  return { ...actual, pathsOwnedByCurrentUser: vi.fn(actual.pathsOwnedByCurrentUser) };
});

import { placeCheckout, PLACING_AT_ONCE } from "../../../../src/engine/catalog/checkouts/checkout-placement.js";
import { learnDeclaredSubmodules } from "../../../../src/engine/catalog/checkouts/declared-submodules.js";
import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { readRemoteAddress, repositoryIdentityLocation, worktreeRegistry } from "../../../../src/engine/catalog/repository-identity.js";
import { standingGitAccess } from "../../../../src/engine/native/git-access.js";
import { pathsOwnedByCurrentUser } from "../../../../src/engine/native/native.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";

const folders: string[] = [];
const restores: Array<() => void> = [];
afterEach(() => {
  vi.unstubAllEnvs();
  vi.mocked(pathsOwnedByCurrentUser).mockReset();
  for (const restore of restores.splice(0)) restore();
  for (const folder of folders.splice(0)) removeFolderSync(folder);
});

/** Makes Rerail's owner read say another account owns every folder, as `chown` or `takeown` would; Git still reads the user's. */
function takeOwnersAway(): void {
  if (process.platform === "win32") {
    vi.mocked(pathsOwnedByCurrentUser).mockReturnValue(false);
    return;
  }
  const spy = vi.spyOn(process, "geteuid").mockReturnValue((process.geteuid?.() ?? 0) + 1);
  restores.push(() => spy.mockRestore());
}

function git(cwd: string, args: readonly string[]): string {
  return childProcess.execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], { cwd, encoding: "utf8" }).trim();
}

function temporaryRoot(): string {
  const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-placement-")));
  folders.push(root);
  return root;
}

/** A repository at `path` with one commit; its real path. */
function repository(path: string): string {
  mkdirSync(path, { recursive: true });
  git(path, ["init", "--quiet", "-b", "main"]);
  writeFileSync(join(path, "a.py"), "x = 1\n");
  git(path, ["add", "a.py"]);
  git(path, ["commit", "--quiet", "-m", "one"]);
  return realpathSync.native(path);
}

/** A repository with a declared submodule and `count` linked worktrees beside it, each holding the submodule's declaration; the main checkout first. */
function repositoryWithWorktrees(count: number): { root: string; checkouts: string[] } {
  const root = temporaryRoot();
  const library = repository(join(root, "library"));
  const main = repository(join(root, "main"));
  git(main, ["submodule", "add", "--quiet", library, "lib"]);
  git(main, ["commit", "--quiet", "-m", "lib"]);
  // Declared and in the index, not checked out: the checkout declares it, and no checkout of it is there to confirm.
  git(main, ["submodule", "deinit", "--quiet", "--force", "lib"]);
  const worktrees = Array.from({ length: count }, (_, i) => join(root, `wt-${i}`));
  for (const [i, worktree] of worktrees.entries()) git(main, ["worktree", "add", "--quiet", "-b", `wt-${i}`, worktree]);
  return { root, checkouts: [main, ...worktrees] };
}

/** The Git processes started while `act` ran: blocking the caller (`spawnSync`) or not (`spawn`), by their arguments. */
async function gitStarts(act: () => unknown): Promise<{ blocking: string[][]; notBlocking: string[][] }> {
  const blocking = vi.mocked(childProcess.spawnSync);
  const started = vi.mocked(childProcess.spawn);
  const [blockingBefore, startedBefore] = [blocking.mock.calls.length, started.mock.calls.length];
  await act();
  const gitOnly = (calls: readonly unknown[][]): string[][] => calls
    .filter(([file]) => /^git(\.exe)?$/iu.test(basename(String(file))))
    .map(([, args]) => (args as string[]).filter((arg, i, all) => arg !== "-c" && all[i - 1] !== "-c" && all[i - 1] !== "-C"));
  return { blocking: gitOnly(blocking.mock.calls.slice(blockingBefore)), notBlocking: gitOnly(started.mock.calls.slice(startedBefore)) };
}

/** How many `git worktree list` processes have started that the caller did not wait on. */
function gitLists(): number {
  return vi.mocked(childProcess.spawn).mock.calls.filter(([file, args]) => /^git(\.exe)?$/iu.test(basename(String(file))) && (args as string[]).includes("worktree")).length;
}

/**
 * How many checkouts' submodules `act` learnt (one per checkout the pass placed, the top level's first), with `onCall`
 * told the count at each call before it runs.
 */
async function learntWhile(act: () => Promise<unknown>, onCall: (calls: number) => void): Promise<number> {
  const learn = vi.mocked(learnDeclaredSubmodules);
  const { learnDeclaredSubmodules: actual } = await vi.importActual<typeof import("../../../../src/engine/catalog/checkouts/declared-submodules.js")>(
    "../../../../src/engine/catalog/checkouts/declared-submodules.js",
  );
  const before = learn.mock.calls.length;
  learn.mockImplementation(async (root, options) => {
    onCall(learn.mock.calls.length - before);
    return await actual(root, options);
  });
  try {
    await act();
  } finally {
    learn.mockImplementation(actual);
  }
  return learn.mock.calls.length - before;
}

/** What the first events of a session ask of each checkout: where it sits, its registry, its remote and its submodules. */
function firstEventAsks(checkouts: readonly string[]): void {
  const owners = new CheckoutOwners();
  for (const checkout of checkouts) {
    expect(repositoryIdentityLocation(checkout).git).toBe(true);
    expect(worktreeRegistry(checkout)?.worktrees).toHaveLength(checkouts.length);
    expect(readRemoteAddress(checkout)).toBeUndefined();
    expect(owners.declaredSubmodules(checkout)).toEqual([]);
  }
}

describe("placeCheckout", () => {
  it("learns what a checkout's first events ask Git, for it and every worktree Git lists, with no Git the caller waits on", async () => {
    const { checkouts } = repositoryWithWorktrees(3);
    const placing = await gitStarts(() => placeCheckout(checkouts[1]!));
    expect(placing.blocking).toEqual([]);
    expect(placing.notBlocking.length).toBeGreaterThan(0);

    expect(await gitStarts(() => firstEventAsks(checkouts))).toEqual({ blocking: [], notBlocking: [] });
    // A session opening there later finds each answer kept, its submodules' among them.
    expect(await gitStarts(() => placeCheckout(checkouts[2]!))).toEqual({ blocking: [], notBlocking: [] });
  });

  it("asks Git again once what an answer rested on changes: a remote added, a submodule checked out", async () => {
    const { root, checkouts } = repositoryWithWorktrees(1);
    const [main, worktree] = checkouts as [string, string];
    await placeCheckout(main);
    expect(await gitStarts(() => firstEventAsks(checkouts))).toEqual({ blocking: [], notBlocking: [] });

    git(main, ["remote", "add", "origin", "https://example.invalid/team/main.git"]);
    expect(readRemoteAddress(worktree)).toEqual({ name: "origin", url: "https://example.invalid/team/main.git" });
    git(main, ["-c", "protocol.file.allow=always", "submodule", "update", "--init", "--quiet"]);
    expect(new CheckoutOwners().declaredSubmodules(main)).toEqual([join(main, "lib")]);
    git(main, ["rm", "--quiet", "-r", "--cached", "lib"]);
    expect(new CheckoutOwners().declaredSubmodules(main)).toEqual([]);
    expect(root).toBeTruthy();
  });

  it("asks Git each time for the remote of a configuration that includes another file, which no kept answer reads", async () => {
    const root = temporaryRoot();
    const main = repository(join(root, "main"));
    const included = join(root, "remote.gitconfig");
    writeFileSync(included, "[remote \"origin\"]\n\turl = https://example.invalid/team/one.git\n");
    git(main, ["config", "include.path", included]);
    expect(readRemoteAddress(main)).toEqual({ name: "origin", url: "https://example.invalid/team/one.git" });
    writeFileSync(included, "[remote \"origin\"]\n\turl = https://example.invalid/team/two.git\n");
    expect(readRemoteAddress(main)).toEqual({ name: "origin", url: "https://example.invalid/team/two.git" });
  });

  it("asks Git again once the .gitmodules file stops declaring a submodule, with nothing staged", async () => {
    const { checkouts } = repositoryWithWorktrees(0);
    const [main] = checkouts as [string];
    git(main, ["-c", "protocol.file.allow=always", "submodule", "update", "--init", "--quiet"]);
    expect(new CheckoutOwners().declaredSubmodules(main)).toEqual([join(main, "lib")]);
    // Git reads the file as it stands; the index still holds the submodule.
    writeFileSync(join(main, ".gitmodules"), "");
    expect(new CheckoutOwners().declaredSubmodules(main)).toEqual([]);
  });

  it("starts at most four Git processes at once for one placement of a repository's worktrees", async () => {
    const root = temporaryRoot();
    const main = repository(join(root, "main"));
    // Git reads a worktree's own config once this is set, so each worktree's placement asks Git.
    git(main, ["config", "extensions.worktreeConfig", "true"]);
    const worktrees = Array.from({ length: 20 }, (_, i) => join(root, `wt-${i}`));
    for (const [i, worktree] of worktrees.entries()) git(main, ["worktree", "add", "--quiet", "-b", `wt-${i}`, worktree]);
    const spawn = vi.mocked(childProcess.spawn);
    const { spawn: realSpawn } = await vi.importActual<typeof childProcess>("node:child_process");
    let running = 0;
    let most = 0;
    spawn.mockImplementation((...args: Parameters<typeof childProcess.spawn>) => {
      const child = realSpawn(...args);
      if (/^git(\.exe)?$/iu.test(basename(String(args[0]))) && String(args[1]).includes("--show-toplevel")) {
        most = Math.max(most, (running += 1));
        child.once("exit", () => (running -= 1));
      }
      return child;
    });
    try {
      const placing = await gitStarts(() => placeCheckout(main));
      expect(placing.notBlocking.filter(args => args.includes("--show-toplevel")).length).toBeGreaterThanOrEqual(worktrees.length);
      expect(most).toBeGreaterThan(1);
      expect(most).toBeLessThanOrEqual(PLACING_AT_ONCE);
    } finally {
      spawn.mockImplementation(realSpawn);
    }
  });

  it("places a repository's worktrees once for sessions that open there together", async () => {
    const { checkouts } = repositoryWithWorktrees(6);
    const [learnt, lists] = [vi.mocked(learnDeclaredSubmodules).mock.calls.length, gitLists()];
    await Promise.all([placeCheckout(checkouts[0]!), placeCheckout(checkouts[0]!), placeCheckout(checkouts[0]!)]);
    // One Git list, and each listed checkout's submodules learnt once by the pass the three sessions share (the
    // session's own checkout once per session besides).
    expect(gitLists() - lists).toBe(1);
    expect(vi.mocked(learnDeclaredSubmodules).mock.calls.length - learnt).toBe(checkouts.length + 3);
  });

  it("starts at most four Git processes at once to place the submodules checked out in a repository's checkouts", async () => {
    const root = temporaryRoot();
    const main = repository(join(root, "main"));
    const paths = Array.from({ length: 6 }, (_, i) => `lib-${i}`);
    writeFileSync(join(main, ".gitmodules"), paths.map(path => `[submodule "${path}"]\n\tpath = ${path}\n\turl = ./${path}\n`).join(""));
    const commit = git(main, ["rev-parse", "HEAD"]);
    for (const path of paths) git(main, ["update-index", "--add", "--cacheinfo", `160000,${commit},${path}`]);
    git(main, ["add", ".gitmodules"]);
    git(main, ["commit", "--quiet", "-m", "libs"]);
    const linked = join(root, "linked");
    git(main, ["worktree", "add", "--quiet", "-b", "linked", linked]);
    // Each a repository of its own, which Git places by asking it: the main checkout's and the linked worktree's.
    for (const checkout of [main, linked]) for (const path of paths) git(checkout, ["init", "--quiet", path]);
    const spawn = vi.mocked(childProcess.spawn);
    const { spawn: realSpawn } = await vi.importActual<typeof childProcess>("node:child_process");
    let [running, most] = [0, 0];
    spawn.mockImplementation((...args: Parameters<typeof childProcess.spawn>) => {
      const child = realSpawn(...args);
      if (/^git(\.exe)?$/iu.test(basename(String(args[0]))) && String(args[1]).includes("--show-toplevel")) {
        most = Math.max(most, (running += 1));
        child.once("exit", () => (running -= 1));
      }
      return child;
    });
    try {
      const placing = await gitStarts(() => placeCheckout(main));
      // The main checkout, then each submodule in it and in the linked worktree.
      expect(placing.notBlocking.filter(args => args.includes("--show-toplevel")).length).toBeGreaterThanOrEqual(1 + 2 * paths.length);
      expect(most).toBeLessThanOrEqual(PLACING_AT_ONCE);
    } finally {
      spawn.mockImplementation(realSpawn);
    }
  });

  it("places no further worktree once its signal is aborted midway", async () => {
    const { checkouts } = repositoryWithWorktrees(12);
    const stopping = new AbortController();
    const learnt = await learntWhile(() => placeCheckout(checkouts[0]!, { signal: stopping.signal }), calls => {
      // The second is the first listed checkout's: Rerail stops once the pass over the worktrees has begun.
      if (calls === 2) stopping.abort();
    });
    // The top level's, then the checkouts the four workers had already begun.
    expect(learnt).toBeLessThanOrEqual(1 + PLACING_AT_ONCE);
  });

  it("gives the event loop a turn while it places worktrees from their files with no Git to wait on", async () => {
    const root = temporaryRoot();
    const main = repository(join(root, "main"));
    const worktrees = Array.from({ length: 36 }, (_, i) => join(root, `wt-${i}`));
    for (const [i, worktree] of worktrees.entries()) git(main, ["worktree", "add", "--quiet", "-b", `wt-${i}`, worktree]);
    let [seen, turnAt]: [number, number | undefined] = [0, undefined];
    const learnt = await learntWhile(() => placeCheckout(main), calls => {
      seen = calls;
      // Queued as the pass begins, as a request would arrive: it runs at the pass's first turn of the event loop.
      if (calls === 2) setImmediate(() => (turnAt ??= seen));
    });
    expect(turnAt).toBeDefined();
    expect(turnAt).toBeLessThan(learnt);
  });

  it("asks Git nothing while a lookup variable moves Git's lookup, as no placement holds Git's answer then", async () => {
    const { checkouts } = repositoryWithWorktrees(4);
    const [main] = checkouts as [string];
    for (const [name, value] of [["GIT_OBJECT_DIRECTORY", join(main, ".git", "objects")], ["GIT_DIR", join(main, ".git")]] as const) {
      vi.stubEnv(name, value);
      expect(await gitStarts(() => placeCheckout(checkouts[1]!)), name).toEqual({ blocking: [], notBlocking: [] });
      vi.unstubAllEnvs();
    }
  });

  it("asks Git nothing for a checkout another account owns, each of whose placements asks Git", async () => {
    const { checkouts } = repositoryWithWorktrees(4);
    const subfolder = join(checkouts[1]!, "sub");
    mkdirSync(subfolder);
    takeOwnersAway();
    // No owner of the engine's Git reads stands, so a remote read ahead would read Git's installation first.
    expect(standingGitAccess()).toBeUndefined();
    expect(await gitStarts(() => placeCheckout(checkouts[1]!))).toEqual({ blocking: [], notBlocking: [] });
    expect(await gitStarts(() => placeCheckout(checkouts[0]!))).toEqual({ blocking: [], notBlocking: [] });
    // A session in a subfolder: Git places the subfolder, whose answer names the checkout above, and nothing more.
    const placing = await gitStarts(() => placeCheckout(subfolder));
    expect({ blocking: placing.blocking, placements: placing.notBlocking.map(args => args.includes("--show-toplevel")) }).toEqual({ blocking: [], placements: [true] });
  });

  it("returns once every Git it started has exited when one of its reads fails, and places the others' worktrees", async () => {
    const { checkouts } = repositoryWithWorktrees(4);
    const [main, failing] = [checkouts[0]!, checkouts[1]!];
    // The top level's submodule read and one worktree's fail at once, while the worktree list and the remote ask Git.
    const learnt = await learntWhile(async () => {
      vi.mocked(learnDeclaredSubmodules).mockImplementation(async (root, options) => {
        if (root === main || root === failing) throw new Error("the submodule read failed");
        const { learnDeclaredSubmodules: actual } = await vi.importActual<typeof import("../../../../src/engine/catalog/checkouts/declared-submodules.js")>(
          "../../../../src/engine/catalog/checkouts/declared-submodules.js",
        );
        return await actual(root, options);
      });
      const spawn = vi.mocked(childProcess.spawn);
      const before = spawn.mock.calls.length;
      await placeCheckout(main);
      const placedAt = spawn.mock.calls.length;
      const running = spawn.mock.results.slice(before, placedAt).filter((result, i) => {
        const child = result.value as childProcess.ChildProcess;
        return /^git(\.exe)?$/iu.test(basename(String(spawn.mock.calls[before + i]![0]))) && child.exitCode === null && child.signalCode === null;
      });
      await new Promise(resolve => setTimeout(resolve, 200));
      expect({ running: running.length, startedAfter: spawn.mock.calls.length - placedAt }).toEqual({ running: 0, startedAfter: 0 });
    }, () => undefined);
    // The top level, then each checkout Git lists, the top level and the failing worktree among them.
    expect(learnt).toBe(1 + checkouts.length);
  });

  it("starts no Git once its signal is aborted, and fails nothing for a folder that is gone or outside Git", async () => {
    const { root, checkouts } = repositoryWithWorktrees(2);
    const stopped = new AbortController();
    stopped.abort();
    expect(await gitStarts(() => placeCheckout(checkouts[0]!, { signal: stopped.signal }))).toEqual({ blocking: [], notBlocking: [] });

    const gone = join(root, "gone");
    mkdirSync(gone);
    removeFolderSync(gone);
    await expect(placeCheckout(gone)).resolves.toBeUndefined();
    const outside = join(temporaryRoot(), "plain");
    mkdirSync(outside);
    expect(await gitStarts(() => placeCheckout(outside))).toEqual({ blocking: [], notBlocking: [] });
  });
});
