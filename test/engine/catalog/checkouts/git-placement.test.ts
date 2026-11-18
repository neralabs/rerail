import { execFileSync, spawnSync } from "node:child_process";
import { closeSync, ftruncateSync, mkdirSync, openSync, realpathSync, writeSync } from "node:fs";
import { mkdtemp, open, readFile, rename, rm, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { physicalPath } from "../../../../src/engine/catalog/checkout-control.js";
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
import { pathsOwnedByCurrentUser } from "../../../../src/engine/native/native.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";
import { linkFolder } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit), gitText: vi.fn(actual.gitText) };
});

vi.mock("../../../../src/engine/native/native.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/native/native.js")>();
  return { ...actual, pathsOwnedByCurrentUser: vi.fn(actual.pathsOwnedByCurrentUser) };
});

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  vi.mocked(pathsOwnedByCurrentUser).mockReset();
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

    const folders = [join(root, "into-checkout"), join(main, "out-of-checkout"), join(root, "gone")];
    expect(expectGitsAnswers(...folders)).toBe(folders.length);
    // A plain folder with no Git folder in it or above it is answered as Git would, with no Git process (outsideGit).
    expect(expectGitsAnswers(join(root, "outside"))).toBe(0);
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

describe("GIT_CEILING_DIRECTORIES, which stops Git's search only above the folder whose own .git Git reads first", () => {
  it("keeps Git's answer for a checkout asked twice and places its worktree from its files, even at the ceiling", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    const folders = [main, join(root, "linked")];
    vi.stubEnv("GIT_CEILING_DIRECTORIES", [root, ...folders].join(delimiter));

    expect(expectGitsAnswers(main, main)).toBe(1);
    expect(expectGitsAnswers(join(root, "linked"), ...folders)).toBe(0);
    expectGitsRegistries(...folders);
    expect(folders.reduce((sum, folder) => sum + locateCalls(folder), 0)).toBe(1);
  });
});

/** Points the link at `path`, a symbolic link or on Windows a junction, at `target` instead. */
async function relink(path: string, target: string): Promise<void> {
  await unlink(path);
  linkFolder(target, path);
}

