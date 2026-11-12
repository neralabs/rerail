import { execFileSync } from "node:child_process";
import type * as ChildProcess from "node:child_process";
import { copyFileSync, renameSync, rmSync } from "node:fs";
import type * as FileSystem from "node:fs";
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  gitBranchTips, gitCandidates, gitCommitsBetween, gitConfigFiles, gitExcludeFiles, gitHead, gitHeadReader, gitInstalled, gitUnmergedReader,
  headMovedOnlyByCommits, readGitBlobIds, readGitBlobs, runGit,
} from "../../src/engine/git.js";
import { honoursNoLazyFetch, partialCloneLacks } from "../support/partial-clone.js";

/**
 * The arguments of every Git process started without blocking, for the tests that count what a read costs, and work to
 * run once each just before the next `rev-parse --verify` starts: another process moving a ref while a read is in flight.
 */
const started = vi.hoisted(() => ({ args: [] as string[][], beforeVerify: [] as Array<() => void> }));
vi.mock("node:child_process", async importOriginal => {
  const actual = await importOriginal<typeof ChildProcess>();
  const execFile = (...call: Parameters<typeof actual.execFile>) => {
    if (Array.isArray(call[1])) {
      const args = [...(call[1] as readonly string[])];
      started.args.push(args);
      if (args.includes("--verify")) started.beforeVerify.shift()?.();
    }
    return (actual.execFile as (...forwarded: unknown[]) => ReturnType<typeof actual.execFile>)(...call);
  };
  return { ...actual, execFile };
});
/** How many more reads of each path fail as a file another program holds fails on Windows (`EBUSY`). */
const heldFiles = vi.hoisted(() => new Map<string, number>());
vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  const readFileSync = (...args: Parameters<typeof actual.readFileSync>) => {
    const path = String(args[0]);
    const held = heldFiles.get(path) ?? 0;
    if (held > 0) {
      heldFiles.set(path, held - 1);
      throw Object.assign(new Error(`EBUSY: resource busy or locked, open '${path}'`), { code: "EBUSY" });
    }
    return actual.readFileSync(...args);
  };
  return { ...actual, readFileSync };
});

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, encoding: "utf8" });
}

async function repository(files: Record<string, string>): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-git-blobs-"));
  roots.push(root);
  git(root, "init", "--quiet");
  for (const [path, text] of Object.entries(files)) await writeFile(join(root, path), text);
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

/** A `--filter=blob:none` clone of a one-file repository, and the id of the blob it does not hold. */
async function partialClone(): Promise<{ clone: string; oid: string }> {
  const origin = await repository({ "a.py": "print(1)\n" });
  git(origin, "config", "uploadpack.allowFilter", "true");
  const clone = join(origin, "..", `${origin.split("/").pop()}-clone`);
  roots.push(clone);
  git(origin, "clone", "--quiet", "--filter=blob:none", "--no-checkout", `file://${origin}`, clone);
  return { clone, oid: git(clone, "rev-parse", "HEAD:a.py").trim() };
}

describe("readGitBlobs", () => {
  it("keeps every blob it was asked for when another in the batch is larger than the limit", async () => {
    const root = await repository({ "big.py": "x".repeat(5_000), "small.py": "print(1)\n" });
    await writeFile(join(root, "small.py"), "print(2)\n");
    git(root, "add", "small.py");
    const blobs = await readGitBlobs(root, [":./big.py", ":./small.py", "HEAD:./small.py", ":./missing.py"], { maxBlobBytes: 1_000 });
    expect(Object.fromEntries([...blobs].map(([revision, blob]) => [revision, blob.toString("utf8")])))
      .toEqual({ ":./small.py": "print(2)\n", "HEAD:./small.py": "print(1)\n" });
  });

  it.skipIf(!honoursNoLazyFetch())("never fetches a blob a partial clone does not hold", async () => {
    const { clone, oid } = await partialClone();
    expect(await readGitBlobs(clone, ["HEAD:./a.py"], { maxBlobBytes: 1_000 })).toEqual(new Map());
    expect(partialCloneLacks(clone, oid)).toBe(true);
  });
});

describe("readGitBlobIds", () => {
  it("names each blob a revision or object id resolves to, and nothing for a missing one or a tree, whatever its path holds", async () => {
    const root = await repository({ "a b.py": "1\n", "x blob": "2\n" });
    await mkdir(join(root, "dir"));
    await writeFile(join(root, "dir", "c.py"), "3\n");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "more");
    const spaced = git(root, "rev-parse", "HEAD:a b.py").trim();
    const ids = await readGitBlobIds(root, [
      "HEAD:./a b.py", "HEAD:./x blob", "HEAD:./dir", "HEAD:./gone blob", spaced, "0".repeat(40), "HEAD:./dir/c.py",
    ]);
    expect(Object.fromEntries(ids)).toEqual({
      "HEAD:./a b.py": spaced,
      "HEAD:./x blob": git(root, "rev-parse", "HEAD:x blob").trim(),
      [spaced]: spaced,
      "HEAD:./dir/c.py": git(root, "rev-parse", "HEAD:dir/c.py").trim(),
    });
  });

  it("names nothing outside a repository", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-git-ids-"));
    roots.push(root);
    await expect(readGitBlobIds(root, ["HEAD:./a.py"])).resolves.toEqual(new Map());
  });
});

