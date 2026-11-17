/**
 * A pruning pass whose `git worktree list` fails. Git's answer is the only thing that tells a removed worktree from a
 * copy of the clone or a folder renamed away, so a listing Git did not give whole keeps every checkout of the clone:
 * read as an empty listing, it would forget every one of them that is not on disk.
 */
import { execFileSync } from "node:child_process";
import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as Git from "../../../../src/engine/git.js";
import { GitUnavailableError } from "../../../../src/engine/git.js";
import { listRepositories, pruneCheckouts, recordCheckout } from "../../../../src/engine/catalog/repository-registry.js";
import { openRerailDatabase } from "../../../../src/engine/storage/database.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/** What the listing gives instead of Git's answer: a result to answer with, or an error to throw; undefined for Git's. */
const listing = vi.hoisted(() => ({ failure: undefined as (() => Git.GitResult) | undefined }));

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof Git>();
  return {
    ...actual,
    runGitAsync: async (cwd: string, args: readonly string[], options?: Git.RunGitOptions & { input?: string }) => {
      if (listing.failure !== undefined && args[0] === "worktree" && args[1] === "list") return listing.failure();
      return await actual.runGitAsync(cwd, args, options);
    },
  };
});

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  listing.failure = undefined;
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** A clone at `main` with two worktrees Git removed, all three recorded. */
function cloneWithRemovedWorktrees() {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-prune-git-failure-")));
  cleanups.push(() => removeFolder(root));
  const main = join(root, "main");
  git(root, "init", "-q", "-b", "main", main);
  git(main, "commit", "-q", "--allow-empty", "-m", "base");
  const database = openRerailDatabase(join(root, "rerail.sqlite"));
  cleanups.push(() => { database.$client.close(); });
  const removed = ["one", "two"].map(name => join(root, name));
  for (const [index, worktree] of removed.entries()) git(main, "worktree", "add", "-q", "-b", `removed-${index}`, worktree);
  for (const path of [main, ...removed]) recordCheckout(database, path);
  for (const worktree of removed) git(main, "worktree", "remove", worktree);
  const recorded = (): string[] => listRepositories(database).flatMap(repository => repository.checkouts.map(row => row.checkoutPath)).sort();
  return { main, removed, database, recorded };
}

describe("a pruning pass whose Git listing fails", () => {
  it.each([
    { when: "Git exits with an error", failure: (): Git.GitResult => ({ status: 128, stdout: "", stderr: "fatal: not a git repository\n" }) },
    {
      when: "Git runs past its timeout",
      failure: (): Git.GitResult => ({ status: null, stdout: "", stderr: "", error: Object.assign(new Error("Command failed"), { killed: true, signal: "SIGTERM" }) }),
    },
    {
      // Git was stopped mid-listing: what it printed by then names the clone's own checkout and nothing else.
      when: "Git prints more than the read takes",
      failure: (): Git.GitResult => ({
        status: null,
        stdout: "worktree /somewhere/else\nHEAD 0000000000000000000000000000000000000000\nbranch refs/heads/main\n\n",
        stderr: "",
        error: Object.assign(new RangeError("stdout maxBuffer length exceeded"), { code: "ERR_CHILD_PROCESS_STDIO_MAXBUFFER" }),
      }),
    },
    { when: "Git is not installed", failure: (): Git.GitResult => { throw new GitUnavailableError(["/usr/bin/git"]); } },
  ])("keeps every checkout of the clone when $when", async ({ failure }) => {
    const { main, removed, database, recorded } = cloneWithRemovedWorktrees();
    const before = recorded();
    listing.failure = failure;

    expect(await pruneCheckouts(database)).toEqual([]);
    expect(recorded()).toEqual(before);

    // Git's own answer forgets the removed worktrees, so the failure was all that kept them.
    listing.failure = undefined;
    expect((await pruneCheckouts(database)).sort()).toEqual([...removed].sort());
    expect(recorded()).toEqual([main]);
  });
});
