import { execFile, execFileSync, spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { basename, delimiter, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });
vi.mock("../../src/engine/child-process.js", { spy: true });

import { endProcessTree } from "../../src/engine/child-process.js";
import {
  findGit, gitBehindLauncher, gitBranchTips, gitCandidates, gitCommitsBetween, gitConfigFiles, gitCrlfPaths, gitExcludeFiles, gitExecutable, gitHead, gitHome,
  gitInstalled, gitModifiedOrUntrackedListing, gitModifiedOrUntrackedPaths, GitReads, GitUnavailableError, gitUnmergedReader, headMovedOnlyByCommits, readGitBlobIds,
  readGitBlobs, runGit, searchGitFrom,
} from "../../src/engine/git.js";
import { processState } from "../../src/engine/process-liveness.js";
import { honoursNoLazyFetch, partialCloneLacks } from "../support/partial-clone.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const actualChildProcess = await vi.importActual<typeof import("node:child_process")>("node:child_process");

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => removeFolder(root))); });

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
  const clone = join(origin, "..", `${basename(origin)}-clone`);
  roots.push(clone);
  git(origin, "clone", "--quiet", "--filter=blob:none", "--no-checkout", pathToFileURL(origin).href, clone);
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

describe("GitReads", () => {
  it("answers a read it stopped as without Git, and starts no Git once stopped", async () => {
    const root = await repository({ "a.py": "1\n" });
    const reads = new GitReads();
    const running = reads.run(signal => gitHead(root, { signal }));
    await reads.stop();

    await expect(running).resolves.toBeUndefined();
    await expect(reads.run(signal => gitHead(root, { signal }))).resolves.toBeUndefined();
    await expect(reads.run(signal => readGitBlobs(root, ["HEAD:./a.py"], { maxBlobBytes: 1_000, signal }))).resolves.toEqual(new Map());
    await expect(new GitReads().run(signal => gitHead(root, { signal }))).resolves.toMatch(/^[0-9a-f]{40}/u);
    await writeFile(join(root, "b.py"), "untracked\n");
    // A stopped listing names no file changed, and says it cannot tell rather than none.
    await expect(reads.run(signal => gitModifiedOrUntrackedPaths(root, { signal }))).resolves.toBeUndefined();
    await expect(new GitReads().run(signal => gitModifiedOrUntrackedPaths(root, { signal }))).resolves.toEqual(["b.py"]);
  });

  /**
   * Has the next Git a read streams from print `output`, then stay running with its output open until it is ended, as a
   * Git still at work does. Resolves once that output has reached the read.
   */
  function streamingGitThatStays(output: string): Promise<void> {
    const printed = Promise.withResolvers<void>();
    vi.mocked(spawn).mockImplementationOnce((_git, args, options) => {
      const script = `process.stdout.write(${JSON.stringify(output)}); setTimeout(() => {}, 30_000)`;
      const child = actualChildProcess.spawn(process.execPath, ["-e", script, "--", ...args], options);
      // Ahead of the read's own listener, which has the chunk by the time this is awaited.
      child.stdout?.once("data", () => printed.resolve());
      return child;
    });
    return printed.promise;
  }

  it("stops a read still streaming from its Git at once, with the blobs it had read", async () => {
    const root = await repository({ "a.py": "1\n" });
    const printed = streamingGitThatStays(`${"a".repeat(40)} blob 2\n1\n\n`);
    const reads = new GitReads();
    // A timeout this long never ends the read here: only the stop does.
    const read = reads.run(signal => readGitBlobs(root, ["HEAD:./a.py", "HEAD:./b.py"], { maxBlobBytes: 1_000, timeout: 60_000, signal }));
    await printed;

    const stopped = reads.stop().then(() => "stopped");
    await expect(Promise.race([stopped, delay(3_000, "still running", { ref: false })])).resolves.toBe("stopped");
    await expect(read).resolves.toEqual(new Map([["HEAD:./a.py", Buffer.from("1\n")]]));
  });

  /**
   * A launcher, run with the file to write a pid to: it runs the real Git as its own child holding the output it was
   * given, writes that child's pid, and waits for it, as Git for Windows' `cmd\git.exe` waits for the git.exe it starts.
   * `detached` keeps the child out of the job object Node puts its children in, which would end it with the launcher;
   * a launcher without one leaves its child running when it is killed.
   */
  const LAUNCHER = [
    "const git = require('node:child_process').spawn(process.execPath, ['-e', 'setTimeout(() => {}, 30_000)'], { stdio: 'inherit', windowsHide: true, detached: true });",
    "require('node:fs').writeFileSync(process.argv[1], String(git.pid));",
    "setTimeout(() => {}, 30_000);",
  ].join(" ");

  /** The pid in `file`, once a launcher has written it there. */
  async function launchedGit(file: string): Promise<number> {
    return await vi.waitFor(async () => {
      const pid = Number(await readFile(file, "utf8"));
      if (!(pid > 0)) throw new Error(`${file} holds no pid yet`);
      return pid;
    }, { timeout: 10_000, interval: 20 });
  }

  it.runIf(process.platform === "win32")("ends the Git behind a launcher with the launcher, and answers once both have exited", async () => {
    const root = await repository({ "a.py": "1\n" });
    const pids = await mkdtemp(join(tmpdir(), "rerail-git-launched-"));
    roots.push(pids);
    const [headPid, blobsPid] = [join(pids, "head"), join(pids, "blobs")];
    vi.mocked(execFile).mockImplementationOnce((_git, args, options, callback) =>
      actualChildProcess.execFile(process.execPath, ["-e", LAUNCHER, "--", headPid, ...(args ?? [])], options, callback));
    vi.mocked(spawn).mockImplementationOnce((_git, args, options) =>
      actualChildProcess.spawn(process.execPath, ["-e", LAUNCHER, "--", blobsPid, ...args], options));
    const reads = new GitReads();
    const answers = Promise.all([
      reads.run(signal => gitHead(root, { signal })),
      reads.run(signal => readGitBlobs(root, ["HEAD:./a.py"], { maxBlobBytes: 1_000, timeout: 60_000, signal })),
    ]);
    const gits = await Promise.all([launchedGit(headPid), launchedGit(blobsPid)]);
    await reads.stop();

    // Each read settles only once its whole tree has exited, so neither Git is waited for here.
    const running = gits.filter(pid => processState(pid) !== "gone");
    for (const pid of running) process.kill(pid);
    expect(running).toEqual([]);
    await expect(answers).resolves.toEqual([undefined, new Map()]);
  });
});