describe("gitBranchTips and gitCommitsBetween", () => {
  it("lists each commit a branch or remote-tracking branch points at once, and no tag", async () => {
    const root = await repository({ "a.py": "1\n" });
    const seed = (await gitHead(root))!;
    git(root, "branch", "same");
    git(root, "tag", "v1");
    git(root, "switch", "--quiet", "-c", "other");
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    const other = (await gitHead(root))!;
    git(root, "update-ref", "refs/remotes/origin/main", seed);
    git(root, "tag", "v2", other);
    git(root, "switch", "--quiet", "--detach", seed);
    git(root, "commit", "--quiet", "--allow-empty", "-m", "detached");
    expect((await gitBranchTips(root)).sort()).toEqual([seed, other].sort());
  });

  it("lists the commits a move added, and none where Git cannot say", async () => {
    const root = await repository({ "a.py": "1\n" });
    const seed = (await gitHead(root))!;
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    git(root, "commit", "--quiet", "--allow-empty", "-m", "three");
    const head = (await gitHead(root))!;
    expect(await gitCommitsBetween(root, seed, head)).toEqual([head, git(root, "rev-parse", "HEAD~1").trim()]);
    expect(await gitCommitsBetween(root, head, seed)).toEqual([]);
    expect(await gitCommitsBetween(root, "0".repeat(40), head)).toEqual([]);
  });
});

describe("runGit", () => {
  it.skipIf(!honoursNoLazyFetch())("never fetches an object a partial clone does not hold", async () => {
    const { clone, oid } = await partialClone();
    expect(runGit(clone, ["cat-file", "-p", oid]).status).not.toBe(0);
    expect(partialCloneLacks(clone, oid)).toBe(true);
  });
});

describe("gitHead and headMovedOnlyByCommits", () => {
  it("names no commit outside a repository, or where nothing is committed yet", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-git-head-"));
    roots.push(root);
    await expect(gitHead(root)).resolves.toBeUndefined();
    git(root, "init", "--quiet");
    await expect(gitHead(root)).resolves.toBeUndefined();
  });

  it("tells commits from anything else that moved HEAD since a commit", async () => {
    const root = await repository({ "a.py": "1\n" });
    const seed = (await gitHead(root))!;
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    git(root, "commit", "--quiet", "--allow-empty", "--amend", "-m", "two again");
    expect(await headMovedOnlyByCommits(root, seed)).toBe(true);
    const committed = (await gitHead(root))!;
    git(root, "reset", "--quiet", "--hard", seed);
    expect(await headMovedOnlyByCommits(root, committed)).toBe(false);
    // A commit after the move does not hide it.
    git(root, "commit", "--quiet", "--allow-empty", "-m", "three");
    expect(await headMovedOnlyByCommits(root, committed)).toBe(false);
  });

  it("goes by HEAD's newest move for a checkout not seen before, and takes a commit it cannot find as moved", async () => {
    const root = await repository({ "a.py": "1\n" });
    expect(await headMovedOnlyByCommits(root, undefined)).toBe(true);
    git(root, "switch", "--quiet", "-c", "other");
    expect(await headMovedOnlyByCommits(root, undefined)).toBe(false);
    expect(await headMovedOnlyByCommits(root, "0".repeat(40))).toBe(false);
  });
});