describe("a kept answer is placed again once Git's answer can move, whatever file times say", () => {
  it("a main checkout reached through a link, once the link's folder is renamed and the link pointed at the new name", async () => {
    const root = await temporaryRoot();
    repository(join(root, "r1"));
    const link = join(root, "L");
    linkFolder(join(root, "r1"), link);
    expectGitsAnswers(link, link);

    await rename(join(root, "r1"), join(root, "r2"));
    await relink(link, join(root, "r2"));
    expect(expectGitsAnswers(link)).toBe(1);
    expect(expectGitsAnswers(link)).toBe(0);
  });

  it("a linked worktree under a linked folder, and the owner of its files once the recorded checkouts change", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    mkdirSync(join(root, "p1"));
    git(main, "worktree", "add", "--quiet", "-b", "w", join(root, "p1", "w"));
    const link = join(root, "L");
    linkFolder(join(root, "p1"), link);
    let recorded = "1";
    const owners = new CheckoutOwners(() => recorded);
    const file = join(link, "w", "src", "x.ts");
    expectGitsAnswers(main, join(link, "w"));
    expect(owners.ownerOf(file)).toEqual({ checkoutRoot: physicalPath(join(root, "p1", "w")), path: "src/x.ts" });

    await rename(join(root, "p1"), join(root, "p2"));
    await relink(link, join(root, "p2"));
    recorded = "2";
    expect(owners.ownerOf(file)).toEqual({ checkoutRoot: physicalPath(join(root, "p2", "w")), path: "src/x.ts" });
    expectGitsRegistries(join(link, "w"));
  });

  it("a linked worktree whose .git file names its Git folder through a link, once the repository is renamed and the link pointed at it", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "r1"));
    const worktree = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", worktree);
    const link = join(root, "L");
    linkFolder(join(root, "r1"), link);
    await rewriteInPlace(join(worktree, ".git"), `gitdir: ${join(link, ".git", "worktrees", "w")}\n`);
    expectGitsRegistries(main, worktree);

    await rename(join(root, "r1"), join(root, "r2"));
    await relink(link, join(root, "r2"));
    expectGitsRegistries(worktree);
  });

  it("core.bare and core.worktree in config.worktree, which Git reads once extensions.worktreeConfig is set", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "config", "extensions.worktreeConfig", "true");
    git(main, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    mkdirSync(join(root, "elsewhere"));
    expectGitsAnswers(main, join(root, "w"), main, join(root, "w"));

    git(main, "config", "--worktree", "core.bare", "true");
    git(join(root, "w"), "config", "--worktree", "core.worktree", join(root, "elsewhere"));
    expectGitsAnswers(main, join(root, "w"));
    git(main, "config", "--worktree", "--unset", "core.bare");
    git(join(root, "w"), "config", "--worktree", "--unset", "core.worktree");
    expectGitsAnswers(main, join(root, "w"), main, join(root, "w"));
  });

  it("core.bare and core.worktree in a file the config includes, which Git does not read to place a checkout", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    mkdirSync(join(root, "elsewhere"));
    await writeFile(join(root, "included"), `[core]\n\tbare = true\n\tworktree = ${join(root, "elsewhere").replaceAll("\\", "/")}\n`);
    expectGitsAnswers(main);

    git(main, "config", "include.path", join(root, "included"));
    expectGitsAnswers(main, main);
  });

  it("a checkout whose Git folder Git no longer takes for one, and again once it does", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    const head = join(main, ".git", "HEAD");
    const contents = await readFile(head, "utf8");
    expectGitsAnswers(main, join(root, "w"));

    await rewriteInPlace(head, "not a head\n");
    expectGitsAnswers(main);
    await rewriteInPlace(head, contents);
    await rename(join(main, ".git", "objects"), join(main, ".git", "objects-away"));
    expectGitsAnswers(main, join(root, "w"));
    await rename(join(main, ".git", "objects-away"), join(main, ".git", "objects"));
    expectGitsAnswers(main, join(root, "w"));
  });

  it("a .git file rewritten while Git answers, and back before the next ask, is placed as Git places it then", async () => {
    const root = await temporaryRoot();
    const mine = repository(join(root, "mine"));
    const your = repository(join(root, "your"));
    git(mine, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    git(your, "worktree", "add", "--quiet", "-b", "o", join(root, "o"));
    const worktree = join(root, "w");
    const dotGit = join(worktree, ".git");
    const own = await readFile(dotGit, "utf8");
    // Git reads the other repository's worktree in the file while it runs; the file names its own again before
    // Rerail reads it next, as a second rewrite within one Git process leaves it.
    const actual = vi.mocked(runGit).getMockImplementation()!;
    vi.mocked(runGit).mockImplementationOnce((cwd, args, options) => {
      rewriteInPlaceNow(dotGit, `gitdir: ${join(your, ".git", "worktrees", "o")}\n`);
      try {
        return actual(cwd, args, options);
      } finally {
        rewriteInPlaceNow(dotGit, own);
      }
    });

    expect(repositoryIdentityLocation(worktree)).not.toEqual(gitsAnswer(worktree));
    expect(repositoryIdentityLocation(worktree)).toEqual(gitsAnswer(worktree));
  });
});

describe.runIf(process.platform === "win32")("on Windows, where Node reads no owner, the engine reads it as Git does", () => {
  it("takes the folders of a repository the user made, which Git takes too, and no folder Windows' installer owns or path that is gone", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    const { pathsOwnedByCurrentUser: engineReads } = await vi.importActual<typeof import("../../../../src/engine/native/native.js")>(
      "../../../../src/engine/native/native.js",
    );
    const system = process.env["SystemRoot"] ?? String.raw`C:\Windows`;

    expect(gitsAnswer(main).git).toBe(true);
    expect({
      own: engineReads([main, join(main, ".git"), root]),
      system: engineReads([system]),
      ownAndSystem: engineReads([main, system]),
      gone: engineReads([join(root, "gone")]),
      none: engineReads([]),
    }).toEqual({ own: true, system: false, ownAndSystem: false, gone: false, none: false });
  });

  it("asks Git for a linked worktree whose folders the engine does not take for the user's, or when no engine can tell", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "another-owner", join(root, "another-owner"));
    git(main, "worktree", "add", "--quiet", "-b", "owner-unread", join(root, "owner-unread"));
    git(main, "worktree", "add", "--quiet", "-b", "own", join(root, "own"));
    expect(expectGitsAnswers(main)).toBe(1);

    vi.mocked(pathsOwnedByCurrentUser).mockReturnValue(false);
    expect(expectGitsAnswers(join(root, "another-owner"))).toBe(1);
    vi.mocked(pathsOwnedByCurrentUser).mockReturnValue(undefined);
    expect(expectGitsAnswers(join(root, "owner-unread"))).toBe(1);
    vi.mocked(pathsOwnedByCurrentUser).mockReset();
    expect(expectGitsAnswers(join(root, "own"))).toBe(0);
    // Git reads the owner of the worktree's folder, its .git file and the Git folder that file names.
    expect(vi.mocked(pathsOwnedByCurrentUser).mock.lastCall?.[0]).toHaveLength(3);
    expect(vi.mocked(pathsOwnedByCurrentUser).mock.lastCall?.[0].slice(0, 2)).toEqual([join(root, "own"), join(root, "own", ".git")]);
  });
});

