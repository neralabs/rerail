import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { repositoryIdentityLocation, repositoryIdentityLocationAsync } from "../../../src/engine/catalog/repository-identity.js";
import { createCheckoutResolver, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { gitText, runGit, runGitAsync } from "../../../src/engine/git.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { startStateReader } from "../../../src/runtime/index.js";
import { linkFolder } from "../../support/platform/links.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

vi.mock("../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit), runGitAsync: vi.fn(actual.runGitAsync), gitText: vi.fn(actual.gitText) };
});

vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof import("node:fs")>();
  return { ...actual, lstatSync: vi.fn(actual.lstatSync) };
});

const temporaryDirectories: string[] = [];
const openDatabases: RerailDatabase[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  vi.mocked(lstatSync).mockReset();
  for (const database of openDatabases.splice(0)) database.$client.close();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** A new folder that Git, asked about it, places in no repository. */
async function plainRoot(): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-outside-git-")));
  temporaryDirectories.push(root);
  expect(gitTop(root), "the temporary folder is outside every repository").toBeUndefined();
  return root;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, stdio: "ignore" });
}

/** Git's own answer for `folder`, in this process's environment: its top level, or undefined outside a work tree. */
function gitTop(folder: string): string | undefined {
  const result = spawnSync("git", ["-C", folder, "rev-parse", "--show-toplevel"], { encoding: "utf8" });
  return result.status === 0 ? resolve(result.stdout.trim()) : undefined;
}

/** How many times Rerail asked Git where `folder`'s repository is. */
function locateCalls(folder: string): number {
  return vi.mocked(runGit).mock.calls.filter(([cwd, args]) => cwd === resolve(folder) && args.includes("--show-toplevel")).length;
}

/** Rerail's answer for `folder` against Git's: whether it is in Git, and its top level when it is. */
function expectGitsAnswer(folder: string): void {
  const top = gitTop(folder);
  const location = repositoryIdentityLocation(folder);
  expect({ git: location.git, top: location.git ? location.workspaceRoot : undefined }).toEqual({ git: top !== undefined, top });
}

/** Whether Rerail asked Git about `folder` while it gave Git's answer for it. */
function askedWhileAnswering(folder: string): boolean {
  const before = locateCalls(folder);
  expectGitsAnswer(folder);
  return locateCalls(folder) > before;
}

function database(root: string): RerailDatabase {
  const opened = openRerailDatabase(join(root, "rerail.sqlite"));
  openDatabases.push(opened);
  return opened;
}

/** A checkout with one commit at `root/name`. */
function checkout(root: string, name: string): string {
  const folder = folderAt(root, name);
  git(folder, "init", "-q");
  git(folder, "commit", "-q", "--allow-empty", "-m", "base");
  return folder;
}

function folderAt(...segments: string[]): string {
  const folder = join(...segments);
  mkdirSync(folder, { recursive: true });
  return folder;
}

/** `folder` with a file `name` in it. */
function withFile(folder: string, name: string, text: string): string {
  writeFileSync(join(folder, name), text);
  return folder;
}

/** `path`, made a link to the folder `target` (a junction on Windows). */
function linkAt(target: string, path: string): string {
  linkFolder(target, path);
  return path;
}