describe("gitModifiedOrUntrackedListing", () => {
  it("says that a listing Git failed failed, where gitModifiedOrUntrackedPaths answers none", async () => {
    const root = await repository({ "a.py": "1\n" });
    await writeFile(join(root, "b.py"), "untracked\n");
    await expect(gitModifiedOrUntrackedListing(root)).resolves.toEqual({ ended: "printed", paths: ["b.py"] });
    // Git cannot start in a folder that is not there.
    const gone = join(root, "gone");
    await expect(gitModifiedOrUntrackedListing(gone)).resolves.toEqual({ ended: "failed" });
    await expect(gitModifiedOrUntrackedPaths(gone)).resolves.toEqual([]);
  });
});

/**
 * What the clean filter of `heldListing` does while `held` is there: leave a process behind that holds Git's stderr after
 * Git exits, or keep Git waiting for it, so Git never finishes.
 */
type HeldFilter = "leaves-a-process" | "keeps-git-waiting";

/**
 * Lists the changed files of a repository whose clean filter waits, outside the repository, while a file is there (as
 * `filter` says), and answers what the read answered within `waitMs`, or that it was still waiting. Git runs the filter
 * to compare a.py, changed on disk, with the index. `whileReading` runs once the read has started. The wait ends once
 * the listing is answered, and so does removing the repository.
 */
