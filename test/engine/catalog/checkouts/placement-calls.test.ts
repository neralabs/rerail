import { execFileSync } from "node:child_process";
import { accessSync, lstatSync, mkdirSync, openSync, readlinkSync, realpathSync, statSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { repositoryIdentityLocation } from "../../../../src/engine/catalog/repository-identity.js";
import { runGit } from "../../../../src/engine/git.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit) };
});

// Each file-system call a placement makes passes through, counted.
vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof import("node:fs")>();
  const realpathSync = Object.assign(vi.fn(actual.realpathSync), { native: vi.fn(actual.realpathSync.native) });
  const counted = { accessSync: vi.fn(actual.accessSync), lstatSync: vi.fn(actual.lstatSync), openSync: vi.fn(actual.openSync) };
  return { ...actual, ...counted, readlinkSync: vi.fn(actual.readlinkSync), realpathSync, statSync: vi.fn(actual.statSync) };
});

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

async function temporaryRoot(): Promise<string> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-placement-calls-")));
  roots.push(root);
  return root;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

type Counted = "access" | "lstat" | "open" | "readlink" | "realpath" | "realpathNative" | "stat";
type Calls = Readonly<Record<Counted | "git", number>>;

function mocks(): Readonly<Record<Counted, { readonly mock: { readonly calls: readonly (readonly unknown[])[] } }>> {
  return {
    access: vi.mocked(accessSync),
    lstat: vi.mocked(lstatSync),
    open: vi.mocked(openSync),
    readlink: vi.mocked(readlinkSync),
    realpath: vi.mocked(realpathSync),
    realpathNative: vi.mocked(realpathSync.native),
    stat: vi.mocked(statSync),
  };
}

/** Each counted function's calls so far, and the Git processes started. */
function marks(): Calls {
  const counts = Object.entries(mocks()).map(([name, fn]) => [name, fn.mock.calls.length]);
  return { ...Object.fromEntries(counts), git: vi.mocked(runGit).mock.calls.length } as Calls;
}

/** Whether a call's path is `root` or below it, as the system spells paths there. */
function inside(root: string, call: readonly unknown[]): boolean {
  if (typeof call[0] !== "string") return false;
  const [path, folder] = [resolve(call[0]), root].map(spelled => (process.platform === "win32" ? spelled.toLowerCase() : spelled));
  return path === folder || path!.startsWith(`${folder}${sep}`);
}

/**
 * The file-system calls on `root` and the folders below it, and the Git processes, of one placement of `folder`, which
 * Git places in a repository. A call above `root` reads the system's temporary folder, and Git's own lookup reads the
 * folders of `PATH`, which differ from one machine to the next.
 */
function placementCalls(root: string, folder: string): Calls {
  const before = marks();
  expect(repositoryIdentityLocation(folder).git, folder).toBe(true);
  const counted = Object.entries(mocks()).map(([name, fn]) => [name, fn.mock.calls.slice(before[name as Counted]).filter(call => inside(root, call)).length]);
  return { ...Object.fromEntries(counted), git: vi.mocked(runGit).mock.calls.length - before.git } as Calls;
}

/**
 * The calls each placement below makes on the case's folders, per system the suite runs on. A worktree whose `.git`
 * file names its Git folder by a clean absolute path is reached with no real path read, and a placement that read its
 * inputs makes no gone check, so the gone check adds none of these calls.
 */
const CALLS: Readonly<Partial<Record<NodeJS.Platform, Readonly<Record<"first" | "fromFiles" | "kept" | "main", Calls>>>>> = {
  linux: {
    first: { access: 2, lstat: 10, open: 6, readlink: 0, realpath: 2, realpathNative: 10, stat: 0, git: 1 },
    fromFiles: { access: 2, lstat: 5, open: 5, readlink: 0, realpath: 0, realpathNative: 7, stat: 0, git: 0 },
    kept: { access: 2, lstat: 4, open: 4, readlink: 0, realpath: 0, realpathNative: 3, stat: 0, git: 0 },
    main: { access: 4, lstat: 12, open: 5, readlink: 0, realpath: 2, realpathNative: 6, stat: 0, git: 1 },
  },
  win32: {
    first: { access: 2, lstat: 8, open: 6, readlink: 0, realpath: 0, realpathNative: 11, stat: 0, git: 1 },
    fromFiles: { access: 2, lstat: 5, open: 5, readlink: 0, realpath: 0, realpathNative: 6, stat: 0, git: 0 },
    kept: { access: 2, lstat: 4, open: 4, readlink: 0, realpath: 0, realpathNative: 2, stat: 0, git: 0 },
    main: { access: 4, lstat: 10, open: 5, readlink: 0, realpath: 0, realpathNative: 8, stat: 0, git: 1 },
  },
};

describe("a placement of a linked worktree whose .git file is as `git worktree add` writes it", () => {
  it.skipIf(CALLS[process.platform] === undefined)("makes these file-system calls and Git starts, by Git, from its files, kept, and for its main checkout", async () => {
    const root = await temporaryRoot();
    const main = join(root, "main");
    mkdirSync(main);
    git(main, "init", "-q");
    git(main, "commit", "-q", "--allow-empty", "-m", "base");
    git(main, "worktree", "add", "-q", "-b", "one", join(root, "one"));
    git(main, "worktree", "add", "-q", "-b", "two", join(root, "two"));

    // The first worktree's inputs are read and Git asked; the second is placed from its files; the first is kept.
    const first = placementCalls(root, join(root, "one"));
    const fromFiles = placementCalls(root, join(root, "two"));
    const kept = placementCalls(root, join(root, "one"));
    const mainCalls = placementCalls(root, main);

    expect({ first, fromFiles, kept, main: mainCalls }).toEqual(CALLS[process.platform]);
  });
});