describe("a folder outside Git", () => {
  it("starts no Git process however often the daemon's readers ask about it", async () => {
    const root = await plainRoot();
    const folder = join(root, "notes");
    mkdirSync(folder);
    const db = database(root);
    const resolveCheckout = createCheckoutResolver(db);
    const startState = startStateReader(db);

    for (let event = 0; event < 3; event += 1) {
      expect(repositoryIdentityLocation(folder).git).toBe(false);
      expect(resolveCheckout(folder)).toBeUndefined();
      expect(startState(folder)).toBe("stopped");
    }

    expect(locateCalls(folder)).toBe(0);
  });

  it("Rerail runs in starts no Git process to look for a remote each time a session starts there", async () => {
    const root = await plainRoot();
    const folder = join(root, "notes");
    mkdirSync(folder);
    const db = database(root);

    for (let session = 0; session < 3; session += 1) expect(recordCheckout(db, folder).git).toBe(false);

    expect(locateCalls(folder) + vi.mocked(gitText).mock.calls.filter(([cwd]) => cwd === folder).length).toBe(0);
  });

  it("is placed in a repository made in it or above it after it was asked about", async () => {
    const root = await plainRoot();
    const folder = join(root, "app", "src");
    mkdirSync(folder, { recursive: true });
    const resolveCheckout = createCheckoutResolver(database(root));
    expect(resolveCheckout(folder)).toBeUndefined();

    git(join(root, "app"), "init", "-q");
    expect(repositoryIdentityLocation(folder)).toMatchObject({ git: true, workspaceRoot: gitTop(folder) });
    expect(gitTop(folder)).toBe(resolve(root, "app"));
  });

  it("is outside Git again, with no Git process, once its checkout's .git is gone", async () => {
    const root = await plainRoot();
    const folder = folderAt(checkout(root, "app"), "src");
    expect(askedWhileAnswering(folder)).toBe(true);

    await removeFolder(join(root, "app", ".git"));
    expect(askedWhileAnswering(folder)).toBe(false);
  });
});

describe("a linked worktree whose main checkout was deleted", () => {
  it("starts no Git process however often the daemon's readers ask about it, since Git refuses it", async () => {
    const root = await plainRoot();
    const main = checkout(root, "main");
    const worktree = join(root, "feature");
    git(main, "worktree", "add", "-q", "-b", "feature", worktree);
    const db = database(root);
    const recorded = recordCheckout(db, worktree);
    await removeFolder(main);
    expect(gitTop(worktree), "Git refuses the worktree: its .git file names a Git folder that went with the main checkout").toBeUndefined();
    vi.mocked(runGit).mockClear();
    const resolveCheckout = createCheckoutResolver(db);
    const startState = startStateReader(db);

    for (let event = 0; event < 3; event += 1) {
      expect(repositoryIdentityLocation(worktree).git).toBe(false);
      expect(resolveCheckout(worktree)?.repositoryKey).toBe(recorded.repositoryKey);
      expect(startState(worktree)).toBe("stopped");
    }

    expect(locateCalls(worktree)).toBe(0);
  });
});

describe("a folder Git places in a repository with no work tree", () => {
  it.for([
    { name: "a bare repository", make: (root: string) => bareRepository(root) },
    { name: "a folder inside a checkout's .git", make: (root: string) => join(checkout(root, "main"), ".git", "refs") },
  ])("$name is keyed by its remote", async ({ make }) => {
    const root = await plainRoot();
    const folder = make(root);
    git(folder, "remote", "add", "origin", "https://github.com/example/demo.git");

    const recorded = recordCheckout(database(root), folder);

    expect(recorded).toMatchObject({ git: false, repositoryKey: "remote:github.com/example/demo" });
  });
});

/** A folder shape: the folder Rerail is asked about, the environment it is asked in, or why this machine cannot make it. */
type Made = string | { readonly folder: string; readonly env?: Record<string, string>; readonly skip?: string };

interface Shape {
  readonly name: string;
  /** Whether Rerail asks Git: only where a `.git` or `HEAD` is in the folder or above it, or `GIT_DIR` is set. */
  readonly asks: boolean;
  readonly windowsOnly?: true;
  /** Makes the shape under `root`, a new folder outside Git. */
  make(root: string): Made;
}

/** `path` through the machine's administrative share, `\\localhost\C$\...`, or why it cannot be. */
function byShare(path: string): Made {
  const shared = path.replace(/^([A-Za-z]):/u, (_, drive: string) => `\\\\localhost\\${drive}$`);
  return existsSync(shared) ? shared : { folder: path, skip: "the administrative share is not reachable here" };
}

