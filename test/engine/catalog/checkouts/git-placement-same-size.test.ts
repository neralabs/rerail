// A file Git reads to place a folder, rewritten in place at the same size, can keep the lstat it had: a fast enough
// rewrite does on a machine whose file times are coarser than the rewrite (Linux 6.12 on its second try; NTFS in most
// of 300 tries). Rerail reads the text of each such file at every use, so the text tells the change. Node's lstat is
// wrapped here so a test can hand back the lstat a file had before such a rewrite, or the lstat of an owner changed
// with chown.
import { execFileSync, spawnSync } from "node:child_process";
import { lstatSync, mkdirSync, realpathSync, symlinkSync, type Stats } from "node:fs";
import { mkdtemp, open, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { repositoryIdentityLocation, worktreeRegistry, type RepositoryIdentityLocation } from "../../../../src/engine/catalog/repository-identity.js";
import { runGit } from "../../../../src/engine/git.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit) };
});

vi.mock("node:fs", async importOriginal => {
  const fs = await importOriginal<typeof import("node:fs")>();
  return { ...fs, lstatSync: vi.fn(fs.lstatSync) };
});

const realLstat = (await vi.importActual<typeof import("node:fs")>("node:fs")).lstatSync;

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  vi.mocked(lstatSync).mockReset();
  vi.mocked(lstatSync).mockImplementation(realLstat);
  vi.unstubAllEnvs();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function temporaryRoot(): Promise<string> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-same-size-")));
  cleanups.push(() => removeFolder(root));
  return root;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

function repository(path: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "--quiet");
  git(path, "commit", "--quiet", "--allow-empty", "-m", "seed");
  return realpathSync.native(path);
}

function gitsAnswer(folder: string, env: NodeJS.ProcessEnv = process.env): RepositoryIdentityLocation {
  const requested = resolve(folder);
  const result = spawnSync("git", ["-C", requested, "rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"], { encoding: "utf8", env });
  if (result.status !== 0) return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id"), main: true };
  const [top, common, gitDirectory] = result.stdout.trim().split("\n").map(line => resolve(line.trim()));
  return { git: true, workspaceRoot: top!, identityPath: join(common!, "rerail-repository-id"), main: common === gitDirectory };
}

/** How many Git processes Rerail started to place `folder`. */
function placements(folder: string): number {
  return vi.mocked(runGit).mock.calls.filter(([cwd, args]) => cwd === resolve(folder) && args.includes("--show-toplevel")).length;
}

/**
 * Writes `text` over the file at `path` in place, as Git rewrites a `.git` file. Git for Windows hides a worktree's
 * `.git` file, and Windows refuses to make a file anew over a hidden one.
 */
async function rewriteInPlace(path: string, text: string): Promise<void> {
  const file = await open(path, "r+");
  try {
    await file.truncate(0);
    await file.write(text, 0);
  } finally {
    await file.close();
  }
}

/** `stats` with its owner another account's, as `chown` would leave it. */
function anotherAccounts(stats: Stats): Stats {
  return Object.assign(Object.create(Object.getPrototypeOf(stats) as object) as Stats, stats, { uid: stats.uid + 1 });
}

/** Has every Git process Rerail starts read the folders as another account's, with no safe.directory entry to trust them. */
function gitSeesAnotherOwner(root: string): void {
  vi.stubEnv("GIT_CONFIG_GLOBAL", join(root, "no.gitconfig"));
  vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
  const actual = vi.mocked(runGit).getMockImplementation()!;
  vi.mocked(runGit).mockImplementation((cwd, args, options) => {
    process.env["GIT_TEST_ASSUME_DIFFERENT_OWNER"] = "1";
    try {
      return actual(cwd, args, options);
    } finally {
      delete process.env["GIT_TEST_ASSUME_DIFFERENT_OWNER"];
    }
  });
  cleanups.push(async () => {
    vi.mocked(runGit).mockImplementation(actual);
  });
}

/** From now on, an lstat of `path` gives `stats`, as if nothing had changed there since. */
function lstatKeeps(path: string, stats: Stats): void {
  vi.mocked(lstatSync).mockImplementation(((target: Parameters<typeof lstatSync>[0], options?: Parameters<typeof lstatSync>[1]) =>
    resolve(String(target)) === path ? stats : realLstat(target, options)) as typeof lstatSync);
}