/**
 * Makes the owner read say another account owns every folder, as `chown` or `takeown` would leave them. Git, a process
 * of its own, still reads the real owners, so its answers stay those of the user's folders.
 */
function takeOwnersAway(): void {
  if (process.platform === "win32") {
    vi.mocked(pathsOwnedByCurrentUser).mockReturnValue(false);
    return;
  }
  const user = process.geteuid?.() ?? 0;
  const spy = vi.spyOn(process, "geteuid").mockReturnValue(user + 1);
  cleanups.push(async () => {
    spy.mockRestore();
  });
}

describe("a kept answer's owners are read again each time the recorded checkouts change", () => {
  it("asks Git again once its folders are another account's, as each start asked it before answers were kept", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    let recorded = "one checkout";
    const owners = new CheckoutOwners(() => recorded);
    const folders = [main, join(root, "linked")];
    expect(expectGitsAnswers(...folders)).toBe(1);

    recorded = "a checkout recorded, its owners as they were";
    owners.lookAgainAt(root);
    expect(expectGitsAnswers(...folders)).toBe(0);

    takeOwnersAway();
    expect(expectGitsAnswers(...folders)).toBe(0);
    recorded = "a checkout recorded once the owners changed";
    owners.lookAgainAt(root);
    expect(expectGitsAnswers(...folders)).toBe(2);
    expect(expectGitsAnswers(...folders)).toBe(0);
  });
});

/** `git worktree list` in `checkout`, read as `worktreeRegistry` reads it. */
function gitsRegistry(checkout: string): string[] {
  const result = spawnSync("git", ["-C", checkout, "worktree", "list", "--porcelain"], { encoding: "utf8" });
  return result.stdout.split("\n").filter(line => line.startsWith("worktree ")).map(line => line.slice("worktree ".length));
}

/** Rerail's registry of each checkout, the common directory it names and the checkout's placement, in order, against Git's. */
function expectGitsRegistries(...checkouts: string[]): void {
  for (const checkout of checkouts) {
    const registry = worktreeRegistry(checkout);
    expect(registry?.worktrees, `the registry at ${checkout}`).toEqual(gitsRegistry(checkout));
    expect(registry?.commonDirectory, `the common directory at ${checkout}`).toBe(physicalPath(dirname(gitsAnswer(checkout).identityPath)));
    expect(repositoryIdentityLocation(checkout), `the placement of ${checkout}`).toEqual(gitsAnswer(checkout));
  }
}

/**
 * Writes `text` over the file at `path` in place, as Git rewrites a `.git` file. Git for Windows hides a worktree's
 * `.git` file, and Windows refuses to make a file anew over a hidden one.
 */
/** {@link rewriteInPlace} without waiting, for a rewrite made while a Git process Rerail started runs. */
function rewriteInPlaceNow(path: string, text: string): void {
  const descriptor = openSync(path, "r+");
  try {
    ftruncateSync(descriptor, 0);
    writeSync(descriptor, text, 0);
  } finally {
    closeSync(descriptor);
  }
}

async function rewriteInPlace(path: string, text: string): Promise<void> {
  const file = await open(path, "r+");
  try {
    await file.truncate(0);
    await file.write(text, 0);
  } finally {
    await file.close();
  }
}

