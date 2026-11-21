import { execFileSync } from "node:child_process";
import { accessSync, lstatSync, mkdirSync, openSync, readlinkSync, realpathSync, statSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
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

type Calls = Readonly<Record<"access" | "lstat" | "open" | "readlink" | "realpath" | "realpathNative" | "stat" | "git", number>>;

function tally(): Calls {
  return {
    access: vi.mocked(accessSync).mock.calls.length,
    lstat: vi.mocked(lstatSync).mock.calls.length,
    open: vi.mocked(openSync).mock.calls.length,
    readlink: vi.mocked(readlinkSync).mock.calls.length,
    realpath: vi.mocked(realpathSync).mock.calls.length,
    realpathNative: vi.mocked(realpathSync.native).mock.calls.length,
    stat: vi.mocked(statSync).mock.calls.length,
    git: vi.mocked(runGit).mock.calls.length,
  };
}

/** The file-system calls and Git starts of one placement of `folder`, which Git places in a repository. */
function placementCalls(folder: string): Calls {
  const before = tally();
  expect(repositoryIdentityLocation(folder).git, folder).toBe(true);
  const after = tally();
  return Object.fromEntries(Object.entries(after).map(([name, count]) => [name, count - before[name as keyof Calls]])) as unknown as Calls;
}

/**
 * The calls each placement below makes, per system the suite runs on: a placement of a worktree whose `.git` file names
 * its Git folder by a clean absolute path reads no real path of that Git folder's spelling and makes no gone check
 * where its placement inputs were read, so a worktree of the gone check costs what one costs without it.
 */
const CALLS: Readonly<Partial<Record<NodeJS.Platform, Readonly<Record<"first" | "fromFiles" | "kept" | "main", Calls>>>>> = {
  linux: {
    first: { access: 3, lstat: 10, open: 6, readlink: 0, realpath: 4, realpathNative: 11, stat: 2, git: 1 },
    fromFiles: { access: 2, lstat: 5, open: 5, readlink: 0, realpath: 2, realpathNative: 5, stat: 0, git: 0 },
    kept: { access: 2, lstat: 4, open: 4, readlink: 0, realpath: 0, realpathNative: 3, stat: 0, git: 0 },
    main: { access: 4, lstat: 12, open: 5, readlink: 0, realpath: 4, realpathNative: 4, stat: 0, git: 1 },
  },
  win32: {
    first: { access: 4, lstat: 8, open: 6, readlink: 0, realpath: 0, realpathNative: 15, stat: 5, git: 1 },
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
    const first = placementCalls(join(root, "one"));
    const fromFiles = placementCalls(join(root, "two"));
    const kept = placementCalls(join(root, "one"));
    const mainCalls = placementCalls(main);

    expect({ first, fromFiles, kept, main: mainCalls }).toEqual(CALLS[process.platform]);
  });
});
