import { execFileSync } from "node:child_process";
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  gitBranchTips, gitCandidates, gitCommitsBetween, gitConfigFiles, gitExcludeFiles, gitHead, gitInstalled, gitModifiedOrUntrackedPaths, gitUnmergedReader,
  headMovedOnlyByCommits, readGitBlobIds, readGitBlobs, runGit,
} from "../../src/engine/git.js";
import { honoursNoLazyFetch, partialCloneLacks } from "../support/partial-clone.js";

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

describe("Git reads and their time limit", () => {
  it("answers with what Git printed when the event loop stalls past the time limit after Git finished", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    await writeFile(join(root, "a.py"), "print(2)\n");
    const head = git(root, "rev-parse", "HEAD").trim();
    const reads = Promise.all([gitModifiedOrUntrackedPaths(root), gitHead(root)]);
    // Both Gits finish during the stall, and the event loop handles their exits only once it ends, past the 10 s limit.
    setImmediate(() => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10_500));
    await expect(reads).resolves.toEqual([["a.py"], head]);
  }, 30_000);

  it("ends a read at the time limit when Git has exited but a process it started keeps the output open", async () => {
    const root = await repository({ ".gitattributes": "a.py filter=held\n", "a.py": "print(1)\n" });
    const released = join(root, ".git", "released");
    // Git runs the clean filter to compare a.py, changed on disk, with the index. The loop the filter leaves behind holds
    // Git's stderr, outside the repository, until the test creates `released`.
    git(root, "config", "filter.held.clean", `(cd / && while [ ! -e '${released.replaceAll("\\", "/")}' ]; do sleep 0.2; done) </dev/null >/dev/null & cat`);
    await writeFile(join(root, "a.py"), "print(2)\n");
    let late: NodeJS.Timeout | undefined;
    try {
      const read = gitModifiedOrUntrackedPaths(root);
      const waited = new Promise(settle => { late = setTimeout(settle, 20_000, "still waiting at 20 s"); });
      await expect(Promise.race([read, waited])).resolves.toBeUndefined();
    } finally {
      clearTimeout(late);
      await writeFile(released, "");
    }
  }, 30_000);
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