describe("a file Git reads to place a folder, rewritten at the same size with its lstat unchanged, is read by its text", () => {
  it("a worktree's .git file rewritten to name another repository's worktree is placed as Git places it", async () => {
    const root = await temporaryRoot();
    // Two repositories whose worktree Git folders are spelled at the same length, so the rewrite keeps the size.
    const first = repository(join(root, "aa"));
    const other = repository(join(root, "bb"));
    git(first, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    git(other, "worktree", "add", "--quiet", "-b", "w", join(root, "o", "w"));
    const dotGit = join(root, "w", ".git");
    const before = realLstat(dotGit);
    expect(repositoryIdentityLocation(first)).toEqual(gitsAnswer(first));
    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(gitsAnswer(join(root, "w")));

    await rewriteInPlace(dotGit, `gitdir: ${join(other, ".git", "worktrees", "w")}\n`);
    lstatKeeps(dotGit, before);

    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(gitsAnswer(join(root, "w")));
    expect(gitsAnswer(join(root, "w")).identityPath).toBe(join(other, ".git", "rerail-repository-id"));
  });

  it("a repository's config rewritten to make it bare is placed as Git places it", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "aa"));
    const config = join(main, ".git", "config");
    const text = await readFile(config, "utf8");
    const bare = text.replace(/bare = false/u, "bare =  true");
    expect(bare).not.toBe(text);
    const before = realLstat(config);
    expect(repositoryIdentityLocation(main)).toEqual(gitsAnswer(main));

    await rewriteInPlace(config, bare);
    lstatKeeps(config, before);

    expect(repositoryIdentityLocation(main)).toEqual(gitsAnswer(main));
    expect(gitsAnswer(main).git).toBe(false);
  });

  it("keeps a checkout's answer, with no Git process, when its config is rewritten with the same text", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "aa"));
    const config = join(main, ".git", "config");
    const before = realLstat(config);
    const answer = repositoryIdentityLocation(main);
    const placed = placements(main);

    await rewriteInPlace(config, await readFile(config, "utf8"));
    lstatKeeps(config, before);

    expect(repositoryIdentityLocation(main)).toEqual(answer);
    expect({ answer, gitAfterTheFirst: placements(main) - placed }).toEqual({ answer: gitsAnswer(main), gitAfterTheFirst: 0 });
  });
});

describe.skipIf(process.platform === "win32")("a kept answer on POSIX, where an lstat gives each folder's owner", () => {
  it("asks Git at every ask once the checkout's folder is another account's, as Git reads its owner at every start", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "aa"));
    const answer = repositoryIdentityLocation(main);
    const placed = placements(main);
    const stats = realLstat(main);

    lstatKeeps(main, anotherAccounts(stats));
    expect(repositoryIdentityLocation(main)).toEqual(answer);
    expect(repositoryIdentityLocation(main)).toEqual(answer);
    // Git takes another account's folder only through safe.directory, read from files outside the stamp.
    expect({ answer, gitAfterTheFirst: placements(main) - placed }).toEqual({ answer: gitsAnswer(main), gitAfterTheFirst: 2 });
  });

  it("asks Git at every ask once the checkout's Git folder is another account's, as Git reads its owner too", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "aa"));
    const answer = repositoryIdentityLocation(main);
    const placed = placements(main);
    const gitFolder = join(main, ".git");

    lstatKeeps(gitFolder, anotherAccounts(realLstat(gitFolder)));
    expect(repositoryIdentityLocation(main)).toEqual(answer);
    expect(repositoryIdentityLocation(main)).toEqual(answer);
    expect({ answer, gitAfterTheFirst: placements(main) - placed }).toEqual({ answer: gitsAnswer(main), gitAfterTheFirst: 2 });
  });

  it("gives no worktree list for a worktree whose .git file names a link once the Git folder behind it is another account's", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "aa"));
    const worktree = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", worktree);
    const link = join(root, "w.git");
    symlinkSync(join(main, ".git", "worktrees", "w"), link);
    await rewriteInPlace(join(worktree, ".git"), `gitdir: ${link}\n`);
    expect(worktreeRegistry(worktree)?.worktrees).toHaveLength(2);

    const gitFolder = join(main, ".git", "worktrees", "w");
    lstatKeeps(gitFolder, anotherAccounts(realLstat(gitFolder)));
    gitSeesAnotherOwner(root);
    expect(gitsAnswer(worktree, { ...process.env, GIT_TEST_ASSUME_DIFFERENT_OWNER: "1" }).git).toBe(false);
    expect(worktreeRegistry(worktree)).toBeUndefined();
  });

  // Git reads the owner of the folder it changed into by that folder's real path, never a link's own.
  it("asks Git for a worktree reached through a link the user owns whose real folder is another account's", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "aa"));
    const real = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", real);
    const link = join(root, "L");
    symlinkSync(real, link);
    expect(repositoryIdentityLocation(main)).toEqual(gitsAnswer(main));

    lstatKeeps(real, anotherAccounts(realLstat(real)));
    gitSeesAnotherOwner(root);
    const refused = gitsAnswer(link, { ...process.env, GIT_TEST_ASSUME_DIFFERENT_OWNER: "1" });
    expect(refused.git).toBe(false);
    expect(repositoryIdentityLocation(link)).toEqual(refused);
  });

  it("places a worktree reached through a link another account owns from its files when its real folder is the user's", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "aa"));
    const real = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", real);
    const link = join(root, "L");
    symlinkSync(real, link);
    expect(repositoryIdentityLocation(main)).toEqual(gitsAnswer(main));
    const placed = placements(link);

    lstatKeeps(link, anotherAccounts(realLstat(link)));
    expect(repositoryIdentityLocation(link)).toEqual(gitsAnswer(link));
    expect(placements(link) - placed).toBe(0);
  });
});