describe("gitExcludeFiles", () => {
  afterEach(() => { vi.unstubAllEnvs(); });

  /** A repository in a fresh home whose Git reads no global or system configuration. */
  async function isolatedRepository(): Promise<{ root: string; home: string }> {
    const home = await realpath(await mkdtemp(join(tmpdir(), "rerail-git-exclude-")));
    roots.push(home);
    vi.stubEnv("HOME", home);
    vi.stubEnv("XDG_CONFIG_HOME", "");
    vi.stubEnv("GIT_CONFIG_GLOBAL", join(home, "no-gitconfig"));
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    const root = join(home, "repo");
    await mkdir(root);
    git(root, "init", "--quiet");
    git(root, "commit", "--quiet", "--allow-empty", "-m", "seed");
    return { root, home };
  }

  it("names the checkout's info/exclude and Git's default excludes file while core.excludesFile is unset", async () => {
    const { root, home } = await isolatedRepository();
    expect(await gitExcludeFiles(root)).toEqual([join(root, ".git", "info", "exclude"), join(home, ".config", "git", "ignore")]);
    vi.stubEnv("XDG_CONFIG_HOME", join(home, "xdg"));
    expect((await gitExcludeFiles(root))[1]).toBe(join(home, "xdg", "git", "ignore"));
  });

  it("follows core.excludesFile, with `~` read as the home, and gives a linked worktree the common info/exclude", async () => {
    const { root, home } = await isolatedRepository();
    git(root, "config", "core.excludesFile", "~/ignores");
    git(root, "worktree", "add", "--quiet", join(home, "linked"));
    expect(await gitExcludeFiles(join(home, "linked"))).toEqual([join(root, ".git", "info", "exclude"), join(home, "ignores")]);
  });

  it("names none outside a Git checkout", async () => {
    const { home } = await isolatedRepository();
    expect(await gitExcludeFiles(home)).toEqual([]);
  });

  it("names the configuration files that can set core.excludesFile: the repository's, the worktree's and the global ones", async () => {
    const { root, home } = await isolatedRepository();
    vi.stubEnv("GIT_CONFIG_GLOBAL", "");
    expect(await gitConfigFiles(root)).toEqual([
      join(root, ".git", "config"), join(root, ".git", "config.worktree"), join(home, ".gitconfig"), join(home, ".config", "git", "config"),
    ]);
    vi.stubEnv("GIT_CONFIG_GLOBAL", join(home, "global"));
    expect((await gitConfigFiles(root)).slice(2)).toEqual([join(home, "global")]);
    expect(await gitConfigFiles(home)).toEqual([]);
  });
});

