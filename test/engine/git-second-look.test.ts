/**
 * The second look for Git: once the Git a process kept is gone, the look that finds another decides by what the first
 * look decided by, the folder it looked from, `PATH` and the repository it found there. Each test loads git.ts afresh,
 * so its process has not looked for Git yet. It looks, changes what a look would read anew (the repository's `.git`, the
 * folder, `PATH`), removes the Git the first look kept and makes a Git call, which looks again.
 *
 * Every Git here is a stand-in: an empty file Rerail takes for a Git. The tests read which one the process keeps, and on
 * Windows, which cannot run an empty git.exe, what each call answers then.
 */
import { chmod, mkdir, mkdtemp, rename, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { removeFolder } from "../support/platform/remove-folder.js";

type GitModule = typeof import("../../src/engine/git.js");

const GIT = process.platform === "win32" ? "git.exe" : "git";
const machinePath = process.env.PATH;

describe("the second look for Git", () => {
  const startedIn = process.cwd();
  let root = "";
  let repository = "";
  let outside: string[] = [];

  /** git.ts as a new process loads it: no Git looked for yet. */
  async function freshGit(): Promise<GitModule> {
    vi.resetModules();
    return await import("../../src/engine/git.js");
  }

  /** `folder`, made with a stand-in Git in it. */
  async function withGit(folder: string): Promise<string> {
    await mkdir(folder, { recursive: true });
    await writeFile(join(folder, GIT), "");
    await chmod(join(folder, GIT), 0o755);
    return folder;
  }

  const gitIn = (folder: string): string => join(folder, GIT);

  /** Removes the Git `git` keeps, then makes a Git call, which looks again and waits for that look; the Git kept after it. */
  async function lookAgain(git: GitModule): Promise<string> {
    await rm(git.gitExecutable());
    await git.runGitAsync(root, ["--version"]);
    return git.gitExecutable();
  }

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-git-second-look-"));
    repository = join(root, "repository");
    await mkdir(join(repository, ".git"), { recursive: true });
    await withGit(join(repository, "tools"));
    outside = await Promise.all(["outside-1", "outside-2", "outside-3"].map(async name => await withGit(join(root, name))));
  });

  afterEach(async () => {
    process.chdir(startedIn);
    vi.unstubAllEnvs();
    await removeFolder(root);
  });

  it("skips the repository's PATH entries once that repository's .git is gone, in a call and in a read", async () => {
    // This machine's Git last, for the read: Windows refuses to start an empty stand-in, and a read it refuses throws
    // (EFTYPE), as on main.
    vi.stubEnv("PATH", [join(repository, "tools"), outside[0]!, outside[1]!, ...(machinePath === undefined ? [] : [machinePath])].join(delimiter));
    const git = await freshGit();
    // As the daemon looks: from the repository it serves.
    git.searchGitFrom(repository);
    expect(git.gitExecutable()).toBe(gitIn(outside[0]!));
    await rename(join(repository, ".git"), join(root, "moved-aside"));
    expect(await lookAgain(git)).toBe(gitIn(outside[1]!));
    await rm(git.gitExecutable());
    await git.gitHead(root);
    expect(git.gitExecutable().startsWith(root)).toBe(false);
  });

  it("skips an entry that links into the repository once its .git is gone", async () => {
    const link = join(root, "linked-tools");
    // A junction on Windows, which needs no privilege; the type is ignored elsewhere.
    await symlink(join(repository, "tools"), link, "junction");
    vi.stubEnv("PATH", [link, ...outside].join(delimiter));
    const git = await freshGit();
    git.searchGitFrom(repository);
    expect(git.gitExecutable()).toBe(gitIn(outside[0]!));
    await rename(join(repository, ".git"), join(root, "moved-aside"));
    expect(await lookAgain(git)).toBe(gitIn(outside[1]!));
  });

  it("skips a relative entry and a node_modules entry at a later look, outside the repository too", async () => {
    // Both entries lie outside the repository the first look found, so only their own skips keep them out, and they
    // hold a Git only from the second look on.
    const plain = join(root, "plain");
    const nodeModules = join(root, "elsewhere", "node_modules", ".bin");
    await mkdir(plain);
    vi.stubEnv("PATH", ["tools", nodeModules, ...outside].join(delimiter));
    process.chdir(plain);
    const git = await freshGit();
    git.searchGitFrom(repository);
    expect(git.gitExecutable()).toBe(gitIn(outside[0]!));
    await withGit(join(plain, "tools"));
    await withGit(nodeModules);
    expect(await lookAgain(git)).toBe(gitIn(outside[1]!));
  });

  it("looks from the folder of its first look after the process moves to another repository", async () => {
    const other = join(root, "other");
    await mkdir(join(other, ".git"), { recursive: true });
    vi.stubEnv("PATH", [join(repository, "tools"), ...outside].join(delimiter));
    await mkdir(join(repository, "src"));
    process.chdir(join(repository, "src"));
    const git = await freshGit();
    expect(git.gitExecutable()).toBe(gitIn(outside[0]!));
    // A look from there finds that repository, and the first repository's tools are outside it.
    process.chdir(other);
    expect(await lookAgain(git)).toBe(gitIn(outside[1]!));
  });

  it("looks from the folder of its first look after the process moves to a folder that is then deleted", async ({ skip }) => {
    skip(process.platform === "win32", "Windows keeps a process's working folder open, so it cannot be deleted under the process");
    vi.stubEnv("PATH", [join(repository, "tools"), ...outside].join(delimiter));
    process.chdir(repository);
    const git = await freshGit();
    expect(git.gitExecutable()).toBe(gitIn(outside[0]!));
    // Node keeps the working folder it last read, so a deleted one reads as gone only after a move into it.
    const gone = join(root, "gone");
    await mkdir(gone);
    process.chdir(gone);
    await removeFolder(gone);
    expect(await lookAgain(git)).toBe(gitIn(outside[1]!));
  });

  it("looks from the folder its first look was named after the process names another", async () => {
    const other = join(root, "other");
    await mkdir(join(other, ".git"), { recursive: true });
    vi.stubEnv("PATH", [join(repository, "tools"), ...outside].join(delimiter));
    const git = await freshGit();
    git.searchGitFrom(repository);
    expect(git.gitExecutable()).toBe(gitIn(outside[0]!));
    git.searchGitFrom(other);
    expect(await lookAgain(git)).toBe(gitIn(outside[1]!));
  });

  it("answers a call whose Git found again Windows cannot run as one that failed, and rejects none", async ({ skip }) => {
    skip(process.platform !== "win32", "Node throws, rather than reports, a start Windows refuses as not a program (EFTYPE)");
    vi.stubEnv("PATH", outside.join(delimiter));
    const git = await freshGit();
    git.searchGitFrom(repository);
    expect(git.gitExecutable()).toBe(gitIn(outside[0]!));
    // The look finds outside-2's stand-in, an empty git.exe, as an installer leaves one it is still writing.
    await rm(git.gitExecutable());
    await expect(git.gitHead(root)).resolves.toBeUndefined();
    expect(git.gitExecutable()).toBe(gitIn(outside[1]!));
    await expect(git.gitModifiedOrUntrackedListing(root)).resolves.toEqual({ ended: "failed" });
    await expect(git.readGitBlobIds(root, ["HEAD:a.py"])).resolves.toEqual(new Map());
    await expect(git.runGitAsync(root, ["--version"])).resolves.toMatchObject({ status: null, error: { code: "EFTYPE" } });
    expect(git.runGit(root, ["--version"])).toMatchObject({ status: null, error: { code: "EFTYPE" } });
  });

  it("reads the PATH of its first look, not one the process set since", async () => {
    const elsewhere = await withGit(join(root, "elsewhere"));
    vi.stubEnv("PATH", outside.join(delimiter));
    const git = await freshGit();
    git.searchGitFrom(repository);
    expect(git.gitExecutable()).toBe(gitIn(outside[0]!));
    vi.stubEnv("PATH", [elsewhere, ...outside].join(delimiter));
    expect(await lookAgain(git)).toBe(gitIn(outside[1]!));
  });
});
