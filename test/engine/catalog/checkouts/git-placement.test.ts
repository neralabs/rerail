import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, readFile, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import {
  repositoryIdentityLocation,
  resolveCloneId,
  worktreeRegistry,
  type RepositoryIdentityLocation,
} from "../../../../src/engine/catalog/repository-identity.js";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { gitText, runGit } from "../../../../src/engine/git.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";
import { linkFolder } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit), gitText: vi.fn(actual.gitText) };
});

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function temporaryRoot(): Promise<string> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-git-placement-")));
  cleanups.push(() => removeFolder(root));
  return root;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], { cwd, stdio: "ignore" });
}

/** A repository at `path` with one commit; returns its real path. */
function repository(path: string): string {
  mkdirSync(join(path, "src"), { recursive: true });
  git(path, "init", "--quiet");
  git(path, "commit", "--quiet", "--allow-empty", "-m", "seed");
  return realpathSync.native(path);
}

/** Git's own answer for `folder`, in this process's environment, as Rerail reads it from `git rev-parse`. */
function gitsAnswer(folder: string): RepositoryIdentityLocation {
  const requested = resolve(folder);
  const result = spawnSync("git", ["-C", requested, "rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"], { encoding: "utf8" });
  if (result.status !== 0) return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id"), main: true };
  const [top, common, gitDirectory] = result.stdout.trim().split("\n").map(line => resolve(line.trim()));
  return { git: true, workspaceRoot: top!, identityPath: join(common!, "rerail-repository-id"), main: common === gitDirectory };
}

/** How many Git processes Rerail started to place `folder`. */
function locateCalls(folder: string): number {
  return vi.mocked(runGit).mock.calls.filter(([cwd, args]) => cwd === resolve(folder) && args.includes("--show-toplevel")).length;
}

/** Rerail's answer for each folder, in order, against Git's own; returns how many Git processes Rerail started for them. */
function expectGitsAnswers(...folders: string[]): number {
  const asked = [...new Set(folders.map(folder => resolve(folder)))];
  const before = asked.reduce((sum, folder) => sum + locateCalls(folder), 0);
  for (const folder of folders) expect(repositoryIdentityLocation(folder), folder).toEqual(gitsAnswer(folder));
  return asked.reduce((sum, folder) => sum + locateCalls(folder), 0) - before;
}

describe("a linked worktree is placed from the files Git reads, as Git places it", () => {
  it("once Git placed a checkout of its repository, wherever the worktree sits", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "beside", join(root, "beside"));
    git(main, "worktree", "add", "--quiet", "-b", "nested", join(main, ".claude", "worktrees", "nested"));
    git(main, "worktree", "add", "--quiet", "--detach", join(root, "deep", "detached"));

    expect(expectGitsAnswers(main)).toBe(1);
    expect(expectGitsAnswers(join(root, "beside"), join(main, ".claude", "worktrees", "nested"), join(root, "deep", "detached"))).toBe(0);
  });

  it("asks Git for the first checkout of a repository it has not placed, and places the others from their files", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    for (const name of ["a", "b", "c"]) git(main, "worktree", "add", "--quiet", "-b", name, join(root, name));

    expect(expectGitsAnswers(join(root, "a"))).toBe(1);
    expect(expectGitsAnswers(join(root, "b"), join(root, "c"), main)).toBe(1);
  });

  it("places a bare repository's worktrees, and keeps each answer while its files are unchanged", async () => {
    const root = await temporaryRoot();
    const seed = repository(join(root, "seed"));
    git(root, "clone", "--quiet", "--bare", seed, join(root, "bare.git"));
    git(join(root, "bare.git"), "worktree", "add", "--quiet", join(root, "one"));
    git(join(root, "bare.git"), "worktree", "add", "--quiet", join(root, "two"));

    expect(expectGitsAnswers(join(root, "bare.git"), join(root, "one"))).toBe(2);
    expect(expectGitsAnswers(join(root, "two"), join(root, "one"), join(root, "two"))).toBe(0);
  });
});