describe("gitUnmergedReader", () => {
  /** A reader whose every read must list the entries: a read Git could not answer fails the test. */
  function reader(): (cwd: string) => Promise<ReadonlyMap<string, string>> {
    const read = gitUnmergedReader();
    return async cwd => {
      const entries = await read(cwd);
      if (entries === undefined) throw new Error(`Git listed no unmerged entries in ${cwd}`);
      return entries;
    };
  }
  const gitUnmerged = async (cwd: string) => await reader()(cwd);
  /** A repository stopped on a conflict over `a.py`, on branch `first`, merging `second`. */
  async function conflicted(): Promise<string> {
    const root = await repository({ "a.py": "1\n" });
    git(root, "branch", "first");
    git(root, "branch", "second");
    for (const branch of ["second", "first"]) {
      git(root, "checkout", "--quiet", branch);
      await writeFile(join(root, "a.py"), `${branch}\n`);
      git(root, "commit", "--quiet", "-am", branch);
    }
    expect(() => git(root, "merge", "--quiet", "second")).toThrow();
    return root;
  }

  it("cannot say outside a repository, and has nothing where nothing is unmerged", async () => {
    const outside = await mkdtemp(join(tmpdir(), "rerail-git-unmerged-"));
    roots.push(outside);
    expect(await gitUnmergedReader()(outside)).toBeUndefined();
    expect(await gitUnmerged(await repository({ "a.py": "1\n" }))).toEqual(new Map());
  });

  it("names each unmerged path relative to the folder, and reads differently for a different conflict on it", async () => {
    const root = await repository({ "seed.py": "1\n" });
    await mkdir(join(root, "src"));
    await writeFile(join(root, "src", "a.py"), "1\n");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "source");
    git(root, "branch", "first");
    git(root, "branch", "second");
    const commitOn = async (branch: string, text: string) => {
      git(root, "checkout", "--quiet", branch);
      await writeFile(join(root, "src", "a.py"), text);
      git(root, "commit", "--quiet", "-am", branch);
    };
    await commitOn("first", "first\n");
    await commitOn("second", "second\n");
    git(root, "checkout", "--quiet", "first");
    expect(() => git(root, "merge", "--quiet", "second")).toThrow();
    const conflict = await gitUnmerged(join(root, "src"));
    expect([...conflict.keys()]).toEqual(["a.py"]);
    expect(await gitUnmerged(root)).toEqual(new Map([["src/a.py", conflict.get("a.py")]]));
    git(root, "merge", "--abort");
    await commitOn("first", "first again\n");
    expect(() => git(root, "merge", "--quiet", "second")).toThrow();
    expect((await gitUnmerged(root)).get("src/a.py")).not.toBe(conflict.get("a.py"));
  });

  it("lists more entries than fit in the megabyte of output a Git command gets by default", async () => {
    const root = await repository({ "a.py": "1\n" });
    const blob = git(root, "rev-parse", "HEAD:a.py").trim();
    const paths = Array.from({ length: 6_000 }, (_, at) => `src/${"conflicted-".repeat(5)}${at}.py`);
    const entries = paths.flatMap(path => [1, 2, 3].map(stage => `100644 ${blob} ${stage}\t${path}\n`)).join("");
    expect(Buffer.byteLength(entries)).toBeGreaterThan(1024 * 1024);
    execFileSync("git", ["update-index", "--index-info"], { cwd: root, input: entries });
    expect([...(await gitUnmerged(root)).keys()]).toEqual(paths.toSorted());
  });

  it("holds its read while the index file is unchanged, and reads again once Git has written it", async () => {
    const root = await conflicted();
    const read = reader();
    const first = await read(root);
    expect([...first.keys()]).toEqual(["a.py"]);
    expect(await read(root)).toBe(first);
    await writeFile(join(root, "a.py"), "edited\n");
    expect(await read(root)).toBe(first);
    git(root, "add", "a.py");
    expect(await read(root)).toEqual(new Map());
  });

  it("does not take its own reads, or Git's other reads, for a change to the index", async () => {
    const root = await conflicted();
    const read = reader();
    const first = await read(root);
    await gitHead(root);
    await readGitBlobs(root, [":2:./a.py", ":3:./a.py", "HEAD:./a.py"], { maxBlobBytes: 1024 });
    await headMovedOnlyByCommits(root, undefined);
    expect(await read(root)).toBe(first);
  });

  it("follows a linked worktree's own index, from a folder inside it too", async () => {
    const root = await repository({ "a.py": "1\n" });
    await mkdir(join(root, "src"));
    await writeFile(join(root, "src", "b.py"), "1\n");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "source");
    git(root, "branch", "first");
    git(root, "branch", "second");
    const worktree = `${root}-worktree`;
    roots.push(worktree);
    git(root, "worktree", "add", "--quiet", worktree, "first");
    const read = reader();
    const inside = join(worktree, "src");
    expect(await read(worktree)).toEqual(new Map());
    expect(await read(inside)).toEqual(new Map());
    git(root, "checkout", "--quiet", "second");
    await writeFile(join(root, "src", "b.py"), "second\n");
    git(root, "commit", "--quiet", "-am", "second");
    await writeFile(join(inside, "b.py"), "first\n");
    git(worktree, "commit", "--quiet", "-am", "first");
    expect(() => git(worktree, "merge", "--quiet", "second")).toThrow();
    expect([...(await read(worktree)).keys()]).toEqual(["src/b.py"]);
    expect([...(await read(inside)).keys()]).toEqual(["b.py"]);
    expect(await read(root)).toEqual(new Map());
  });

  it("reads every time, and still answers, once the index file is gone", async () => {
    const root = await conflicted();
    const read = reader();
    expect([...(await read(root)).keys()]).toEqual(["a.py"]);
    await rm(join(root, ".git", "index"));
    const gone = await read(root);
    expect(gone).toEqual(new Map());
    expect(await read(root)).not.toBe(gone);
  });

  it("cannot say while Git fails to read the index, and keeps nothing from that read", async () => {
    const root = await conflicted();
    const index = join(root, ".git", "index");
    const intact = await readFile(index);
    const read = gitUnmergedReader();
    const first = await read(root);
    expect([...first!.keys()]).toEqual(["a.py"]);
    await writeFile(index, "not an index");
    expect(await read(root)).toBeUndefined();
    expect(await read(root)).toBeUndefined();
    await writeFile(index, intact);
    const again = await read(root);
    expect(again).toEqual(first);
    expect(again).not.toBe(first);
  });
});