/** `root/notes` holding each of `names` as a folder. */
function notesHolding(root: string, names: readonly string[]): string {
  for (const name of names) folderAt(root, "notes", name);
  return join(root, "notes");
}

/** `root/notes`, asked about while the environment variable `name` is `value`. */
function notesWhile(root: string, name: string, value: string): Made {
  return { folder: folderAt(root, "notes"), env: { [name]: value } };
}

/** A checkout at `root/main` with a linked worktree at `root/linked`. */
function linkedWorktree(root: string): string {
  git(checkout(root, "main"), "worktree", "add", "-q", "-b", "linked", join(root, "linked"));
  return join(root, "linked");
}

/** A work tree at `root/work` whose Git folder is `root/kept.git`. */
function keptApart(root: string): string {
  git(root, "init", "-q", `--separate-git-dir=${join(root, "kept.git")}`, join(root, "work"));
  return join(root, "work");
}

function bareRepository(root: string): string {
  git(root, "init", "-q", "--bare", join(root, "bare.git"));
  return join(root, "bare.git");
}

/** A checkout at `root/main` with a submodule at `sub` whose own repository holds `dir/`. */
function withSubmodule(root: string): string {
  const source = checkout(root, "source");
  withFile(folderAt(source, "dir"), "a.md", "a\n");
  git(source, "add", ".");
  git(source, "commit", "-q", "-m", "dir");
  const main = checkout(root, "main");
  git(main, "-c", "protocol.file.allow=always", "submodule", "add", "-q", source, "sub");
  return main;
}

/** A checkout at `root/main` with a repository of its own made at `nested`, not a submodule. */
function nestedRepository(root: string): string {
  const nested = folderAt(checkout(root, "main"), "nested");
  git(nested, "init", "-q");
  return nested;
}

const DOT_FILES = [".gitignore", ".gitattributes", ".gitmodules"];

const outsideShapes: readonly Shape[] = [
  { name: "a new folder", asks: false, make: root => folderAt(root, "notes") },
  { name: "a folder five levels down", asks: false, make: root => folderAt(root, "a", "b", "c", "d", "e") },
  { name: "a path written with .. in it", asks: false, make: root => `${folderAt(root, "b")}${sep}..${sep}b` },
  { name: "a path written with a trailing separator", asks: false, make: root => `${folderAt(root, "notes")}${sep}` },
  { name: "a name with spaces and letters outside ASCII", asks: false, make: root => folderAt(root, "notes é ü 中") },
  { name: "a folder holding .gitignore, .gitattributes and .gitmodules files", asks: false, make: root => DOT_FILES.reduce((folder, name) => withFile(folder, name, "\n"), folderAt(root, "notes")) },
  { name: "a folder holding git, .github and .git-old folders", asks: false, make: root => notesHolding(root, ["git", ".github", ".git-old"]) },
  { name: "a folder named like a bare repository, with no HEAD", asks: false, make: root => folderAt(root, "notes.git", "refs") },
  { name: "a folder holding a link that leads nowhere", asks: false, make: root => resolve(linkAt(join(root, "missing"), join(folderAt(root, "notes"), "docs")), "..") },
  { name: "a link to a folder outside Git", asks: false, make: root => linkAt(folderAt(root, "notes"), join(root, "link")) },
  { name: "while GIT_CEILING_DIRECTORIES names its parent", asks: false, make: root => notesWhile(root, "GIT_CEILING_DIRECTORIES", root) },
  { name: "while GIT_WORK_TREE names it", asks: false, make: root => notesWhile(root, "GIT_WORK_TREE", join(root, "notes")) },
  { name: "while GIT_COMMON_DIR names a repository's Git folder", asks: false, make: root => notesWhile(root, "GIT_COMMON_DIR", join(checkout(root, "main"), ".git")) },
  { name: "while GIT_OBJECT_DIRECTORY names a repository's objects", asks: false, make: root => notesWhile(root, "GIT_OBJECT_DIRECTORY", join(checkout(root, "main"), ".git", "objects")) },
  { name: "while GIT_DISCOVERY_ACROSS_FILESYSTEM is set", asks: false, make: root => notesWhile(root, "GIT_DISCOVERY_ACROSS_FILESYSTEM", "1") },
  { name: "while GIT_INDEX_FILE is set", asks: false, make: root => notesWhile(root, "GIT_INDEX_FILE", join(root, "index")) },
  { name: "by its \\\\localhost\\C$ path", asks: false, windowsOnly: true, make: root => byShare(folderAt(root, "notes")) },
  { name: "written in other letter case", asks: false, windowsOnly: true, make: root => folderAt(root, "notes").toUpperCase() },
  { name: "written with forward slashes", asks: false, windowsOnly: true, make: root => folderAt(root, "notes").split("\\").join("/") },
];

