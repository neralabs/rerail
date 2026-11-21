import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { copyFile, mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { eq } from "drizzle-orm";
import { afterEach, describe, expect, it, vi } from "vitest";
import { pruneCheckouts, recordCheckout, recordStartedWorktree } from "../../../src/engine/catalog/repository-registry.js";
import { repositoryIdentityLocation, worktreeRegistry } from "../../../src/engine/catalog/repository-identity.js";
import { runGitBytes } from "../../../src/engine/git.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { checkouts } from "../../../src/engine/storage/schema.js";
import { setRepositoryStarted } from "../../../src/runtime/index.js";
import { removeFolder, removeFolderSync } from "../../support/platform/remove-folder.js";

vi.mock("../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../src/engine/git.js")>();
  return { ...actual, runGitBytes: vi.fn(actual.runGitBytes) };
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
  cleanups.push(() => removeFolder(root));
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

/** Whether this filesystem matches names case-insensitively, as macOS does by default. */
function caseInsensitiveFilesystem(): boolean {
  const probe = mkdtempSync(join(tmpdir(), "rerail-case-"));
  try {
    return existsSync(probe.toUpperCase()) && existsSync(probe.toLowerCase());
  } finally {
    removeFolderSync(probe);
  }
}

const worktreeLists = () => vi.mocked(runGitBytes).mock.calls.filter(([, args]) => args[0] === "worktree").length;

describe("recordStartedWorktree", () => {
  it.skipIf(process.platform === "win32").each([false, true])("registers and preserves a worktree with a newline (legacy Git: %s)", async legacy => {
    const { root, main, database } = await startedRepository();
    const path = join(root, "feature\nsecond line");
    git(main, "worktree", "add", "--quiet", "--detach", path);
    const actual = (await vi.importActual<typeof import("../../../src/engine/git.js")>("../../../src/engine/git.js")).runGitBytes;
    await vi.mocked(runGitBytes).withImplementation((cwd, args, options) => legacy && args.includes("-z")
      ? { status: 129, stdout: Buffer.alloc(0), stderr: "unknown switch" } : actual(cwd, args, options), async () => {
      expect(worktreeRegistry(main)?.worktrees).toEqual([main, path]);
      expect(recordStartedWorktree(database, path, repositoryIdentityLocation(path))).toBe(true);
      database.$client.prepare("INSERT INTO rule_settings(owner_kind,owner_id,folder,target_kind,target_id,\"on\",updated_at) VALUES ('checkout',?,'','rule','r',0,1)").run(path);
      expect(await pruneCheckouts(database)).toEqual([]);
      expect(database.$client.prepare("SELECT owner_id FROM rule_settings WHERE owner_kind='checkout'").pluck().all()).toEqual([path]);
    });
  });

  it("reads the replacement checkout's common directory when a linked worktree path changes hands", async () => {
    const a = await startedRepository(), b = await startedRepository();
    const path = join(a.root, "linked"), moved = join(a.root, "moved");
    git(a.main, "worktree", "add", "--quiet", "--detach", path);
    expect(worktreeRegistry(path)).toMatchObject({ commonDirectory: join(a.main, ".git"), worktrees: [a.main, path] });
    git(a.main, "worktree", "move", path, moved);
    expect(worktreeRegistry(path)).toBeUndefined();
    git(b.main, "worktree", "add", "--quiet", "--detach", path);
    expect(worktreeRegistry(path)).toMatchObject({ commonDirectory: join(b.main, ".git"), worktrees: [b.main, path] });
    expect(worktreeRegistry(moved)).toMatchObject({ commonDirectory: join(a.main, ".git"), worktrees: [a.main, moved] });
  });
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

  it("sees a worktree `git worktree move` moved, which leaves the worktrees folder as it was, and not its old path", async () => {
    const { root, main, database, forged } = await startedRepository();
    const before = join(root, "before");
    git(main, "worktree", "add", "--quiet", "-b", "linked", before);
    expect(recordStartedWorktree(database, forged, repositoryIdentityLocation(forged))).toBe(false);

    const after = join(root, "after");
    git(main, "worktree", "move", before, after);
    expect(recordStartedWorktree(database, after, repositoryIdentityLocation(after))).toBe(true);

    await mkdir(before);
    await writeFile(join(before, ".git"), `gitdir: ${join(main, ".git")}\n`);
    expect(recordStartedWorktree(database, before, repositoryIdentityLocation(before))).toBe(false);
  });

  it("refuses a folder whose own Git folder claims the clone's id and names the folder as its worktree", async () => {
    const { root, main, database } = await startedRepository();
    // A Git folder of the attacker's, with the recorded clone's id copied in, that really does register `planted`.
    const lookalike = join(root, "lookalike");
    await mkdir(lookalike);
    git(lookalike, "init", "--quiet");
    git(lookalike, "commit", "--quiet", "--allow-empty", "-m", "seed");
    const planted = join(root, "planted");
    git(lookalike, "worktree", "add", "--quiet", "-b", "planted", planted);
    await copyFile(join(main, ".git", "rerail-repository-id"), join(lookalike, ".git", "rerail-repository-id"));
    git(main, "worktree", "add", "--quiet", "-b", "real", join(root, "real"));
    const location = repositoryIdentityLocation(planted);
    expect(dirname(location.identityPath)).toBe(join(lookalike, ".git"));

    expect(recordStartedWorktree(database, planted, location)).toBe(false);
    // The refusal leaves what the recorded checkout's registry says untouched.
    expect(recordStartedWorktree(database, join(root, "real"), repositoryIdentityLocation(join(root, "real")))).toBe(true);
  });

  it("records a worktree when a copy of its checkout made with its .git was recorded first", async () => {
    const { root, main, database } = await startedRepository();
    const copy = join(root, "copy");
    execFileSync("cp", ["-a", main, copy]);
    // The copy keeps the clone id; recorded again after it, the main checkout's row comes second.
    expect(recordCheckout(database, copy).cloneId).toBe(recordCheckout(database, main).cloneId);
    database.delete(checkouts).where(eq(checkouts.checkoutPath, main)).run();
    const { cloneId } = recordCheckout(database, main);
    // The select recordStartedWorktree makes has no order: this test covers the refusal only while the copy comes first.
    expect(database.select({ path: checkouts.checkoutPath }).from(checkouts).where(eq(checkouts.cloneId, cloneId)).all().map(row => row.path))
      .toEqual([copy, main]);
    const worktree = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "linked", worktree);

    expect(recordStartedWorktree(database, worktree, repositoryIdentityLocation(worktree))).toBe(true);
  });

  it("refuses a registered worktree of the recorded checkout when the location names another common directory", async () => {
    const { root, main, database } = await startedRepository();
    const worktree = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "linked", worktree);
    const genuine = repositoryIdentityLocation(worktree);
    // The same clone id file, reached through a common directory the recorded checkout does not use.
    const elsewhere = join(root, "elsewhere-common");
    await mkdir(elsewhere);
    await copyFile(genuine.identityPath, join(elsewhere, "rerail-repository-id"));

    expect(recordStartedWorktree(database, worktree, { ...genuine, identityPath: join(elsewhere, "rerail-repository-id") })).toBe(false);
    expect(recordStartedWorktree(database, worktree, genuine)).toBe(true);
  });

  it.skipIf(!caseInsensitiveFilesystem())("records a worktree Git registered under a path typed in another case than the folder has", async () => {
    const { root, main, database } = await startedRepository();
    await mkdir(join(root, "elsewhere"));
    // Git keeps the path as typed; the folder's top level is reported in the case it has on disk.
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "Elsewhere", "w"));
    const worktree = join(root, "elsewhere", "w");
    const location = repositoryIdentityLocation(worktree);
    expect(location.workspaceRoot).toBe(worktree);

    expect(recordStartedWorktree(database, worktree, location)).toBe(true);
  });
});