describe("gitHeadReader", () => {
  afterEach(() => {
    heldFiles.clear();
    started.beforeVerify.length = 0;
  });

  /** `HEAD` as `git rev-parse --verify --quiet HEAD` names it in `cwd`, or undefined where it names no commit. */
  function gitAnswer(cwd: string): string | undefined {
    try {
      return execFileSync("git", ["rev-parse", "--verify", "--quiet", "HEAD"], { cwd, encoding: "utf8" }).trim();
    } catch {
      return undefined;
    }
  }
  /** Reads `cwd` with `read`, checks that it answers as Git does, and gives how many Git processes the read started. */
  async function processes(read: (cwd: string) => Promise<string | undefined>, cwd: string): Promise<number> {
    started.args.length = 0;
    const answer = await read(cwd);
    const count = started.args.length;
    expect(answer).toBe(gitAnswer(cwd));
    return count;
  }
  /** Whether this Git makes a reftable repository: 2.45.0 on. */
  function makesReftable(): boolean {
    const match = /(\d+)\.(\d+)/u.exec(execFileSync("git", ["--version"], { encoding: "utf8" }));
    return match !== null && (Number(match[1]) > 2 || (Number(match[1]) === 2 && Number(match[2]) >= 45));
  }

  it("answers as Git does, and starts no Git process while the files Git reads HEAD from are unchanged", async () => {
    const root = await repository({ "a.py": "1\n" });
    await mkdir(join(root, "src"));
    const read = gitHeadReader();
    // The first read asks Git where the checkout keeps HEAD, then asks for HEAD.
    expect(await processes(read, root)).toBe(2);
    expect(await processes(read, root)).toBe(0);
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    expect(await processes(read, root)).toBe(1);
    expect(await processes(read, root)).toBe(0);
    // Another branch at the same commit, then a detached HEAD.
    git(root, "switch", "--quiet", "-c", "other");
    expect(await processes(read, root)).toBe(1);
    git(root, "switch", "--quiet", "--detach", "HEAD~");
    expect(await processes(read, root)).toBe(1);
    expect(await processes(read, root)).toBe(0);
    // A folder below the top asks Git every time: a `.git` made between it and the top would change the answer.
    expect(await processes(read, join(root, "src"))).toBe(1);
    expect(await processes(read, join(root, "src"))).toBe(1);
  });

  it("follows a branch another process moved, as a loose ref and once packed", async () => {
    const root = await repository({ "a.py": "1\n" });
    const branch = git(root, "symbolic-ref", "HEAD").trim();
    const seed = gitAnswer(root)!;
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    const two = gitAnswer(root)!;
    const read = gitHeadReader({ settleMs: 0 });
    await processes(read, root);
    git(root, "update-ref", branch, seed);
    expect(await processes(read, root)).toBe(1);
    // Packed: the loose ref is gone, and packed-refs holds the branch.
    git(root, "pack-refs", "--all");
    expect(await processes(read, root)).toBe(1);
    expect(await processes(read, root)).toBe(0);
    git(root, "update-ref", branch, two);
    expect(await processes(read, root)).toBe(1);
    git(root, "pack-refs", "--all");
    git(root, "commit", "--quiet", "--allow-empty", "-m", "three");
    git(root, "pack-refs", "--all");
    expect(await processes(read, root)).toBe(1);
    expect(await processes(read, root)).toBe(0);
  });

  it("asks Git each time it cannot read the branch's file, and keeps nothing from that read", async () => {
    const root = await repository({ "a.py": "1\n" });
    const branch = git(root, "symbolic-ref", "HEAD").trim();
    const seed = gitAnswer(root)!;
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    const two = gitAnswer(root)!;
    const read = gitHeadReader();
    await processes(read, root);
    // Another program holds the branch's file each time the reader looks, as Windows lets one while Git replaces it.
    heldFiles.set(join(root, ".git", ...branch.split("/")), 2);
    git(root, "update-ref", branch, seed);
    expect(await processes(read, root)).toBe(1);
    git(root, "update-ref", branch, two);
    expect(await processes(read, root)).toBe(1);
    // Neither held read kept an answer: the next read asks Git once more, and the one after it does not.
    expect(await processes(read, root)).toBe(1);
    expect(await processes(read, root)).toBe(0);
  });

  it("asks Git again while packed-refs is newer than the time it must stand unchanged", async () => {
    const root = await repository({ "a.py": "1\n" });
    git(root, "pack-refs", "--all");
    const read = gitHeadReader({ settleMs: 3_600_000 });
    expect(await processes(read, root)).toBe(2);
    expect(await processes(read, root)).toBe(1);
  });

  it("reads a linked worktree's own HEAD, and its branch moved from the main checkout", async () => {
    const root = await repository({ "a.py": "1\n" });
    git(root, "branch", "first");
    const worktree = `${root}-worktree`;
    roots.push(worktree);
    git(root, "worktree", "add", "--quiet", worktree, "first");
    const read = gitHeadReader();
    expect(await processes(read, worktree)).toBe(2);
    expect(await processes(read, root)).toBe(2);
    git(worktree, "commit", "--quiet", "--allow-empty", "-m", "in the worktree");
    expect(await processes(read, worktree)).toBe(1);
    expect(await processes(read, root)).toBe(0);
    git(root, "update-ref", "refs/heads/first", git(root, "rev-parse", "HEAD").trim());
    expect(await processes(read, worktree)).toBe(1);
    git(worktree, "switch", "--quiet", "--detach");
    expect(await processes(read, worktree)).toBe(1);
    expect(await processes(read, worktree)).toBe(0);
  });

  it("follows the Git folder of a worktree Git adds again at the same path", async () => {
    const root = await repository({ "a.py": "1\n" });
    for (const branch of ["one", "two", "three"]) git(root, "branch", branch);
    const parent = await mkdtemp(join(tmpdir(), "rerail-git-head-"));
    roots.push(parent);
    // Both folders are named `wt`, so each takes the Git folder `worktrees/wt` while it is free.
    const place = join(parent, "a", "wt");
    git(root, "worktree", "add", "--quiet", place, "one");
    const read = gitHeadReader();
    expect(await processes(read, place)).toBe(2);
    git(root, "worktree", "remove", place);
    git(root, "worktree", "add", "--quiet", join(parent, "b", "wt"), "two");
    git(root, "worktree", "add", "--quiet", place, "three");
    expect(await processes(read, place)).toBe(2);
    // The worktree's HEAD now lives in another Git folder than the one first read.
    git(place, "commit", "--quiet", "--allow-empty", "-m", "three moved");
    expect(await processes(read, place)).toBe(1);
  });

  it("names no commit outside a repository or before the first commit, and the first commit once made", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-git-head-"));
    roots.push(root);
    const read = gitHeadReader();
    expect(await read(root)).toBeUndefined();
    git(root, "init", "--quiet");
    expect(await read(root)).toBeUndefined();
    expect(await read(root)).toBeUndefined();
    git(root, "commit", "--quiet", "--allow-empty", "-m", "first");
    expect(await processes(read, root)).toBe(1);
    expect(await processes(read, root)).toBe(0);
  });

  it("asks Git every time for a branch that names another ref", async () => {
    const root = await repository({ "a.py": "1\n" });
    const branch = git(root, "symbolic-ref", "HEAD").trim();
    git(root, "symbolic-ref", "refs/heads/alias", branch);
    git(root, "symbolic-ref", "HEAD", "refs/heads/alias");
    const read = gitHeadReader();
    await processes(read, root);
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    expect(await processes(read, root)).toBe(1);
    expect(await processes(read, root)).toBe(1);
  });

  it.skipIf(!makesReftable())("asks Git every time in a reftable repository", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-git-head-"));
    roots.push(root);
    git(root, "init", "--quiet", "--ref-format=reftable");
    git(root, "commit", "--quiet", "--allow-empty", "-m", "first");
    const read = gitHeadReader();
    expect(await processes(read, root)).toBe(2);
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    expect(await processes(read, root)).toBe(1);
    expect(await processes(read, root)).toBe(1);
  });

  /** A repository on its branch at `one`, with the commit `two` after it. */
  async function twoCommits(): Promise<{ root: string; branch: string; one: string; two: string }> {
    const root = await repository({ "a.py": "1\n" });
    const branch = git(root, "symbolic-ref", "HEAD").trim();
    const one = gitAnswer(root)!;
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    const two = gitAnswer(root)!;
    git(root, "update-ref", branch, one);
    return { root, branch, one, two };
  }
  /** A folder to read, and how another process moves its `HEAD` to the commit `one` and to the commit `two`. */
  interface Moves {
    readonly cwd: string;
    readonly toOne: () => void;
    readonly toTwo: () => void;
    readonly settleMs?: number;
  }
  const movedWhileGitReads: Array<[string, () => Promise<Moves>]> = [
    ["its branch's loose ref", async () => {
      const { root, branch, one, two } = await twoCommits();
      return { cwd: root, toOne: () => git(root, "update-ref", branch, one), toTwo: () => git(root, "update-ref", branch, two) };
    }],
    ["HEAD to another branch", async () => {
      const { root, branch, two } = await twoCommits();
      git(root, "branch", "other", two);
      return { cwd: root, toOne: () => git(root, "symbolic-ref", "HEAD", branch), toTwo: () => git(root, "symbolic-ref", "HEAD", "refs/heads/other") };
    }],
    ["HEAD to another branch, both packed", async () => {
      const { root, branch, two } = await twoCommits();
      git(root, "branch", "other", two);
      git(root, "pack-refs", "--all");
      return {
        cwd: root, settleMs: 0, toOne: () => git(root, "symbolic-ref", "HEAD", branch), toTwo: () => git(root, "symbolic-ref", "HEAD", "refs/heads/other"),
      };
    }],
    ["a detached HEAD", async () => {
      const { root, one, two } = await twoCommits();
      git(root, "update-ref", "--no-deref", "HEAD", one);
      return { cwd: root, toOne: () => git(root, "update-ref", "--no-deref", "HEAD", one), toTwo: () => git(root, "update-ref", "--no-deref", "HEAD", two) };
    }],
    ["a linked worktree's branch, from the main checkout", async () => {
      const { root, one, two } = await twoCommits();
      git(root, "branch", "first", one);
      const worktree = `${root}-worktree`;
      roots.push(worktree);
      git(root, "worktree", "add", "--quiet", worktree, "first");
      return { cwd: worktree, toOne: () => git(root, "update-ref", "refs/heads/first", one), toTwo: () => git(root, "update-ref", "refs/heads/first", two) };
    }],
  ];

  it.each(movedWhileGitReads)("keeps no answer the files it read do not name: %s moved back while Git read it, then again", async (_name, setUp) => {
    const { cwd, toOne, toTwo, settleMs } = await setUp();
    const read = gitHeadReader(settleMs === undefined ? {} : { settleMs });
    await processes(read, cwd);
    // The read finds the files at `two`, then another process moves HEAD back to `one` before Git reads it, so Git names
    // `one`. Once the files say `two` again, the read must name `two`.
    toTwo();
    started.beforeVerify.push(toOne);
    await processes(read, cwd);
    toTwo();
    await processes(read, cwd);
  });

  it.each(["objects", "refs"])("names no commit, as Git, once the Git folder's %s folder is gone", async folder => {
    const root = await repository({ "a.py": "1\n" });
    // Packed, so the branch's loose ref is not what goes with `refs`.
    git(root, "pack-refs", "--all");
    const read = gitHeadReader({ settleMs: 0 });
    await processes(read, root);
    expect(await processes(read, root)).toBe(0);
    await rm(join(root, ".git", folder), { recursive: true, force: true });
    await processes(read, root);
  });

  it("names no commit, as Git, once the repository's format names an extension this Git does not know", async () => {
    const root = await repository({ "a.py": "1\n" });
    const read = gitHeadReader();
    await processes(read, root);
    expect(await processes(read, root)).toBe(0);
    git(root, "config", "core.repositoryformatversion", "1");
    git(root, "config", "extensions.rerailUnknownExtension", "true");
    await processes(read, root);
  });

  it("starts no Git process after a write to config that leaves the repository's format as it was", async () => {
    const root = await repository({ "a.py": "1\n" });
    const read = gitHeadReader();
    await processes(read, root);
    git(root, "config", "branch.topic.remote", "origin");
    expect(await processes(read, root)).toBe(0);
  });

  it("holds no file Git replaces open across a turn of the event loop", async () => {
    // Git replaces HEAD, a branch's ref and config by renaming its lock file over them, which Windows refuses while
    // another process has the file open: a read that held one across the turns of a busy event loop failed an agent's
    // `git commit` and `git switch`.
    const root = await repository({ "a.py": "1\n" });
    const branch = git(root, "symbolic-ref", "HEAD").trim();
    const files = [join(root, ".git", "HEAD"), join(root, ".git", ...branch.split("/")), join(root, ".git", "config")];
    const read = gitHeadReader();
    await read(root);
    const refused: string[] = [];
    for (let round = 0; round < 10 && refused.length === 0; round += 1) {
      let reading = true;
      const done = read(root).finally(() => { reading = false; });
      while (reading) {
        refused.push(...files.filter(file => !replacedByRename(file)));
        await new Promise(resolve => setImmediate(resolve));
      }
      await done;
    }
    expect(refused).toEqual([]);
  }, 30_000);
});