const askedShapes: readonly Shape[] = [
  { name: "a checkout's top folder", asks: true, make: root => checkout(root, "main") },
  { name: "a folder in a checkout", asks: true, make: root => folderAt(checkout(root, "main"), "src") },
  { name: "a folder five levels down in a checkout", asks: true, make: root => folderAt(checkout(root, "main"), "a", "b", "c", "d", "e") },
  { name: "a folder the checkout's .gitignore lists", asks: true, make: root => folderAt(withFile(checkout(root, "main"), ".gitignore", "build/\n"), "build") },
  { name: "a linked worktree", asks: true, make: root => linkedWorktree(root) },
  { name: "a folder in a linked worktree", asks: true, make: root => folderAt(linkedWorktree(root), "src") },
  { name: "a work tree whose Git folder is kept apart", asks: true, make: root => keptApart(root) },
  { name: "a Git folder kept apart from its work tree", asks: true, make: root => join(resolve(keptApart(root), "..", "kept.git"), "refs") },
  { name: "a submodule", asks: true, make: root => join(withSubmodule(root), "sub") },
  { name: "a folder in a submodule", asks: true, make: root => join(withSubmodule(root), "sub", "dir") },
  { name: "a repository made inside a checkout", asks: true, make: root => nestedRepository(root) },
  { name: "a bare repository", asks: true, make: root => bareRepository(root) },
  { name: "a folder in a bare repository", asks: true, make: root => join(bareRepository(root), "refs") },
  { name: "a folder inside a checkout's .git", asks: true, make: root => join(checkout(root, "main"), ".git", "refs") },
  { name: "an empty .git folder", asks: true, make: root => notesHolding(root, [".git"]) },
  { name: "a .git file that leads nowhere", asks: true, make: root => withFile(folderAt(root, "notes"), ".git", `gitdir: ${join(root, "missing")}\n`) },
  { name: "a .git file with no gitdir line", asks: true, make: root => withFile(folderAt(root, "notes"), ".git", "notes\n") },
  { name: "a .git link to a checkout's Git folder", asks: true, make: root => resolve(linkAt(join(checkout(root, "main"), ".git"), join(folderAt(root, "notes"), ".git")), "..") },
  { name: "a .git link that leads nowhere", asks: true, make: root => resolve(linkAt(join(root, "missing"), join(folderAt(root, "notes"), ".git")), "..") },
  { name: "a folder holding a file named HEAD", asks: true, make: root => withFile(folderAt(root, "notes"), "HEAD", "notes\n") },
  { name: "a link into a checkout", asks: true, make: root => linkAt(folderAt(checkout(root, "main"), "src"), join(root, "link")) },
  { name: "a link inside a checkout to a folder outside one", asks: true, make: root => linkAt(folderAt(root, "outside"), join(checkout(root, "main"), "out")) },
  { name: "a link to a checkout's top folder", asks: true, make: root => linkAt(checkout(root, "main"), join(root, "link")) },
  { name: "a plain folder while GIT_DIR names a repository", asks: true, make: root => notesWhile(root, "GIT_DIR", join(checkout(root, "main"), ".git")) },
  { name: "a plain folder while GIT_DIR names no repository", asks: true, make: root => notesWhile(root, "GIT_DIR", folderAt(root, "empty")) },
  { name: "a folder in a checkout past GIT_CEILING_DIRECTORIES", asks: true, make: root => ({ folder: folderAt(checkout(root, "main"), "src"), env: { GIT_CEILING_DIRECTORIES: join(root, "main") } }) },
  { name: "a checkout Git takes as another owner's", asks: true, make: root => ({ folder: checkout(root, "main"), env: { GIT_TEST_ASSUME_DIFFERENT_OWNER: "1" } }) },
  { name: "a folder that is gone", asks: true, make: root => join(root, "gone") },
  { name: "a link that leads nowhere", asks: true, make: root => linkAt(join(root, "missing"), join(root, "link")) },
  { name: "a folder in a checkout by its \\\\localhost\\C$ path", asks: true, windowsOnly: true, make: root => byShare(folderAt(checkout(root, "main"), "src")) },
  { name: "a folder in a checkout written in other letter case", asks: true, windowsOnly: true, make: root => folderAt(checkout(root, "main"), "src").toUpperCase() },
];

