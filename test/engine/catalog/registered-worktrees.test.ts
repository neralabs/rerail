import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { recordCheckout, recordStartedWorktree } from "../../../src/engine/catalog/repository-registry.js";
import { repositoryIdentityLocation } from "../../../src/engine/catalog/repository-identity.js";
import { runGit } from "../../../src/engine/git.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { setRepositoryStarted } from "../../../src/runtime/index.js";

vi.mock("../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit) };
});

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** A started repository's main checkout, recorded, with one `.git` file naming its Git folder. */
async function startedRepository() {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-registered-worktrees-")));
  cleanups.push(() => rm(root, { recursive: true, force: true }));
  const main = join(root, "main");
  await mkdir(main);
  git(main, "init", "--quiet");
  git(main, "commit", "--quiet", "--allow-empty", "-m", "seed");
  const database = openRerailDatabase(join(root, "rerail.sqlite"));
  cleanups.push(async () => { database.$client.close(); });
  setRepositoryStarted(database, recordCheckout(database, main).repositoryKey, true);
  const forged = join(main, "forged");
  await mkdir(forged);
  await writeFile(join(forged, ".git"), `gitdir: ${join(main, ".git")}\n`);
  return { root, main, database, forged };
}

const worktreeLists = () => vi.mocked(runGit).mock.calls.filter(([, args]) => args[0] === "worktree").length;

describe("recordStartedWorktree", () => {
  it("asks Git once for a .git file it does not list, however often that folder is looked at", async () => {
    const { database, forged } = await startedRepository();
    const location = repositoryIdentityLocation(forged);
    const before = worktreeLists();

    expect(recordStartedWorktree(database, forged, location)).toBe(false);
    expect(recordStartedWorktree(database, forged, location)).toBe(false);
    expect(recordStartedWorktree(database, forged, location)).toBe(false);

    expect(worktreeLists() - before).toBe(1);
  });

  it("sees a worktree added after Git was last asked, and one removed", async () => {
    const { root, main, database, forged } = await startedRepository();
    expect(recordStartedWorktree(database, forged, repositoryIdentityLocation(forged))).toBe(false);

    const worktree = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "linked", worktree);
    const location = repositoryIdentityLocation(worktree);
    expect(recordStartedWorktree(database, worktree, location)).toBe(true);

    git(main, "worktree", "add", "--quiet", "-b", "other", join(root, "other"));
    git(main, "worktree", "remove", join(root, "other"));
    await mkdir(join(root, "other"));
    await writeFile(join(root, "other", ".git"), `gitdir: ${join(main, ".git")}\n`);
    expect(recordStartedWorktree(database, join(root, "other"), repositoryIdentityLocation(join(root, "other")))).toBe(false);
  });
});