/**
 * Whether a lock file renamed over `file`, as Git replaces it, succeeds within 50 ms of tries that never yield: another
 * program's brief hold (a virus scanner's) passes, and a hold of this process's own event loop does not.
 */
function replacedByRename(file: string): boolean {
  const lock = `${file}.lock`;
  copyFileSync(file, lock);
  const until = performance.now() + 50;
  for (;;) {
    try {
      renameSync(lock, file);
      return true;
    } catch {
      if (performance.now() >= until) {
        rmSync(lock, { force: true });
        return false;
      }
    }
  }
}

describe("gitCandidates", () => {
  const disk = (files: readonly string[], links: Record<string, string> = {}) => ({
    exists: (path: string) => files.includes(path),
    realpath: (path: string) => Object.entries(links).reduce((real, [link, target]) => real.startsWith(link) ? target + real.slice(link.length) : real, path),
  });

  it("tries PATH first, in its order, then the usual install locations, each once", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: "/home/me/.local/share/mise/shims:/opt/local/bin:/usr/bin", cwd: "/tmp" };
    expect(gitCandidates(search, disk([]))).toEqual([
      "/home/me/.local/share/mise/shims/git",
      "/opt/local/bin/git",
      "/usr/bin/git",
      "/usr/local/bin/git",
      "/opt/homebrew/bin/git",
      "/home/linuxbrew/.linuxbrew/bin/git",
      "/run/current-system/sw/bin/git",
      "/home/me/.nix-profile/bin/git",
    ]);
  });

  it("skips PATH entries a repository could control: relative, inside the repository it runs in, or in node_modules", () => {
    const search = {
      platform: "darwin" as const,
      home: "/Users/me",
      // Empty and `.` mean the current folder; bin is direnv's PATH_add from the repository's .envrc.
      path: ":.:bin:/Users/me/code/app/bin:/Users/me/code/app/node_modules/.bin:/Users/me/code/node_modules/.bin:/Users/me/code/app-tools/bin:/opt/homebrew/bin",
      cwd: "/Users/me/code/app/src",
    };
    expect(gitCandidates(search, disk(["/Users/me/code/app/.git"])).slice(0, 2)).toEqual([
      "/Users/me/code/app-tools/bin/git",
      "/opt/homebrew/bin/git",
    ]);
  });

  it("skips an entry that reaches the repository through a symlink, either way round", () => {
    // macOS's /var is /private/var: the folder Rerail runs in reads as /private/var, PATH as /var.
    const search = { platform: "darwin" as const, home: "/Users/me", path: "/var/t/app/bin:/Users/me/tools:/usr/bin", cwd: "/private/var/t/app" };
    const links = { "/var/": "/private/var/", "/Users/me/tools": "/private/var/t/app/bin" };
    expect(gitCandidates(search, disk(["/private/var/t/app/.git"], links))[0]).toBe("/usr/bin/git");
  });

  it("on Windows takes git.exe from absolute entries only, unquoted, with the same skips in any case", () => {
    // A repository's environment (direnv, a devcontainer) can set LOCALAPPDATA as easily as PATH.
    vi.stubEnv("LOCALAPPDATA", String.raw`C:\repo\evil`);
    try {
      const search = {
        platform: "win32" as const,
        home: String.raw`C:\Users\me`,
        path: String.raw`"C:\Program Files\Git\cmd";.;\tools;C:\Code\App\Bin;C:\code\web\NODE_MODULES\.bin;D:\scoop\shims`,
        cwd: String.raw`c:\code\app`,
      };
      expect(gitCandidates(search, disk([String.raw`c:\code\app\.git`]))).toEqual([
        String.raw`C:\Program Files\Git\cmd\git.exe`,
        String.raw`D:\scoop\shims\git.exe`,
        String.raw`C:\Program Files (x86)\Git\cmd\git.exe`,
        String.raw`C:\Users\me\AppData\Local\Programs\Git\cmd\git.exe`,
        String.raw`C:\Users\me\scoop\apps\git\current\cmd\git.exe`,
      ]);
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("keeps every absolute entry outside node_modules when the folder it runs in is in no repository", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: "/home/me/bin", cwd: "/home/me" };
    expect(gitCandidates(search, disk([]))[0]).toBe("/home/me/bin/git");
  });

  it("still lists PATH and the install locations from a working folder that was deleted", async () => {
    const gone = await mkdtemp(join(tmpdir(), "rerail-git-gone-"));
    const before = process.cwd();
    process.chdir(gone);
    try {
      await rm(gone, { recursive: true });
      expect(() => process.cwd()).toThrow();
      vi.stubEnv("PATH", "/opt/local/bin");
      expect(gitCandidates()).toContain("/opt/local/bin/git");
    } finally {
      vi.unstubAllEnvs();
      process.chdir(before);
    }
  });
});

