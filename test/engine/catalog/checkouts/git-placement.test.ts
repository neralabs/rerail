import { execFileSync, spawnSync } from "node:child_process";
import { closeSync, cpSync, ftruncateSync, mkdirSync, openSync, realpathSync, symlinkSync, writeSync } from "node:fs";
import { chmod, mkdtemp, open, readFile, rename, rm, unlink, writeFile } from "node:fs/promises";
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

/** Git's own answer for `folder`, in `env` (this process's environment by default), as Rerail reads it from `git rev-parse`. */
function gitsAnswer(folder: string, env: NodeJS.ProcessEnv = process.env): RepositoryIdentityLocation {
  const requested = resolve(folder);
  const result = spawnSync("git", ["-C", requested, "rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"], { encoding: "utf8", env });
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

  // chmod takes nothing from root, and Windows reads no mode bits.
  it.skipIf(process.platform === "win32" || process.geteuid?.() === 0)("a worktree whose repository's config the user cannot read, which Git refuses", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    expect(expectGitsAnswers(main)).toBe(1);
    const config = join(main, ".git", "config");
    await chmod(config, 0o000);
    cleanups.push(() => chmod(config, 0o644));

    expect(gitsAnswer(join(root, "w")).git).toBe(false);
    expect(expectGitsAnswers(join(root, "w"))).toBe(1);
  });

  it("a worktree whose .git file is larger than Git reads, which Git refuses", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    expect(expectGitsAnswers(main)).toBe(1);
    const dotGit = join(root, "w", ".git");
    // The file still names the worktree's Git folder: only its size tells Git's refusal.
    await rewriteInPlace(dotGit, `${await readFile(dotGit, "utf8")}${"\n".repeat(1 << 20)}`);

    expect(gitsAnswer(join(root, "w")).git).toBe(false);
    expect(expectGitsAnswers(join(root, "w"))).toBe(1);
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

  it("lists a checkout's worktrees with one placement while a lookup variable is set, as Git lists them", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    const other = repository(join(root, "other"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    git(other, "worktree", "add", "--quiet", "-b", "elsewhere", join(root, "elsewhere"));
    const lists = (): number => vi.mocked(runGit).mock.calls.filter(([cwd, args]) => cwd === main && args[0] === "worktree").length;
    // Each value names another common folder, so each lists once; the second is Git's answer only under its value.
    const variables: Array<[string, string]> = [["GIT_OBJECT_DIRECTORY", join(main, ".git", "objects")], ["GIT_DIR", join(other, ".git")]];
    for (const [name, value] of variables) {
      vi.unstubAllEnvs();
      vi.stubEnv(name, value);
      const [placements, listed] = [locateCalls(main), lists()];
      for (let use = 0; use < 3; use += 1) {
        const registry = worktreeRegistry(main);
        expect(registry?.worktrees, `${name}=${value}`).toEqual(gitsRegistry(main));
        expect(registry?.commonDirectory, `${name}=${value}`).toBe(physicalPath(dirname(gitsAnswer(main).identityPath)));
      }
      // Main's registry kept the common folder Git first named; each value of the variables is asked once.
      expect({ placements: locateCalls(main) - placements, lists: lists() - listed }, `${name}=${value}`).toEqual({ placements: 1, lists: 1 });
    }

    // A refusal is not kept: Git takes the folder once the repository the variable names is there.
    vi.unstubAllEnvs();
    vi.stubEnv("GIT_DIR", join(root, "later", ".git"));
    expect(worktreeRegistry(main)).toBeUndefined();
    vi.unstubAllEnvs();
    repository(join(root, "later"));
    vi.stubEnv("GIT_DIR", join(root, "later", ".git"));
    expect(worktreeRegistry(main)?.worktrees).toEqual(gitsRegistry(main));
  });

  it("asks Git again under a lookup variable once Git refused the checkout, when the checkout's files come back as they were", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    // An object store outside the files the placement reads: taking it away makes Git refuse every folder.
    const objects = join(root, "objects");
    cpSync(join(main, ".git", "objects"), objects, { recursive: true });
    vi.stubEnv("GIT_OBJECT_DIRECTORY", objects);
    expect(worktreeRegistry(main)?.worktrees).toEqual(gitsRegistry(main));

    await rename(objects, `${objects}.away`);
    const config = join(main, ".git", "config");
    await rename(config, `${config}.away`);
    expect(worktreeRegistry(main)).toBeUndefined();
    // Moved back, the config gives the checkout the placement inputs Git accepted it under; Git still refuses it.
    await rename(`${config}.away`, config);
    expect(gitsRegistry(main)).toEqual([]);
    expect(worktreeRegistry(main)).toBeUndefined();
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

  it("lists a checkout as Git does once a folder Git placed above while no engine could tell owners becomes a repository and is placed", async () => {
    const root = await temporaryRoot();
    const outer = repository(join(root, "outer"));
    git(outer, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    const inner = join(outer, "inner");
    mkdirSync(inner);
    // With no owner read the folder's facts cannot be read, and Git's acceptance is kept with no stamp.
    vi.mocked(pathsOwnedByCurrentUser).mockReturnValue(undefined);
    expect(registryPlacements(inner)).toBe(1);

    repository(inner);
    expect(repositoryIdentityLocation(inner)).toEqual(gitsAnswer(inner));
    expect(worktreeRegistry(inner)?.worktrees).toEqual(gitsRegistry(inner));
    expect(gitsRegistry(inner)).toHaveLength(1);
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

/**
 * Has every Git process Rerail starts read the folders as another account's, as Git does for a folder `chown` or
 * `takeown` gave to one, while Rerail's own reads see the environment as it is.
 */
function gitSeesAnotherOwner(): void {
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

describe("a kept answer follows the folders' owners, and the user's trust in another account's, at the next ask", () => {
  it("asks Git at the next ask once the folders are another account's, and at every ask while they are", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    const folders = [main, join(root, "linked")];
    expect(expectGitsAnswers(...folders)).toBe(1);
    expect(expectGitsAnswers(...folders)).toBe(0);

    takeOwnersAway();
    expect(expectGitsAnswers(...folders)).toBe(2);
    expect(expectGitsAnswers(...folders)).toBe(2);
  });

  // Root's own folders are root's whatever SUDO_UID holds, so the case needs a test run that is not root's.
  it.skipIf(process.platform === "win32" || process.geteuid?.() === 0)("takes the folders of the account SUDO_UID names for root's, as Git does", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    const folders = [main, join(root, "linked")];
    const user = process.geteuid!();
    const spy = vi.spyOn(process, "geteuid").mockReturnValue(0);
    cleanups.push(async () => {
      spy.mockRestore();
    });

    vi.stubEnv("SUDO_UID", String(user));
    expect(expectGitsAnswers(...folders, ...folders)).toBe(1);
    vi.stubEnv("SUDO_UID", String(user + 1));
    expect(expectGitsAnswers(...folders, ...folders)).toBe(4);
  });

  it("gives no worktree list once the checkout's folders are another account's that no safe.directory entry names", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    expect(worktreeRegistry(main)?.worktrees).toHaveLength(1);

    vi.stubEnv("GIT_CONFIG_GLOBAL", join(root, "no.gitconfig"));
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    takeOwnersAway();
    gitSeesAnotherOwner();
    expect(worktreeRegistry(main)).toBeUndefined();
  });

  it("lists another account's trusted checkout with no Git process beside its placements, and none once Git refuses it", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    const global = join(root, "global.gitconfig");
    await writeFile(global, "[safe]\n\tdirectory = *\n");
    vi.stubEnv("GIT_CONFIG_GLOBAL", global);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    takeOwnersAway();
    gitSeesAnotherOwner();
    const lists = (): number => vi.mocked(runGit).mock.calls.filter(([, args]) => args[0] === "worktree").length;
    const [placements, listed] = [locateCalls(main), lists()];

    for (let use = 0; use < 3; use += 1) {
      expect(repositoryIdentityLocation(main).git).toBe(true);
      expect(worktreeRegistry(main)?.worktrees).toHaveLength(2);
    }
    // Each placement asks Git, as before answers were kept; the registry takes the answer Git gave last.
    expect({ placements: locateCalls(main) - placements, lists: lists() - listed }).toEqual({ placements: 3, lists: 1 });
    await writeFile(global, "");
    expect(repositoryIdentityLocation(main).git).toBe(false);
    expect(worktreeRegistry(main)).toBeUndefined();
    // The registry asks Git again for a folder Git refused, as it did before answers were kept.
    expect(locateCalls(main) - placements).toBe(5);
  });

  it("lists another account's checkout again once its safe.directory entry is given back, before it is placed again", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    const global = join(root, "global.gitconfig");
    const trust = "[safe]\n\tdirectory = *\n";
    await writeFile(global, trust);
    vi.stubEnv("GIT_CONFIG_GLOBAL", global);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    takeOwnersAway();
    gitSeesAnotherOwner();
    expect(worktreeRegistry(main)?.worktrees).toHaveLength(2);

    await writeFile(global, "");
    expect(repositoryIdentityLocation(main).git).toBe(false);
    await writeFile(global, trust);
    // The person gave their trust back: Git lists the worktrees again, and a session in a new worktree asks the
    // registry of the recorded checkout before anything places that checkout again.
    git(main, "worktree", "add", "--quiet", "-b", "new", join(root, "new"));
    expect(worktreeRegistry(main)?.worktrees).toEqual(gitsRegistry(main));
    expect(gitsRegistry(main)).toHaveLength(3);
  });

  it("places another account's checkout Git trusts through safe.directory, and no longer once the entry is removed", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    const folders = [main, join(root, "linked")];
    const global = join(root, "global.gitconfig");
    await writeFile(global, "[safe]\n\tdirectory = *\n");
    vi.stubEnv("GIT_CONFIG_GLOBAL", global);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    takeOwnersAway();
    gitSeesAnotherOwner();
    const asGit = (): RepositoryIdentityLocation[] => folders.map(folder => gitsAnswer(folder, { ...process.env, GIT_TEST_ASSUME_DIFFERENT_OWNER: "1" }));
    const asRerail = (): RepositoryIdentityLocation[] => folders.map(folder => repositoryIdentityLocation(folder));
    expect(asGit().map(answer => answer.git)).toEqual([true, true]);
    expect(asRerail()).toEqual(asGit());
    expect(asRerail()).toEqual(asGit());

    await writeFile(global, "");
    expect(asGit().map(answer => answer.git)).toEqual([false, false]);
    expect(asRerail()).toEqual(asGit());
  });

  it("gives no list for another account's checkout whose files come back as they were after Git refused it", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    const global = join(root, "global.gitconfig");
    await writeFile(global, "[safe]\n\tdirectory = *\n");
    vi.stubEnv("GIT_CONFIG_GLOBAL", global);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    takeOwnersAway();
    gitSeesAnotherOwner();
    expect(worktreeRegistry(main)?.worktrees).toHaveLength(1);

    // While its .git is a link the files cannot describe it, and Git, the safe.directory entry gone, refuses it.
    await rename(join(main, ".git"), join(root, "main.git"));
    linkFolder(join(root, "main.git"), join(main, ".git"));
    await writeFile(global, "");
    expect(worktreeRegistry(main)).toBeUndefined();
    await unlink(join(main, ".git"));
    await rename(join(root, "main.git"), join(main, ".git"));
    expect(gitsAnswer(main, { ...process.env, GIT_TEST_ASSUME_DIFFERENT_OWNER: "1" }).git).toBe(false);
    expect(worktreeRegistry(main)).toBeUndefined();
  });
});