describe("every other folder, and every worktree whose files Git might read otherwise, is asked of Git", () => {
  it("a clone's own checkout, a folder in it, a work tree whose Git folder is kept apart, and a submodule", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(root, "init", "--quiet", `--separate-git-dir=${join(root, "kept-apart.git")}`, join(root, "work"));
    const source = repository(join(root, "source"));
    git(main, "submodule", "add", "--quiet", source, "sub");

    expect(expectGitsAnswers(main, join(main, "src"), join(root, "work"), join(main, "sub"))).toBe(4);
  });

  it("a bare repository, a folder inside a Git folder, an empty .git folder, and a .git file whose Git folder is gone", async () => {
    const root = await temporaryRoot();
    git(root, "init", "--quiet", "--bare", join(root, "bare.git"));
    git(root, "init", "--quiet", `--separate-git-dir=${join(root, "kept-apart.git")}`, join(root, "work"));
    mkdirSync(join(root, "stray", ".git"), { recursive: true });
    mkdirSync(join(root, "orphan"));
    await writeFile(join(root, "orphan", ".git"), `gitdir: ${join(root, "gone", ".git", "worktrees", "orphan")}\n`);

    const folders = [join(root, "bare.git"), join(root, "bare.git", "refs"), join(root, "kept-apart.git", "refs"), join(root, "stray"), join(root, "orphan")];
    expect(expectGitsAnswers(...folders)).toBe(folders.length);
  });

  it("a link into a checkout, a link inside a checkout to a folder outside one, a plain folder and a folder that is gone", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    mkdirSync(join(root, "outside"));
    linkFolder(join(main, "src"), join(root, "into-checkout"));
    linkFolder(join(root, "outside"), join(main, "out-of-checkout"));

    const folders = [join(root, "into-checkout"), join(main, "out-of-checkout"), join(root, "outside"), join(root, "gone")];
    expect(expectGitsAnswers(...folders)).toBe(folders.length);
  });

  it("a worktree moved by hand, one whose HEAD Git has not written yet, and one whose repository sets an extension", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "moved", join(root, "moved"));
    git(main, "worktree", "add", "--quiet", "-b", "unfinished", join(root, "unfinished"));
    expectGitsAnswers(main);
    await rename(join(root, "moved"), join(root, "moved-by-hand"));
    const head = join(main, ".git", "worktrees", "unfinished", "HEAD");
    const contents = await readFile(head, "utf8");
    await rm(head);

    expect(expectGitsAnswers(join(root, "moved-by-hand"), join(root, "unfinished"))).toBe(2);
    await writeFile(head, contents);
    expect(expectGitsAnswers(join(root, "unfinished"))).toBe(0);

    const other = repository(join(root, "other"));
    git(other, "config", "extensions.worktreeConfig", "true");
    git(other, "worktree", "add", "--quiet", "-b", "configured", join(root, "configured"));
    expect(expectGitsAnswers(other, join(root, "configured"))).toBe(2);
  });

  it("a worktree whose repository moved, before and after `git worktree repair`, and one `git worktree move` moved", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    git(main, "worktree", "add", "--quiet", "-b", "mover", join(root, "mover"));
    expect(expectGitsAnswers(main, join(root, "linked"))).toBe(1);

    git(main, "worktree", "move", join(root, "mover"), join(root, "moved"));
    expect(expectGitsAnswers(join(root, "moved"), join(root, "mover"))).toBe(1);

    await rename(main, join(root, "relocated"));
    expect(expectGitsAnswers(join(root, "linked"))).toBe(1);
    git(join(root, "relocated"), "worktree", "repair", join(root, "linked"));
    expect(expectGitsAnswers(join(root, "relocated"))).toBe(1);
    expect(expectGitsAnswers(join(root, "linked"))).toBe(0);
  });

  it("asks again once Git's config of the repository moves where it is placed", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    expect(expectGitsAnswers(main, main)).toBe(1);

    git(main, "config", "core.bare", "true");
    expect(expectGitsAnswers(main)).toBe(1);
    git(main, "config", "core.bare", "false");
    expect(expectGitsAnswers(main)).toBe(1);
    git(main, "config", "branch.main.description", "a write that does not move it");
    expect(expectGitsAnswers(main)).toBe(0);
  });

  it("asks Git each time while GIT_DIR names a repository, or Git is told to take every folder for another user's", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    expectGitsAnswers(main, join(root, "linked"));

    vi.stubEnv("GIT_DIR", join(main, ".git"));
    expect(expectGitsAnswers(join(root, "linked"), join(root, "linked"))).toBe(2);
    vi.unstubAllEnvs();
    vi.stubEnv("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1");
    expect(expectGitsAnswers(join(root, "linked"), main)).toBe(2);
  });
});