describe("gitInstalled", () => {
  const probe = (files: readonly string[], developerDir?: string) => ({
    exists: (path: string) => files.includes(path),
    runnable: (path: string) => files.includes(path),
    developerDir: () => developerDir,
  });

  it("takes macOS's /usr/bin/git, always present as a stub, only with Apple's developer tools that carry Git", () => {
    // No Command Line Tools and no Xcode: the stub would open the installer, so Rerail says Git is missing.
    expect(gitInstalled("/usr/bin/git", "darwin", probe(["/usr/bin/git"]))).toBe(false);
    expect(gitInstalled("/usr/bin/git", "darwin", probe(["/usr/bin/git", "/Library/Developer/CommandLineTools/usr/bin/git"]))).toBe(true);
    const xcode = "/Applications/Xcode.app/Contents/Developer";
    expect(gitInstalled("/usr/bin/git", "darwin", probe(["/usr/bin/git", `${xcode}/usr/bin/git`], xcode))).toBe(true);
    expect(gitInstalled("/usr/bin/git", "darwin", probe(["/usr/bin/git"], xcode))).toBe(false);
  });

  it("takes any other Git that is there, Homebrew's on a Mac included", () => {
    expect(gitInstalled("/opt/homebrew/bin/git", "darwin", probe(["/opt/homebrew/bin/git"]))).toBe(true);
    expect(gitInstalled("/usr/bin/git", "linux", probe(["/usr/bin/git"]))).toBe(true);
    expect(gitInstalled("/usr/local/bin/git", "linux", probe([]))).toBe(false);
  });

  it.skipIf(process.platform === "win32")("passes over a folder named git and a git without its execute bit, as a shell does", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-git-runnable-"));
    roots.push(root);
    await mkdir(join(root, "folder", "git"), { recursive: true });
    await mkdir(join(root, "plain"));
    await writeFile(join(root, "plain", "git"), "#!/bin/sh\n");
    await mkdir(join(root, "runs"));
    await writeFile(join(root, "runs", "git"), "#!/bin/sh\n");
    await chmod(join(root, "runs", "git"), 0o755);

    expect(gitInstalled(join(root, "folder", "git"), "linux")).toBe(false);
    expect(gitInstalled(join(root, "plain", "git"), "linux")).toBe(false);
    expect(gitInstalled(join(root, "runs", "git"), "linux")).toBe(true);
  });
});
