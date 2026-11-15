import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { repositoryIdentityLocation } from "../../../src/engine/catalog/repository-identity.js";
import { createCheckoutResolver, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { gitText, runGit } from "../../../src/engine/git.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { startStateReader } from "../../../src/runtime/index.js";
import { linkFolder } from "../../support/platform/links.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

vi.mock("../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit), gitText: vi.fn(actual.gitText) };
});

const temporaryDirectories: string[] = [];
const openDatabases: RerailDatabase[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
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

function database(root: string): RerailDatabase {
  const opened = openRerailDatabase(join(root, "rerail.sqlite"));
  openDatabases.push(opened);
  return opened;
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
});

describe("a folder Git may place in a repository is asked of Git, and answered as Git answers", () => {
  it("a checkout, a folder in it, a linked worktree and a work tree whose Git folder is kept apart", async () => {
    const root = await plainRoot();
    const main = join(root, "main");
    mkdirSync(join(main, "src"), { recursive: true });
    git(main, "init", "-q");
    git(main, "commit", "-q", "--allow-empty", "-m", "base");
    git(main, "worktree", "add", "-q", "-b", "linked", join(root, "linked"));
    git(root, "init", "-q", `--separate-git-dir=${join(root, "kept-apart.git")}`, join(root, "work"));

    for (const folder of [main, join(main, "src"), join(root, "linked"), join(root, "work")]) {
      expectGitsAnswer(folder);
      expect(repositoryIdentityLocation(folder).git).toBe(true);
      expect(locateCalls(folder)).toBeGreaterThan(0);
    }
  });

  it("a bare repository, a folder inside a Git folder, and an empty .git folder", async () => {
    const root = await plainRoot();
    git(root, "init", "-q", "--bare", join(root, "bare.git"));
    git(root, "init", "-q", `--separate-git-dir=${join(root, "kept-apart.git")}`, join(root, "work"));
    mkdirSync(join(root, "stray", ".git"), { recursive: true });

    for (const folder of [join(root, "bare.git"), join(root, "bare.git", "refs"), join(root, "kept-apart.git", "refs"), join(root, "stray")]) {
      expectGitsAnswer(folder);
      expect(locateCalls(folder)).toBeGreaterThan(0);
    }
  });

  it("a link into a checkout, and a link inside a checkout to a folder outside one", async () => {
    const root = await plainRoot();
    const main = join(root, "main");
    mkdirSync(join(main, "src"), { recursive: true });
    git(main, "init", "-q");
    mkdirSync(join(root, "outside"));
    linkFolder(join(main, "src"), join(root, "into-checkout"));
    linkFolder(join(root, "outside"), join(main, "out-of-checkout"));

    for (const folder of [join(root, "into-checkout"), join(main, "out-of-checkout")]) {
      expectGitsAnswer(folder);
      expect(locateCalls(folder)).toBeGreaterThan(0);
    }
  });

  it("a plain folder while GIT_DIR names a repository, and a folder that is gone", async () => {
    const root = await plainRoot();
    const main = join(root, "main");
    mkdirSync(main);
    git(main, "init", "-q");
    const plain = join(root, "plain");
    mkdirSync(plain);
    writeFileSync(join(plain, "README.md"), "notes\n");
    vi.stubEnv("GIT_DIR", join(main, ".git"));

    expectGitsAnswer(plain);
    expect(repositoryIdentityLocation(plain).git).toBe(true);
    expect(locateCalls(plain)).toBeGreaterThan(0);

    vi.unstubAllEnvs();
    const gone = join(root, "gone");
    expectGitsAnswer(gone);
    expect(locateCalls(gone)).toBeGreaterThan(0);
  });
});