describe("a repository start", () => {
  it("places every registered worktree with no Git process per worktree", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const main = repository(join(root, "main"));
    git(main, "remote", "add", "origin", "https://example.invalid/acme/main.git");
    setUpTestRepository(main, {}, databasePath);
    const worktrees = ["a", "b", "c", "d", "e", "f"].map(name => {
      git(main, "worktree", "add", "--quiet", "-b", name, join(root, "wt", name));
      return realpathSync.native(join(root, "wt", name));
    });
    const catalogs = new CheckoutCatalogs(databasePath);
    const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
    const auditor = new CheckoutCodeRuleRouter({
      sessionRoot: main,
      startingCheckout: startingCheckout(owners, main).checkoutRoot,
      owners,
      catalogs,
      steerDelivery: { requestSteer: async () => undefined },
      audits: { start: () => "0", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
    });
    cleanups.push(async () => {
      await auditor.close();
      catalogs.close();
    });
    await auditor.openWorkspace("start");
    const gitBefore = vi.mocked(runGit).mock.calls.length;
    const textBefore = vi.mocked(gitText).mock.calls.length;

    // Each worktree gets recorded on the way, and each record makes every kept answer of the owners go.
    expect(auditor.worktreeRoots().toSorted()).toEqual(worktrees.toSorted());
    expect(auditor.worktreeRoots().toSorted()).toEqual(worktrees.toSorted());

    const started = vi.mocked(runGit).mock.calls.slice(gitBefore).map(([, args]) => args[0]);
    expect({ started, configReads: vi.mocked(gitText).mock.calls.length - textBefore }).toEqual({ started: ["worktree"], configReads: 0 });
  });

  it("lists a repository's registry once for all of its checkouts, and reads each again once a worktree is added", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    const lists = () => vi.mocked(runGit).mock.calls.filter(([, args]) => args[0] === "worktree").length;
    const before = lists();

    expect(worktreeRegistry(main)?.worktrees).toEqual(worktreeRegistry(join(root, "linked"))?.worktrees);
    expect(lists() - before).toBe(1);
    git(main, "worktree", "add", "--quiet", "-b", "later", join(root, "later"));
    expect(worktreeRegistry(join(root, "linked"))?.worktrees).toHaveLength(3);
    expect(worktreeRegistry(main)?.worktrees).toHaveLength(3);
    expect(lists() - before).toBe(2);
  });

  it("reads a clone id from the file a repository's checkouts share, and adopts a legacy config value only before one exists", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    git(main, "config", "rerail.repository-id", "legacy-id");
    const configReads = () => vi.mocked(gitText).mock.calls.filter(([, args]) => args[0] === "config").length;
    const before = configReads();

    expect(resolveCloneId(main).cloneId).toBe("legacy-id");
    expect(resolveCloneId(join(root, "linked")).cloneId).toBe("legacy-id");
    expect(resolveCloneId(main).cloneId).toBe("legacy-id");
    expect(configReads() - before).toBe(1);
  });

  it("asks Git for no submodules of a checkout with no .gitmodules, and for those of one that has it", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    const source = repository(join(root, "source"));
    const owners = new CheckoutOwners();
    const before = vi.mocked(runGit).mock.calls.filter(([, args]) => args.includes(".gitmodules")).length;

    expect(owners.declaredSubmodules(main)).toEqual([]);
    expect(vi.mocked(runGit).mock.calls.filter(([, args]) => args.includes(".gitmodules")).length).toBe(before);
    git(main, "submodule", "add", "--quiet", source, "sub");
    expect(owners.declaredSubmodules(main)).toEqual([realpathSync.native(join(main, "sub"))]);
  });
});
