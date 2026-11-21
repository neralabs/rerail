import { execFileSync } from "node:child_process";
import { closeSync, mkdirSync, openSync, writeFileSync } from "node:fs";
import { mkdtemp, open } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  definesNoDriver, nameDiff, readStatus, recordsNoSubmodule, submoduleChanged, UNASKED, worktreeGitFolder, worktreeIndex,
} from "../../../src/engine/git-guard/submodule-state.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

vi.mock("node:fs", { spy: true });
vi.mock("node:fs/promises", { spy: true });
const realFs = await vi.importActual<typeof import("node:fs")>("node:fs");
const realPromises = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");

/** The audit's `git diff` (`audit-control.ts`) and the scan's (`headless-scan.ts`), as each runs it. */
const AUDIT_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"];
const SCAN_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", "HEAD", "--"];

const roots: string[] = [];
afterEach(async () => {
  vi.mocked(openSync).mockReset();
  vi.mocked(closeSync).mockReset();
  vi.mocked(open).mockReset();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

describe("nameDiff and readStatus", () => {
  it.each([
    [AUDIT_DIFF, { relative: true, ignore: "none", listsModified: true }],
    [SCAN_DIFF, { relative: true, ignore: undefined, listsModified: true }],
    [["diff", "--name-only", "-z", "--diff-filter=d", "--"], { relative: false, ignore: undefined, listsModified: true }],
    [["diff", "--name-only", "-z", "--diff-filter=AD", "--"], { relative: false, ignore: undefined, listsModified: false }],
    [["diff", "--name-only", "-z", "--merge-base", "main", "--"], { relative: false, ignore: undefined, listsModified: true }],
    [["diff", "--name-only", "-z", "--", "a.txt"], undefined],
    [["diff", "--name-only", "-z"], undefined],
    [["diff", "--name-status", "-z", "--"], undefined],
    [["diff", "--name-only", "-z", "--relative=src", "--"], undefined],
    [["diff", "--name-only", "-z", "HEAD~1", "HEAD", "--"], undefined],
    [["diff", "--name-only", "-z", "HEAD~1..HEAD", "--"], undefined],
    [["diff", "--name-only", "-z", "HEAD^!", "--"], undefined],
    [["diff", "--name-only", "-z", "--merge-base", "main", "HEAD", "--"], undefined],
  ] as const)("%j is %j", (args, expected) => {
    expect(nameDiff(args)).toEqual(expected);
  });

  it("tells a submodule's own entries from those of the submodules inside it", () => {
    const sha = "0".repeat(40);
    const output = [
      `1 .M SC.. 160000 160000 160000 ${sha} ${sha} inner one`,
      `1 M. S... 160000 160000 160000 ${sha} ${sha} staged`,
      `2 R. N... 100644 100644 100644 ${sha} ${sha} R100 new name`, "old name",
    ].join("\0");

    const read = readStatus(`${output}\0`);

    expect(read.own).toBe(true);
    expect([...read.submodules]).toEqual([["inner one", { staged: false, moved: true }], ["staged", { staged: true, moved: false }]]);
    expect(readStatus(`1 .M SC.. 160000 160000 160000 ${sha} ${sha} only\0`).own).toBe(false);
  });
});

describe("submoduleChanged", () => {
  it("fails, opening nothing, for a checked-out submodule past the depth Rerail follows", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-submodule-depth-"));
    roots.push(root);
    // A Git folder as Git's `is_git_directory` takes one: a `HEAD`, `objects` and `refs`.
    for (const name of ["objects", "refs"]) mkdirSync(join(root, "sub", ".git", name), { recursive: true });
    writeFileSync(join(root, "sub", ".git", "HEAD"), "ref: refs/heads/main\n");
    const opened: string[] = [];

    const changed = await submoduleChanged(join(root, "sub"), false, async folder => {
      opened.push(folder);
      return await Promise.resolve(UNASKED);
    }, 0);

    expect([typeof changed === "object" && changed.error?.message, opened]).toEqual([`Rerail follows submodules 16 levels deep, and ${join(root, "sub")} is deeper`, []]);
  });
});

describe("worktreeIndex", () => {
  it("leaves the index of a folder whose real path cannot be read to Git", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-worktree-index-"));
    roots.push(root);

    expect(worktreeIndex(join(root, "gone"), {})).toBeUndefined();
  });
});

describe("worktreeGitFolder", () => {
  it.each(["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"])("leaves the Git folder to Git where the environment sets %s", async name => {
    const { work } = repositories(await folder());

    expect(worktreeGitFolder(work, {})).toBeDefined();
    expect(worktreeGitFolder(work, { [name]: join(work, ".git") })).toBeUndefined();
  });
});

describe("a file of a Git folder whose close fails", () => {
  it.each<[string, (work: string, linked: string) => boolean]>([
    ["HEAD", work => definesNoDriver(join(work, ".git"))],
    ["config", work => definesNoDriver(join(work, ".git"))],
    ["commondir", work => definesNoDriver(join(work, ".git", "worktrees", "linked"))],
    [".git", (_work, linked) => worktreeGitFolder(linked, {}) !== undefined],
  ])("reads as one that cannot be read: %s", async (name, found) => {
    const { work, linked } = repositories(await folder());
    expect(found(work, linked)).toBe(true);

    failingCloses(name);

    expect(found(work, linked)).toBe(false);
  });

  it("closes a folder that stands at a file's place, a close that fails changing nothing", async () => {
    const { work } = repositories(await folder());
    mkdirSync(join(work, ".git", "config.worktree"));
    failingCloses("config.worktree");

    expect(definesNoDriver(join(work, ".git"))).toBe(false);
  });

  it("reads an index whose close fails as one that cannot be read, so that Git lists its submodules", async () => {
    const { work } = repositories(await folder());
    const index = join(work, ".git", "index");
    expect(await recordsNoSubmodule(index)).toBe(true);
    vi.mocked(open).mockImplementation(async (path, flags, mode) => {
      const handle = await realPromises.open(path, flags, mode);
      const close = handle.close.bind(handle);
      handle.close = async () => {
        await close();
        throw lostShare();
      };
      return handle;
    });

    expect(await recordsNoSubmodule(index)).toBe(false);
  });
});

async function folder(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-git-folder-"));
  roots.push(root);
  return root;
}

/** A repository at `root/work` with one commit, and a worktree of it at `root/linked`, made by the test's own Git. */
function repositories(root: string): { readonly work: string; readonly linked: string } {
  const work = join(root, "work");
  const linked = join(root, "linked");
  const git = (...args: string[]): void => {
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { stdio: "ignore" });
  };
  git("init", "--quiet", work);
  writeFileSync(join(work, "notes.txt"), "one\n");
  git("-C", work, "add", ".");
  git("-C", work, "commit", "--quiet", "-m", "seed");
  git("-C", work, "worktree", "add", "--quiet", "--detach", linked);
  return { work, linked };
}

/** Makes each close of a file named `name` fail once the file is closed, as a close on a network share that went fails. */
function failingCloses(name: string): void {
  const names = new Map<number, string>();
  vi.mocked(openSync).mockImplementation((path, flags, mode) => {
    const file = realFs.openSync(path, flags, mode);
    names.set(file, typeof path === "string" ? basename(path) : "");
    return file;
  });
  vi.mocked(closeSync).mockImplementation(file => {
    realFs.closeSync(file);
    if (names.get(file) === name) throw lostShare();
  });
}

function lostShare(): NodeJS.ErrnoException {
  return Object.assign(new Error("EIO: i/o error, close"), { code: "EIO", syscall: "close" });
}
