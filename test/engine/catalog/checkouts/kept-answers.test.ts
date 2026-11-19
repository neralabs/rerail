// What the catalog keeps of Git's answers for a checkout leaves with the checkout, and a `commondir` is read as Git
// reads it. The daemon keeps each worktree's answer, the answers of folders in it and the config facts its placement
// read, for as long as it runs; a removed worktree's ones are dropped once Git lists its repository's worktrees again
// and the folder is found gone, while every checkout that stays keeps its own.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { physicalPath } from "../../../../src/engine/catalog/checkout-control.js";
import { keptConfigCount } from "../../../../src/engine/catalog/checkouts/git-placement.js";
import {
  keptAnswerCounts,
  repositoryIdentityLocation,
  worktreeRegistry,
  type RepositoryIdentityLocation,
} from "../../../../src/engine/catalog/repository-identity.js";
import { runGit } from "../../../../src/engine/git.js";
import { linkFolder } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit) };
});

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function temporaryRoot(): Promise<string> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-kept-answers-")));
  cleanups.push(() => removeFolder(root));
  return root;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** A repository at `path` with one commit; returns its real path. */
function repository(path: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "--quiet");
  git(path, "commit", "--quiet", "--allow-empty", "-m", "seed");
  return realpathSync.native(path);
}

/** Git's own answer for `folder`, as Rerail reads it from `git rev-parse`. */
function gitsAnswer(folder: string): RepositoryIdentityLocation {
  const requested = resolve(folder);
  const result = spawnSync("git", ["-C", requested, "rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"], { encoding: "utf8" });
  if (result.status !== 0) return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id"), main: true };
  const [top, common, gitDirectory] = result.stdout.trim().split("\n").map(line => resolve(line.trim()));
  return { git: true, workspaceRoot: top!, identityPath: join(common!, "rerail-repository-id"), main: common === gitDirectory };
}

function gitsRegistry(checkout: string): string[] {
  const result = spawnSync("git", ["-C", checkout, "worktree", "list", "--porcelain"], { encoding: "utf8" });
  return result.stdout.split("\n").filter(line => line.startsWith("worktree ")).map(line => line.slice("worktree ".length));
}

/** Rerail's registry of each checkout, the common directory it names and the checkout's placement, against Git's. */
function expectGitsRegistries(...checkouts: string[]): void {
  for (const checkout of checkouts) {
    const registry = worktreeRegistry(checkout);
    expect(registry?.worktrees, `the registry at ${checkout}`).toEqual(gitsRegistry(checkout));
    expect(registry?.commonDirectory, `the common directory at ${checkout}`).toBe(physicalPath(dirname(gitsAnswer(checkout).identityPath)));
    expect(repositoryIdentityLocation(checkout), `the placement of ${checkout}`).toEqual(gitsAnswer(checkout));
  }
}

/** What the catalog keeps now: each map of Git's answers, and the config files' facts. */
function kept(): Record<string, number> {
  return { ...keptAnswerCounts(), configs: keptConfigCount() };
}

/** What the catalog keeps beyond `before`. */
function keptSince(before: Record<string, number>): Record<string, number> {
  const now = kept();
  return Object.fromEntries(Object.entries(now).map(([name, count]) => [name, count - (before[name] ?? 0)]));
}

/** How many Git processes Rerail started since the `started`th. */
function gitProcessesSince(started: number): number {
  return vi.mocked(runGit).mock.calls.length - started;
}

/**
 * A repository `main` with `count` worktrees beside it, each holding a folder `sub`: with `ownConfig`, the repository
 * reads a `config.worktree` of each worktree's own (`extensions.worktreeConfig`), so each is placed by Git.
 */
async function withWorktrees(count: number, ownConfig: boolean): Promise<{ main: string; worktrees: string[] }> {
  const root = await temporaryRoot();
  const main = repository(join(root, "main"));
  mkdirSync(join(main, "sub"));
  if (ownConfig) git(main, "config", "extensions.worktreeConfig", "true");
  const worktrees = Array.from({ length: count }, (_, index) => {
    const worktree = join(root, `w${index}`);
    git(main, "worktree", "add", "--quiet", "-b", `w${index}`, worktree);
    if (ownConfig) git(worktree, "config", "--worktree", "rerail.test", "own");
    mkdirSync(join(worktree, "sub"));
    return realpathSync.native(worktree);
  });
  return { main, worktrees };
}

/** Asks after each checkout as a start and the worktree registry ask: its list, its placement, and a folder in it. */
function askAfter(...checkouts: string[]): void {
  for (const checkout of checkouts) expectGitsRegistries(checkout, join(checkout, "sub"));
}