/** A repository `first` with the worktrees w1 and w2 beside it, each asked once as a start asks after them. */
async function firstWithWorktrees(): Promise<{ root: string; main: string; w1: string; w2: string }> {
  const root = await temporaryRoot();
  const main = repository(join(root, "first"));
  git(main, "worktree", "add", "--quiet", "-b", "w1", join(root, "w1"));
  git(main, "worktree", "add", "--quiet", "-b", "w2", join(root, "w2"));
  const [w1, w2] = [join(root, "w1"), join(root, "w2")];
  expectGitsRegistries(w1, main, w2);
  return { root, main, w1, w2 };
}

describe("a checkout's folder that comes to hold another repository's checkout while Rerail runs answers as Git does", () => {
  it.each([["at the reused folder", true], ["at the first repository", false]])(
    "a worktree removed with Git and its folder made another repository's worktree, asked %s first",
    async (_, reusedFirst) => {
      const { root, main, w1, w2 } = await firstWithWorktrees();
      git(main, "worktree", "remove", "--force", w1);
      const other = repository(join(root, "other"));
      git(other, "worktree", "add", "--quiet", "-b", "o1", w1);

      expectGitsRegistries(...(reusedFirst ? [w1, main, w2, other] : [main, w2, other, w1]));
      expect(resolveCloneId(w1).cloneId).toBe(resolveCloneId(other).cloneId);
      expect(resolveCloneId(w2).cloneId).toBe(resolveCloneId(main).cloneId);
      expect(resolveCloneId(w1).cloneId).not.toBe(resolveCloneId(main).cloneId);
    },
  );

  it("a worktree removed by hand, its entry left for `git worktree prune`, and its folder made another repository's worktree", async () => {
    const { root, main, w1, w2 } = await firstWithWorktrees();
    await removeFolder(w1);
    const other = repository(join(root, "other"));
    git(other, "worktree", "add", "--quiet", "-b", "o1", w1);

    expectGitsRegistries(w1, main, w2);
  });

  it("the folder above a worktree renamed, and another repository's worktree made at the old path", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "first"));
    const worktree = join(root, "side", "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", worktree);
    expectGitsRegistries(worktree, main);
    await rename(join(root, "side"), join(root, "side-old"));
    const other = repository(join(root, "other"));
    git(other, "worktree", "add", "--quiet", "-b", "o1", worktree);

    expectGitsRegistries(worktree, main, join(root, "side-old", "w"));
  });

  it("a worktree whose .git file is rewritten to name another repository's worktree", async () => {
    const { root, main, w1, w2 } = await firstWithWorktrees();
    const other = repository(join(root, "other"));
    git(other, "worktree", "add", "--quiet", "-b", "o1", join(root, "o1"));
    await rewriteInPlace(join(w1, ".git"), `gitdir: ${join(other, ".git", "worktrees", "o1")}\n`);

    expectGitsRegistries(w1, main, w2);
  });

  it("a worktree moved with `git worktree move`, and its old folder made another repository's main checkout", async () => {
    const { root, main, w1, w2 } = await firstWithWorktrees();
    git(main, "worktree", "move", w1, join(root, "moved"));
    repository(w1);

    expectGitsRegistries(w1, main, w2, join(root, "moved"));
  });

  it("a repository whose folder another repository took over is placed by Git again before its worktrees are placed from their files", async () => {
    const root = await temporaryRoot();
    const first = repository(join(root, "repository"));
    expect(expectGitsAnswers(first)).toBe(1);
    await removeFolder(first);
    const other = repository(join(root, "repository"));
    git(other, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));

    expect(expectGitsAnswers(join(root, "linked"))).toBe(1);
    expectGitsAnswers(other);
    git(other, "worktree", "add", "--quiet", "-b", "later", join(root, "later"));
    expect(expectGitsAnswers(join(root, "later"))).toBe(0);
  });
});

describe("each caller gets an answer of its own", () => {
  it("a caller that changes the answer it was given changes no later caller's", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    for (const folder of [main, join(root, "linked")]) {
      // The first answer is placed, the second kept: each is changed as a careless caller might change it.
      for (let ask = 0; ask < 2; ask += 1) {
        const given = repositoryIdentityLocation(folder);
        Object.assign(given, { git: !given.git, workspaceRoot: root, identityPath: join(root, "elsewhere"), main: !given.main });
      }
      expect(repositoryIdentityLocation(folder), folder).toEqual(gitsAnswer(folder));
    }
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
