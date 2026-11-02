import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  baseBranch,
  changedPaths,
  commitExists,
  currentBranch,
  githubRepositoryOf,
  isAncestor,
  localBranches,
  mergeBase,
  mergeTreeConflicts,
} from "../../../src/engine/merge-assist/branch-git.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" }).trim();
}

function write(root: string, files: Record<string, string>): void {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
}

function commit(root: string, files: Record<string, string>, message: string): string {
  write(root, files);
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", message);
  return git(root, "rev-parse", "HEAD");
}

async function repository(): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-branch-git-")));
  roots.push(root);
  git(root, "init", "--quiet", "-b", "main");
  commit(root, { "src/api/client.ts": "export const a = 1;\n", "README.md": "seed\n" }, "seed");
  return root;
}

describe("branches", () => {
  it("names the current branch, null when detached, and lists local branches with their checkouts", async () => {
    const root = await repository();
    const seed = git(root, "rev-parse", "HEAD");
    git(root, "branch", "feature/retry");
    const worktree = join(root, ".worktrees", "other");
    git(root, "worktree", "add", "--quiet", "-b", "other", worktree);

    expect(currentBranch(root)).toBe("main");
    expect(currentBranch(worktree)).toBe("other");
    expect(localBranches(root)).toEqual([
      { name: "feature/retry", commit: seed, worktree: null, upstream: null },
      { name: "main", commit: seed, worktree: root, upstream: null },
      { name: "other", commit: seed, worktree: realpathSync(worktree), upstream: null },
    ]);

    git(root, "checkout", "--quiet", "--detach");
    expect(currentBranch(root)).toBeNull();
  });

  it("names the remote each local branch tracks: a remote, `.` for a local branch, or a URL", async () => {
    const root = await repository();
    git(root, "remote", "add", "origin", root);
    git(root, "fetch", "--quiet", "origin");
    git(root, "branch", "--quiet", "--track", "tracked", "origin/main");
    git(root, "branch", "--quiet", "--track", "stacked", "main");
    git(root, "branch", "fork");
    git(root, "config", "branch.fork.remote", "https://github.com/someone/fork.git");
    git(root, "config", "branch.fork.merge", "refs/heads/fix");

    expect(Object.fromEntries(localBranches(root).map(branch => [branch.name, branch.upstream]))).toEqual({
      fork: "https://github.com/someone/fork.git",
      main: null,
      stacked: ".",
      tracked: "origin",
    });
  });

  it("finds the base branch: origin's HEAD, then origin/main, origin/master, main, master", async () => {
    const root = await repository();
    const seed = git(root, "rev-parse", "HEAD");
    expect(baseBranch(root)).toEqual({ ref: "refs/heads/main", name: "main" });

    git(root, "branch", "-m", "main", "master");
    expect(baseBranch(root)).toEqual({ ref: "refs/heads/master", name: "master" });
    git(root, "branch", "main");
    expect(baseBranch(root)).toEqual({ ref: "refs/heads/main", name: "main" });

    git(root, "update-ref", "refs/remotes/origin/master", seed);
    expect(baseBranch(root)).toEqual({ ref: "refs/remotes/origin/master", name: "origin/master" });
    git(root, "update-ref", "refs/remotes/origin/main", seed);
    expect(baseBranch(root)).toEqual({ ref: "refs/remotes/origin/main", name: "origin/main" });

    git(root, "update-ref", "refs/remotes/origin/trunk", seed);
    git(root, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk");
    expect(baseBranch(root)).toEqual({ ref: "refs/remotes/origin/trunk", name: "origin/trunk" });

    // An origin HEAD that points nowhere is skipped.
    git(root, "update-ref", "-d", "refs/remotes/origin/trunk");
    expect(baseBranch(root)).toEqual({ ref: "refs/remotes/origin/main", name: "origin/main" });
  });

  it("returns null without a base branch", async () => {
    const root = await repository();
    git(root, "branch", "-m", "main", "trunk");
    expect(baseBranch(root)).toBeNull();
  });
});

describe("ancestry", () => {
  it("answers merge base, ancestry and existence, and refuses what looks like an option", async () => {
    const root = await repository();
    const seed = git(root, "rev-parse", "HEAD");
    git(root, "checkout", "--quiet", "-b", "feature");
    const feature = commit(root, { "src/feature.ts": "export {};\n" }, "feature");

    expect(mergeBase(root, "main", "feature")).toBe(seed);
    expect(mergeBase(root, "main", "missing")).toBeNull();
    expect(isAncestor(root, "main", "feature")).toBe(true);
    expect(isAncestor(root, "feature", "main")).toBe(false);
    expect(isAncestor(root, "main", "missing")).toBe(false);
    expect(commitExists(root, feature)).toBe(true);
    expect(commitExists(root, "0".repeat(40))).toBe(false);
    expect(commitExists(root, "--all")).toBe(false);
    expect(mergeBase(root, "--all", "main")).toBeNull();
  });
});

describe("changedPaths", () => {
  it("lists the branch's paths since the merge base, and with the working tree its staged, unstaged and untracked ones", async () => {
    const root = await repository();
    git(root, "checkout", "--quiet", "-b", "feature");
    commit(root, { "src/api/client.ts": "export const a = 2;\n", "src/new.ts": "export {};\n" }, "feature");
    git(root, "mv", "README.md", "docs.md");
    git(root, "commit", "--quiet", "-m", "rename");
    // Base moves on; its own change is not the branch's.
    git(root, "checkout", "--quiet", "main");
    commit(root, { "base-only.ts": "export {};\n" }, "base");
    git(root, "checkout", "--quiet", "feature");

    expect(changedPaths(root, "main", "feature")).toEqual(["README.md", "docs.md", "src/api/client.ts", "src/new.ts"]);

    write(root, { "staged.ts": "1\n", "src/new.ts": "export const changed = 1;\n", "untracked dir/file.ts": "1\n", "ignored.log": "x\n", ".gitignore": "*.log\n" });
    git(root, "add", "staged.ts");
    expect(changedPaths(root, "main", "HEAD")).toEqual(["README.md", "docs.md", "src/api/client.ts", "src/new.ts"]);
    expect(changedPaths(root, "main", "HEAD", { workingTree: true })).toEqual([
      ".gitignore", "README.md", "docs.md", "src/api/client.ts", "src/new.ts", "staged.ts", "untracked dir/file.ts",
    ]);
  });

  it("throws rather than answer for a commit it cannot find", async () => {
    const root = await repository();
    expect(() => changedPaths(root, "main", "missing")).toThrow(/not a commit here/u);
  });
});

describe("mergeTreeConflicts", () => {
  it("lists the files two branches conflict on, nothing for a clean merge, and null for an unknown commit", async () => {
    const root = await repository();
    git(root, "checkout", "--quiet", "-b", "theirs");
    commit(root, { "src/api/client.ts": "export const a = 'theirs';\n", "src/theirs.ts": "1\n" }, "theirs");
    git(root, "checkout", "--quiet", "main");
    git(root, "checkout", "--quiet", "-b", "ours");
    commit(root, { "src/api/client.ts": "export const a = 'ours';\n", "src/ours.ts": "1\n" }, "ours");
    git(root, "checkout", "--quiet", "main");
    git(root, "checkout", "--quiet", "-b", "clean");
    commit(root, { "src/clean.ts": "1\n" }, "clean");
    const before = git(root, "status", "--porcelain");

    expect(mergeTreeConflicts(root, "ours", "theirs")).toEqual(["src/api/client.ts"]);
    expect(mergeTreeConflicts(root, "ours", "clean")).toEqual([]);
    expect(mergeTreeConflicts(root, "ours", "0".repeat(40))).toBeNull();
    // Nothing in the checkout changed.
    expect(git(root, "status", "--porcelain")).toBe(before);
    expect(currentBranch(root)).toBe("clean");
  });
});

describe("githubRepositoryOf", () => {
  it("reads github.com and GitHub Enterprise keys, and nothing else", () => {
    expect(githubRepositoryOf("remote:github.com/neralabs/rerail")).toEqual({ host: "github.com", owner: "neralabs", name: "rerail" });
    expect(githubRepositoryOf("remote:github.example.com/team/app.web")).toEqual({ host: "github.example.com", owner: "team", name: "app.web" });
    expect(githubRepositoryOf("remote:gitlab.com/neralabs/rerail")).toBeNull();
    expect(githubRepositoryOf("remote:git.example.com/team/app")).toBeNull();
    expect(githubRepositoryOf("remote:mygithub.com/team/app")).toBeNull();
    expect(githubRepositoryOf("remote:github/team/app")).toBeNull();
    expect(githubRepositoryOf("remote:github.com/group/sub/app")).toBeNull();
    expect(githubRepositoryOf("remote:github.com/neralabs")).toBeNull();
    expect(githubRepositoryOf("remote:github.com/-bad/app")).toBeNull();
    expect(githubRepositoryOf("remote:github.com/team/..")).toBeNull();
    expect(githubRepositoryOf("clone:0123abcd")).toBeNull();
  });
});