describe("what the catalog keeps for a worktree leaves with it", () => {
  it.each([
    ["placed from their files", false],
    ["each placed by Git, with a config.worktree of its own", true],
  ])("worktrees %s, removed with Git, once Git lists the repository's worktrees again", async (_, ownConfig) => {
    const before = kept();
    const { main, worktrees } = await withWorktrees(6, ownConfig);
    askAfter(main, ...worktrees);
    const configsPerCheckout = ownConfig ? 1 : 0;
    expect(keptSince(before)).toEqual({ heldTopLevels: 7, gitAcceptances: 7, gitPlacedCommons: 1, configs: 1 + 6 * configsPerCheckout });

    const [removed, stay] = [worktrees.slice(0, 4), worktrees.slice(4)];
    for (const worktree of removed) git(main, "worktree", "remove", "--force", worktree);
    expectGitsRegistries(main);

    expect(keptSince(before)).toEqual({ heldTopLevels: 3, gitAcceptances: 3, gitPlacedCommons: 1, configs: 1 + 2 * configsPerCheckout });
    // Each checkout that stays keeps its answers: asked again, none starts a Git process.
    const started = vi.mocked(runGit).mock.calls.length;
    askAfter(main, ...stay);
    expect(gitProcessesSince(started)).toBe(0);
  });

  it("worktrees removed by hand, once Git lists the repository's worktrees again though it still lists them", async () => {
    const before = kept();
    const { main, worktrees } = await withWorktrees(4, false);
    askAfter(main, ...worktrees);
    for (const worktree of worktrees.slice(0, 2)) await removeFolder(worktree);
    git(main, "worktree", "add", "--quiet", "-b", "later", join(dirname(main), "later"));

    expect(gitsRegistry(main).map(listed => resolve(listed))).toEqual(expect.arrayContaining(worktrees));
    expectGitsRegistries(main);
    expect(keptSince(before)).toEqual({ heldTopLevels: 3, gitAcceptances: 3, gitPlacedCommons: 1, configs: 1 });
    const started = vi.mocked(runGit).mock.calls.length;
    askAfter(main, ...worktrees.slice(2));
    expect(gitProcessesSince(started)).toBe(0);
  });
});

describe("a commondir naming a link and then `..` is read as Git reads it", () => {
  /**
   * A Git folder at `folder/.git` whose `commondir` is `x/link/../y`. Cleaned as text it names `x/y` in it, which holds
   * `objects` and `refs`; Git on Linux and macOS follows `link` first, to `elsewhere/deep`, and finds no `y` beside
   * `deep`. Git for Windows opens the joined path, and Windows cleans it as text.
   */
  function handWrittenCommondir(folder: string, elsewhere: string): void {
    const gitFolder = join(folder, ".git");
    for (const made of ["objects", "refs"]) mkdirSync(join(gitFolder, "x", "y", made), { recursive: true });
    mkdirSync(join(elsewhere, "deep"), { recursive: true });
    linkFolder(join(elsewhere, "deep"), join(gitFolder, "x", "link"));
    writeFileSync(join(gitFolder, "HEAD"), "ref: refs/heads/main\n");
    writeFileSync(join(gitFolder, "commondir"), "x/link/../y\n");
  }

  it("lists as Git does once a folder between it and the repository Git places a checkout in becomes a repository", async () => {
    const root = await temporaryRoot();
    const outer = repository(join(root, "outer"));
    const mid = join(outer, "between", "mid");
    const checkout = join(mid, "d0");
    mkdirSync(checkout, { recursive: true });
    handWrittenCommondir(mid, join(root, "elsewhere"));
    if (process.platform !== "win32") expect(gitsAnswer(checkout).workspaceRoot, "Git's search goes on past mid").toBe(outer);
    expectGitsRegistries(checkout);

    repository(join(outer, "between"));
    expectGitsRegistries(checkout);
  });

  it("places a linked worktree whose commondir names its repository through a link and then `..` as Git places it", async () => {
    const root = await temporaryRoot();
    const outer = repository(join(root, "outer"));
    const other = repository(join(root, "other"));
    const worktree = join(root, "w");
    git(outer, "worktree", "add", "--quiet", "-b", "w", worktree);
    // Cleaned as text, `lnk/../../..` names `outer/.git` as Git wrote it; through the link it names `other/.git`.
    const deep = join(other, ".git", "p", "q", "r");
    mkdirSync(deep, { recursive: true });
    const gitFolder = join(outer, ".git", "worktrees", "w");
    linkFolder(deep, join(gitFolder, "lnk"));
    writeFileSync(join(gitFolder, "commondir"), "lnk/../../..\n");
    if (process.platform !== "win32") expect(gitsAnswer(worktree).identityPath, "the common Git folder Git names").toBe(join(other, ".git", "rerail-repository-id"));

    expectGitsRegistries(outer, other, worktree, worktree);
  });
});