async function heldListing(filter: HeldFilter, waitMs: number, whileReading: () => void = () => undefined): Promise<unknown> {
  const root = await repository({ ".gitattributes": "a.py filter=held\n", "a.py": "print(1)\n" });
  const held = join(root, ".git", "held");
  const loop = `while [ -e '${held.replaceAll("\\", "/")}' ]; do sleep 0.2; done`;
  const clean = filter === "leaves-a-process" ? `(cd / && ${loop}) </dev/null >/dev/null & cat` : `cd / && (${loop}) </dev/null >/dev/null; cat`;
  git(root, "config", "filter.held.clean", clean);
  await writeFile(held, "");
  await writeFile(join(root, "a.py"), "print(2)\n");
  let late: NodeJS.Timeout | undefined;
  try {
    const read = gitModifiedOrUntrackedPaths(root);
    whileReading();
    return await Promise.race([read, new Promise(settle => { late = setTimeout(settle, waitMs, `still waiting at ${waitMs / 1000} s`); })]);
  } finally {
    clearTimeout(late);
    await rm(held, { force: true });
  }
}

/** From now on `performance.now()` answers `ms` later than it would: the clock a read finds after the event loop stalled. */
function clockAhead(ms: number): void {
  const now = performance.now.bind(performance);
  vi.spyOn(performance, "now").mockImplementation(() => now() + ms);
}

/** Blocks the event loop for `ms` once the current pass of it reaches its immediates. */
function stallSoon(ms: number): void {
  setImmediate(() => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms));
}