const shapesHere = [...outsideShapes, ...askedShapes].filter(shape => process.platform === "win32" || shape.windowsOnly !== true);

describe("each folder shape gets Git's answer, and Git is asked only where a .git, a HEAD or GIT_DIR can lead it to a repository", () => {
  it.for(shapesHere)("$name", async (shape, context) => {
    const made = shape.make(await plainRoot());
    const { folder, env = {}, skip }: Exclude<Made, string> = typeof made === "string" ? { folder: made } : made;
    if (skip !== undefined) context.skip(skip);
    for (const [name, value] of Object.entries(env)) vi.stubEnv(name, value);

    expect(askedWhileAnswering(folder), "Rerail asked Git").toBe(shape.asks);
  });

  it("an entry above the folder that cannot be read is taken as there, and Git is asked", async () => {
    const root = await plainRoot();
    const folder = folderAt(root, "notes");
    const { lstatSync: actualLstat } = await vi.importActual<typeof import("node:fs")>("node:fs");
    const denied = join(root, ".git");
    const deniedLstat = (path: Parameters<typeof lstatSync>[0], options?: Parameters<typeof lstatSync>[1]): ReturnType<typeof lstatSync> => {
      if (resolve(String(path)) === denied) throw Object.assign(new Error(`EACCES: permission denied, lstat '${String(path)}'`), { code: "EACCES" });
      return actualLstat(path, options);
    };
    vi.mocked(lstatSync).mockImplementation(deniedLstat as typeof lstatSync);

    expect(askedWhileAnswering(folder), "Rerail asked Git").toBe(true);
  });
});

describe("the location read off the thread", () => {
  it("is the blocking read's for a checkout, a folder in it, a linked worktree, a folder outside Git and one that is gone, with no Git outside Git", async () => {
    const root = await plainRoot();
    const main = checkout(root, "main");
    git(main, "worktree", "add", "-q", join(root, "linked"));
    const outside = folderAt(root, "notes");
    const folders = [main, folderAt(main, "src"), join(root, "linked"), outside, join(root, "gone")];

    for (const folder of folders) expect(await repositoryIdentityLocationAsync(folder), folder).toEqual(repositoryIdentityLocation(folder));
    expect(vi.mocked(runGitAsync).mock.calls.filter(([cwd]) => cwd === outside)).toEqual([]);
  });

  it("starts no Git once its signal is aborted, so even a checkout reads as outside Git", async () => {
    const root = await plainRoot();
    const main = checkout(root, "main");

    const location = await repositoryIdentityLocationAsync(main, AbortSignal.abort());

    expect({ git: location.git, workspaceRoot: location.workspaceRoot }).toEqual({ git: false, workspaceRoot: resolve(main) });
  });
});