/** A repository at `path` whose `.git` is a link to its Git folder, moved to `store`; returns the checkout's real path. */
async function gitFolderLinked(path: string, store: string): Promise<string> {
  const main = repository(path);
  await rename(join(main, ".git"), store);
  linkFolder(store, join(main, ".git"));
  return main;
}

/** Git's list of the worktrees of `checkout`; undefined where Git refuses the folder. */
function listedByGit(checkout: string): string[] | undefined {
  return gitsAnswer(checkout).git ? gitsRegistry(checkout) : undefined;
}

/** Asks the registry of `checkout` three times, each against Git's list; returns how many Git processes placed it. */
function registryPlacements(checkout: string): number {
  const before = locateCalls(checkout);
  for (let use = 0; use < 3; use += 1) expect(worktreeRegistry(checkout)?.worktrees, checkout).toEqual(listedByGit(checkout));
  return locateCalls(checkout) - before;
}

describe("the worktree registry keeps Git's acceptance of a checkout its files cannot describe while what Git reads there is unchanged", () => {
  it("places a checkout whose .git is a link once for three lists, under a lookup variable too", async () => {
    const root = await temporaryRoot();
    const main = await gitFolderLinked(join(root, "main"), join(root, "main.git"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    expect(registryPlacements(main)).toBe(1);

    vi.stubEnv("GIT_OBJECT_DIRECTORY", join(root, "main.git", "objects"));
    expect(registryPlacements(main)).toBe(1);
  });

  it("places such a checkout once for lists around a commit at a detached HEAD and a switch to another branch, and again once Git no longer takes its HEAD", async () => {
    const root = await temporaryRoot();
    const main = await gitFolderLinked(join(root, "main"), join(root, "main.git"));
    git(main, "checkout", "--quiet", "--detach");
    expect(registryPlacements(main)).toBe(1);
    git(main, "commit", "--quiet", "--allow-empty", "-m", "next");
    expect(registryPlacements(main)).toBe(0);
    git(main, "checkout", "--quiet", "-b", "other");
    expect(registryPlacements(main)).toBe(0);

    await writeFile(join(root, "main.git", "HEAD"), "no commit\n");
    expect(gitsAnswer(main).git).toBe(false);
    expect(worktreeRegistry(main)).toBeUndefined();
  });

  it("places such a checkout once for lists around config writes that place no checkout, and again once one does", async () => {
    const root = await temporaryRoot();
    const main = await gitFolderLinked(join(root, "main"), join(root, "main.git"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "linked"));
    expect(registryPlacements(main)).toBe(1);
    git(main, "remote", "add", "origin", join(root, "elsewhere.git"));
    git(main, "config", "branch.linked.merge", "refs/heads/linked");
    expect(registryPlacements(main)).toBe(0);

    git(main, "config", "core.repositoryformatversion", "1");
    git(main, "config", "extensions.worktreeConfig", "true");
    expect(registryPlacements(main)).toBe(1);
    git(main, "config", "--worktree", "core.bare", "true");
    expect(gitsAnswer(main).git).toBe(false);
    expect(worktreeRegistry(main)).toBeUndefined();
  });

  it.skipIf(process.platform === "win32")("places such a checkout whose HEAD is a link again only once the link stops naming a ref", async () => {
    // Git for Windows writes no HEAD link; a link Git reads there needs a privilege a person may not have.
    const root = await temporaryRoot();
    const main = await gitFolderLinked(join(root, "main"), join(root, "main.git"));
    const head = join(root, "main.git", "HEAD");
    git(main, "branch", "other");
    const branch = execFileSync("git", ["symbolic-ref", "HEAD"], { cwd: main, encoding: "utf8" }).trim();
    await unlink(head);
    symlinkSync(branch, head);
    expect(registryPlacements(main)).toBe(1);
    await unlink(head);
    symlinkSync("refs/heads/other", head);
    expect(registryPlacements(main)).toBe(0);

    // Git takes a HEAD link only by its text, so a link naming the same ref by its full path is refused.
    await unlink(head);
    symlinkSync(join(root, "main.git", "refs", "heads", "other"), head);
    expect(gitsAnswer(main).git).toBe(false);
    expect(worktreeRegistry(main)).toBeUndefined();

  });

  it("places a worktree whose .git file names a link to its Git folder once, and again once the link names another", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    const other = repository(join(root, "other"));
    const worktree = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", worktree);
    git(other, "worktree", "add", "--quiet", "-b", "w", join(root, "ow"));
    const link = join(root, "w.git");
    linkFolder(join(main, ".git", "worktrees", "w"), link);
    await rewriteInPlace(join(worktree, ".git"), `gitdir: ${link}\n`);
    // Git for Windows refuses a .git file that names a junction to a worktree's Git folder: there each list asks Git, as
    // the registry did before, since a refusal is never kept.
    const placements = gitsAnswer(worktree).git ? 1 : 3;
    expect(registryPlacements(worktree)).toBe(placements);

    // Git refuses the folder once the link names another worktree's Git folder.
    await relink(link, join(other, ".git", "worktrees", "w"));
    expect(gitsAnswer(worktree).git).toBe(false);
    expect(worktreeRegistry(worktree)).toBeUndefined();
    await relink(link, join(main, ".git", "worktrees", "w"));
    expect(registryPlacements(worktree)).toBe(placements);
  });

  it("lists such a worktree as Git does once its commondir names another repository, that repository's config moves, or the folder it names is gone", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    const other = repository(join(root, "other"));
    const worktree = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", worktree);
    git(other, "worktree", "add", "--quiet", "-b", "w", join(root, "ow"));
    const link = join(root, "w.git");
    linkFolder(join(main, ".git", "worktrees", "w"), link);
    await rewriteInPlace(join(worktree, ".git"), `gitdir: ${link}\n`);
    // Git for Windows refuses such a .git file while commondir holds the relative path `git worktree add` wrote, and
    // takes it once commondir names a common Git folder by its full path: where Git refuses, each list asks it and
    // nothing is kept; where it takes the folder, three lists ask it once.
    const placements = (): number => (gitsAnswer(worktree).git ? 1 : 3);
    expect(registryPlacements(worktree)).toBe(placements());

    const commondir = join(main, ".git", "worktrees", "w", "commondir");
    await rewriteInPlace(commondir, `${join(main, ".git")}\n`);
    expect(gitsAnswer(worktree).git).toBe(true);
    expect(registryPlacements(worktree)).toBe(1);
    await rewriteInPlace(commondir, `${join(other, ".git")}\n`);
    expect(registryPlacements(worktree)).toBe(placements());
    git(other, "config", "core.repositoryformatversion", "1");
    git(other, "config", "extensions.worktreeConfig", "true");
    expect(registryPlacements(worktree)).toBe(placements());

    await rewriteInPlace(commondir, `${join(root, "gone")}\n`);
    expect(gitsAnswer(worktree).git).toBe(false);
    expect(worktreeRegistry(worktree)).toBeUndefined();
  });

  it("gives no list for a checkout whose .git is a link once its objects folder is gone, as Git gives none", async () => {
    const root = await temporaryRoot();
    const main = await gitFolderLinked(join(root, "main"), join(root, "main.git"));
    expect(registryPlacements(main)).toBe(1);

    await rename(join(root, "main.git", "objects"), join(root, "objects"));
    expect(gitsAnswer(main).git).toBe(false);
    expect(worktreeRegistry(main)).toBeUndefined();
    await rename(join(root, "objects"), join(root, "main.git", "objects"));
    expect(registryPlacements(main)).toBe(1);
  });

  it("lists as Git does once a checkout's .git link names another Git folder, and none once that Git folder moves", async () => {
    const root = await temporaryRoot();
    const main = await gitFolderLinked(join(root, "main"), join(root, "main.git"));
    const other = await gitFolderLinked(join(root, "other"), join(root, "other.git"));
    git(other, "worktree", "add", "--quiet", "-b", "o", join(root, "o"));
    expectGitsRegistries(main);

    await relink(join(main, ".git"), join(root, "other.git"));
    expectGitsRegistries(main);
    expect(worktreeRegistry(main)?.worktrees).toHaveLength(2);
    await rename(join(root, "other.git"), join(root, "moved.git"));
    expect(gitsAnswer(main).git).toBe(false);
    expect(worktreeRegistry(main)).toBeUndefined();
  });

  it("gives no list for a checkout whose .git is a link once its folders are another account's, and Git's last acceptance while safe.directory names it", async () => {
    const root = await temporaryRoot();
    const main = await gitFolderLinked(join(root, "main"), join(root, "main.git"));
    expect(worktreeRegistry(main)?.worktrees).toEqual(gitsRegistry(main));
    const global = join(root, "global.gitconfig");
    await writeFile(global, "");
    vi.stubEnv("GIT_CONFIG_GLOBAL", global);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    takeOwnersAway();
    gitSeesAnotherOwner();
    expect(worktreeRegistry(main)).toBeUndefined();

    await writeFile(global, "[safe]\n\tdirectory = *\n");
    const before = locateCalls(main);
    for (let use = 0; use < 3; use += 1) expect(worktreeRegistry(main)?.worktrees).toHaveLength(1);
    expect(locateCalls(main) - before).toBe(1);
    // With the entry removed, the next placement asks Git, which refuses, and the registry keeps nothing.
    await writeFile(global, "");
    expect(repositoryIdentityLocation(main).git).toBe(false);
    expect(worktreeRegistry(main)).toBeUndefined();
  });

  it("places a recorded checkout Git places in the repository above it once for three lists, as the registry did before", async () => {
    const root = await temporaryRoot();
    const outer = repository(join(root, "outer"));
    const inner = join(outer, "inner");
    // An empty Git folder, which Git passes over to the repository above.
    mkdirSync(join(inner, ".git"), { recursive: true });
    expect(gitsAnswer(inner).workspaceRoot).toBe(outer);
    expect(registryPlacements(inner)).toBe(1);
  });

  it("asks Git again for such a checkout once Git cannot list the repository above it", async () => {
    const root = await temporaryRoot();
    const outer = repository(join(root, "outer"));
    git(outer, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    const inner = join(outer, "inner");
    mkdirSync(join(inner, ".git"), { recursive: true });
    expect(registryPlacements(inner)).toBe(1);

    await rename(join(outer, ".git"), join(root, "outer.git"));
    expect(worktreeRegistry(inner)).toBeUndefined();
    await removeFolder(join(inner, ".git"));
    repository(inner);
    expectGitsRegistries(inner);
  });

  it("lists as Git does once such a checkout is removed, asked while gone, and made a repository of its own", async () => {
    const root = await temporaryRoot();
    const outer = repository(join(root, "outer"));
    git(outer, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    const inner = join(outer, "inner");
    mkdirSync(join(inner, ".git"), { recursive: true });
    expect(registryPlacements(inner)).toBe(1);

    await removeFolder(inner);
    expect(worktreeRegistry(inner)).toBeUndefined();
    repository(inner);
    expect(worktreeRegistry(inner)?.worktrees).toEqual(gitsRegistry(inner));
    expect(worktreeRegistry(inner)?.worktrees).toHaveLength(1);
  });

  it("lists as Git does once such a checkout becomes a repository of its own and is placed again", async () => {
    const root = await temporaryRoot();
    const outer = repository(join(root, "outer"));
    git(outer, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    const inner = join(outer, "inner");
    mkdirSync(join(inner, ".git"), { recursive: true });
    expect(registryPlacements(inner)).toBe(1);

    await removeFolder(join(inner, ".git"));
    repository(inner);
    expect(repositoryIdentityLocation(inner)).toEqual(gitsAnswer(inner));
    expect(worktreeRegistry(inner)?.worktrees).toEqual(gitsRegistry(inner));
    expect(worktreeRegistry(inner)?.worktrees).toHaveLength(1);
  });

  it("lists as Git does once a folder Git places in the repository above becomes a repository of its own, with no placement between", async () => {
    const root = await temporaryRoot();
    const outer = repository(join(root, "outer"));
    const inner = join(outer, "inner");
    const emptied = join(outer, "emptied");
    mkdirSync(inner);
    mkdirSync(join(emptied, ".git"), { recursive: true });
    expect(registryPlacements(inner)).toBe(1);
    expect(registryPlacements(emptied)).toBe(1);
    expect(worktreeRegistry(outer)?.worktrees).toEqual(gitsRegistry(outer));

    repository(inner);
    await removeFolder(join(emptied, ".git"));
    repository(emptied);
    for (const checkout of [inner, emptied]) expect(worktreeRegistry(checkout)?.worktrees, checkout).toEqual(gitsRegistry(checkout));
    expect(gitsRegistry(inner)).toHaveLength(1);
    git(outer, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    expect(worktreeRegistry(inner)?.worktrees).toEqual(gitsRegistry(inner));
    for (let use = 0; use < 3; use += 1) expect(worktreeRegistry(outer)?.worktrees).toEqual(gitsRegistry(outer));
  });

});

describe("an entry Git reads through a link is read through it", () => {
  it("a checkout whose objects folder is a link, once the folder it names is gone", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    await rename(join(main, ".git", "objects"), join(root, "objects"));
    linkFolder(join(root, "objects"), join(main, ".git", "objects"));
    expect(expectGitsAnswers(main, main)).toBe(1);
    const answer = gitsAnswer(main);

    await rename(join(root, "objects"), join(root, "objects-gone"));
    // Git on Linux reads objects through the link and now refuses the checkout; Git for Windows reads the junction's
    // own entry, as Node's access does there, and still takes it.
    if (process.platform !== "win32") expect(gitsAnswer(main)).not.toEqual(answer);
    expectGitsAnswers(main);
  });

  it.skipIf(process.platform !== "win32")("asks Git for a worktree reached through a junction whose real folder the engine does not take for the user's", async () => {
    const root = await temporaryRoot();
    const main = repository(join(root, "main"));
    const real = join(root, "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", real);
    const link = join(root, "L");
    linkFolder(real, link);
    expect(expectGitsAnswers(main)).toBe(1);
    vi.stubEnv("GIT_CONFIG_GLOBAL", join(root, "no.gitconfig"));
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    vi.mocked(pathsOwnedByCurrentUser).mockImplementation(paths => !paths.some(path => path.toLowerCase() === real.toLowerCase()));
    gitSeesAnotherOwner();

    expect(repositoryIdentityLocation(link)).toEqual(gitsAnswer(link, { ...process.env, GIT_TEST_ASSUME_DIFFERENT_OWNER: "1" }));
    expect(repositoryIdentityLocation(link).git).toBe(false);
    expect(vi.mocked(pathsOwnedByCurrentUser).mock.lastCall?.[0][0]).toBe(real);
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