describe("Git reads and their time limit", () => {
  afterEach(() => { vi.restoreAllMocks(); });

  it("answers with what Git printed when the event loop stalls past the time limit after Git finished", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    await writeFile(join(root, "a.py"), "print(2)\n");
    const head = git(root, "rev-parse", "HEAD").trim();
    vi.mocked(endProcessTree).mockClear();
    const reads = Promise.all([gitModifiedOrUntrackedPaths(root), gitHead(root)]);
    // Both Gits finish during the stall, and the event loop handles their exits only once it ends, past the 10 s limit.
    stallSoon(10_500);
    await expect(reads).resolves.toEqual([["a.py"], head]);
    // Neither read was ended: on Windows that ran `taskkill /T /F` on the pid of a Git already gone.
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("answers once Git has exited and its output has ended, while a process it started holds its stderr", async () => {
    await expect(heldListing("leaves-a-process", 5_000)).resolves.toEqual(["a.py"]);
  }, 30_000);

  it("ends a read whose Git prints nothing at its first look past the time limit on the clock, after a stall", async () => {
    // As if the event loop had stalled for 11 s as the read started: its first look, 1 s in by its timer, is 12 s in on the clock.
    await expect(heldListing("keeps-git-waiting", 5_000, () => clockAhead(11_000))).resolves.toBeUndefined();
  }, 30_000);

  it("answers with what Git printed when the stall that took the read past the time limit on the clock ended after Git finished", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    const head = git(root, "rev-parse", "HEAD").trim();
    vi.mocked(endProcessTree).mockClear();
    const read = gitHead(root);
    clockAhead(11_000);
    // Git finishes during the stall. The read's tick runs first after it, before the event loop handles Git's exit.
    stallSoon(2_000);
    await expect(read).resolves.toBe(head);
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("ends a read whose Git still prints at its first look past the time limit on the clock, and answers that it ran late", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    const printing = Promise.withResolvers<void>();
    // In Git's place, a program that prints a path every 20 ms until it is ended, as a Git listing a large checkout does.
    vi.mocked(execFile).mockImplementationOnce((_git, args, options, callback) => {
      const script = "setInterval(() => process.stdout.write('a.py\\0'), 20)";
      const child = actualChildProcess.execFile(process.execPath, ["-e", script, "--", ...(args ?? [])], options, callback);
      child.stdout?.once("data", () => printing.resolve());
      return child;
    });
    const read = gitModifiedOrUntrackedPaths(root);
    await printing.promise;
    clockAhead(10_000);
    // The read's next look finds it past the limit on the clock, with output still coming: it ends there, as a timer of
    // the time limit ended it, and a walk asks the listing once more off its path.
    await expect(read).resolves.toBeUndefined();
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

  it("finds the global files in Git's own home, which on Windows without HOME is HOMEDRIVE and HOMEPATH", async () => {
    const { root, home } = await isolatedRepository();
    vi.stubEnv("GIT_CONFIG_GLOBAL", "");
    const share = join(home, "share");
    await mkdir(share);
    const globals = [join(share, ".gitconfig"), join(share, ".config", "git", "config")];
    // Git's home, not the account's: HOME here names another folder than the one homedir() answers.
    expect((await gitConfigFiles(root, { env: { ...process.env, HOME: share } })).slice(2)).toEqual(globals);
    if (process.platform === "win32") {
      const { HOME: _home, ...withoutHome } = process.env;
      const drive = share.slice(0, 2);
      expect((await gitConfigFiles(root, { env: { ...withoutHome, HOMEDRIVE: drive, HOMEPATH: share.slice(2) }, platform: "win32" })).slice(2)).toEqual(globals);
    }
  });
});

describe("gitBehindLauncher", () => {
  it("runs Git for Windows' own git.exe, not the launcher that starts it as a child a timeout would leave running", () => {
    const git = String.raw`C:\Program Files\Git`;
    const launcher = String.raw`${git}\cmd\git.exe`;
    const search = { platform: "win32" as const, home: String.raw`C:\Users\me`, path: undefined, cwd: undefined };
    const installed = (...paths: string[]) => {
      const files = [launcher, String.raw`${git}\etc\gitconfig`, ...paths];
      return { exists: (path: string) => files.includes(path), runnable: (path: string) => files.includes(path), realpath: (path: string) => path, developerDir: () => undefined };
    };
    const real = String.raw`${git}\mingw64\bin\git.exe`;
    expect(gitBehindLauncher(launcher, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(String.raw`${git}\bin\git.exe`, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(launcher, search, installed(String.raw`${git}\clangarm64\bin\git.exe`))).toBe(String.raw`${git}\clangarm64\bin\git.exe`);
    // The real one is itself, and a Git laid out otherwise (a scoop shim, a launcher without its Git) is run as found.
    expect(gitBehindLauncher(real, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(String.raw`D:\scoop\shims\git.exe`, search, installed(real))).toBe(String.raw`D:\scoop\shims\git.exe`);
    expect(gitBehindLauncher(launcher, search, installed())).toBe(launcher);
  });

  it.runIf(process.platform === "win32")("is what Rerail runs on this Windows machine", () => {
    expect(gitExecutable()).toBe(gitBehindLauncher(gitExecutable()));
    expect(gitExecutable().toLowerCase()).not.toMatch(/\\cmd\\git\.exe$/u);
  });
});

describe("gitCrlfPaths", () => {
  afterEach(() => { vi.unstubAllEnvs(); });

  it("names the files a checkout writes with CRLF line breaks and the rule of each, as Git's attributes and configuration decide it", async () => {
    const root = await repository({ "a.py": "a\n", "b.bin": "b\n", "c.txt": "c\n", "d.md": "d\n", "e.ts": "e\n" });
    git(root, "config", "core.autocrlf", "false");
    git(root, "config", "core.eol", "native");
    await writeFile(join(root, ".git", "info", "attributes"), "*.py text\n*.bin -text eol=crlf\n*.txt eol=crlf\n*.ts text=auto\n");
    const paths = ["a.py", "b.bin", "c.txt", "d.md", "e.ts"];
    // A text file takes core.eol, whose default (native) is CRLF on Windows alone; -text is never converted.
    expect(Object.fromEntries(await gitCrlfPaths(root, paths, { platform: "win32" }))).toEqual({ "a.py": "text", "c.txt": "text", "e.ts": "auto" });
    expect(Object.fromEntries(await gitCrlfPaths(root, paths, { platform: "linux" }))).toEqual({ "c.txt": "text" });
    // core.autocrlf=true converts every file with no text attribute too, by Git's guess; input converts none on checkout.
    git(root, "config", "core.autocrlf", "true");
    expect(Object.fromEntries(await gitCrlfPaths(root, paths, { platform: "linux" }))).toEqual({ "a.py": "text", "c.txt": "text", "d.md": "auto", "e.ts": "auto" });
    git(root, "config", "core.autocrlf", "input");
    expect(Object.fromEntries(await gitCrlfPaths(root, paths, { platform: "win32" }))).toEqual({ "c.txt": "text" });
  });

  it("reads core.autocrlf from the person's global settings, which tests otherwise keep from every repository", async () => {
    const root = await repository({ "a.py": "a\n" });
    const global = join(root, ".git", "person.gitconfig");
    await writeFile(global, "[core]\n\tautocrlf = true\n");
    vi.stubEnv("GIT_CONFIG_GLOBAL", global);
    expect(Object.fromEntries(await gitCrlfPaths(root, ["a.py"], { platform: "linux" }))).toEqual({ "a.py": "auto" });
  });
});

describe("gitHome", () => {
  it("on Windows without HOME takes HOMEDRIVE and HOMEPATH when that folder exists, as Git for Windows does", () => {
    const share = { HOMEDRIVE: "H:", HOMEPATH: "\\" };
    expect(gitHome(share, "win32", path => path === "H:\\")).toBe("H:\\");
    expect(gitHome(share, "win32", () => false)).toBe(homedir());
    expect(gitHome({ ...share, HOME: "C:\\Users\\me" }, "win32", () => true)).toBe("C:\\Users\\me");
    expect(gitHome(share, "linux", () => true)).toBe(homedir());
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

  it("answers a read its signal stopped as one Git could not answer, starting no Git, and lists the entries next time", async () => {
    const root = await conflicted();
    const read = gitUnmergedReader();
    const stopped = new GitReads();
    await stopped.stop();

    await expect(stopped.run(signal => read(root, { signal }))).resolves.toBeUndefined();
    expect([...(await reader()(root)).keys()]).toEqual(["a.py"]);
    expect([...((await read(root)) ?? new Map()).keys()]).toEqual(["a.py"]);
  });

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

  it("on Windows also looks in Program Files on the drive of the account's home", () => {
    const search = { platform: "win32" as const, home: String.raw`D:\Users\a`, path: undefined, cwd: undefined };
    expect(gitCandidates(search, disk([]))).toEqual(expect.arrayContaining([
      String.raw`D:\Program Files\Git\cmd\git.exe`,
      String.raw`D:\Program Files (x86)\Git\cmd\git.exe`,
    ]));
  });

  it("on Windows looks at each path once, whatever case PATH or the home spells it in", () => {
    // Windows paths ignore case: `c:\program files\git\cmd` on PATH and a home on `c:` name the C: install again.
    const search = { platform: "win32" as const, home: String.raw`c:\Users\me`, path: String.raw`c:\program files\git\cmd`, cwd: undefined };
    expect(gitCandidates(search, disk([]))).toEqual([
      String.raw`c:\program files\git\cmd\git.exe`,
      String.raw`C:\Program Files (x86)\Git\cmd\git.exe`,
      String.raw`c:\Users\me\AppData\Local\Programs\Git\cmd\git.exe`,
      String.raw`c:\Users\me\scoop\apps\git\current\cmd\git.exe`,
    ]);
  });

  it("keeps every absolute entry outside node_modules when the folder it runs in is in no repository", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: "/home/me/bin", cwd: "/home/me" };
    expect(gitCandidates(search, disk([]))[0]).toBe("/home/me/bin/git");
  });

  it("skips the PATH entries of the repository it was told to search from, wherever it runs", async () => {
    // The daemon runs in the Rerail home and serves the repository it was started for (`--workspace-root`).
    const repository = await realpath(await mkdtemp(join(tmpdir(), "rerail-git-served-")));
    roots.push(repository);
    await mkdir(join(repository, ".git"));
    const tools = await realpath(await mkdtemp(join(tmpdir(), "rerail-git-tools-")));
    roots.push(tools);
    const program = process.platform === "win32" ? "git.exe" : "git";
    vi.stubEnv("PATH", [join(repository, "bin"), tools].join(delimiter));
    try {
      expect(gitCandidates()).toContain(join(repository, "bin", program));
      searchGitFrom(repository);
      expect(gitCandidates()).not.toContain(join(repository, "bin", program));
      expect(gitCandidates()[0]).toBe(join(tools, program));
    } finally {
      searchGitFrom(undefined);
      vi.unstubAllEnvs();
    }
  });

  it("still lists PATH and the install locations from a working folder that was deleted", async ({ skip }) => {
    skip(process.platform === "win32", "Windows keeps a process's working folder open, so it cannot be deleted under the process");
    const gone = await mkdtemp(join(tmpdir(), "rerail-git-gone-"));
    const before = process.cwd();
    process.chdir(gone);
    try {
      await removeFolder(gone);
      expect(() => process.cwd()).toThrow();
      vi.stubEnv("PATH", "/opt/local/bin");
      expect(gitCandidates()).toContain("/opt/local/bin/git");
    } finally {
      vi.unstubAllEnvs();
      process.chdir(before);
    }
  });
});

describe("findGit", () => {
  /** A disk holding `files`, each runnable, that counts the realpath calls it answers. */
  const counted = (files: readonly string[]) => {
    const realpaths: string[] = [];
    return {
      realpaths,
      exists: (path: string) => files.includes(path),
      runnable: (path: string) => files.includes(path),
      realpath: (path: string) => {
        realpaths.push(path);
        return path;
      },
      developerDir: () => undefined,
    };
  };
  const entries = Array.from({ length: 40 }, (_, index) => `/opt/tool-${index}/bin`);

  it("stops at the first Git that runs, without resolving the PATH entries after it", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: ["/opt/first/bin", "/opt/git/bin", ...entries].join(":"), cwd: "/home/me/app" };
    const disk = counted(["/home/me/app/.git", "/opt/git/bin/git"]);
    expect(findGit(search, disk).path).toBe("/opt/git/bin/git");
    // The repository it runs in, then each entry up to the Git found.
    expect(disk.realpaths).toEqual(["/home/me/app", "/opt/first/bin", "/opt/git/bin"]);
    // Outside any repository no entry needs its real path.
    const outside = counted(["/opt/git/bin/git"]);
    expect(findGit({ ...search, cwd: "/home/me" }, outside).path).toBe("/opt/git/bin/git");
    expect(outside.realpaths).toEqual([]);
  });

  it("on Windows goes behind a launcher only into a Git for Windows install, by the checks every PATH entry gets", () => {
    const repository = String.raw`C:\code\app`;
    const search = { platform: "win32" as const, home: String.raw`C:\Users\me`, cwd: repository };
    const layout = (root: string, ...more: string[]) => [String.raw`${root}\cmd\git.exe`, String.raw`${root}\mingw64\bin\git.exe`, ...more];
    const found = (path: string, files: readonly string[], links: Record<string, string> = {}) => findGit({ ...search, path }, {
      ...counted([String.raw`${repository}\.git`, ...files]),
      realpath: (folder: string) => Object.entries(links).reduce((real, [link, target]) => real.startsWith(link) ? target + real.slice(link.length) : real, folder),
    }).path;
    const installed = String.raw`C:\Program Files\Git`;
    expect(found(String.raw`${installed}\cmd`, layout(installed, String.raw`${installed}\etc\gitconfig`))).toBe(String.raw`${installed}\mingw64\bin\git.exe`);
    expect(found(String.raw`${installed}\cmd`, layout(installed, String.raw`${installed}\mingw64\share\git`))).toBe(String.raw`${installed}\mingw64\bin\git.exe`);
    // A launcher someone put on PATH beside a git.exe of their choosing, with nothing of Git for Windows around it.
    const planted = String.raw`D:\tools\planted`;
    expect(found(String.raw`${planted}\cmd`, layout(planted))).toBe(String.raw`${planted}\cmd\git.exe`);
    // A real install whose mingw64 folder is a junction into the repository Rerail runs in.
    const linked = String.raw`D:\portable\Git`;
    const junction = { [String.raw`${linked}\mingw64`]: String.raw`${repository}\vendor\mingw64` };
    expect(found(String.raw`${linked}\cmd`, layout(linked, String.raw`${linked}\etc\gitconfig`), junction)).toBe(String.raw`${linked}\cmd\git.exe`);
  });

  it("lists every place it looked, in gitCandidates' order, when no Git runs", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: entries.join(":"), cwd: undefined };
    const found = findGit(search, counted([]));
    expect(found.path).toBeUndefined();
    expect(found.looked).toEqual(gitCandidates(search, counted([])));
    expect(new GitUnavailableError(found.looked).message).toContain("/opt/tool-39/bin/git, /usr/bin/git");
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
