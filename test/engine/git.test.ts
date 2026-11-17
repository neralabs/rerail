import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { basename, delimiter, join, win32 } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });
vi.mock("../../src/engine/child-process.js", { spy: true });

import { endProcessTree } from "../../src/engine/child-process.js";
import {
  findGit, gitBehindLauncher, gitBranchTips, gitCandidates, gitCommitsBetween, gitConfigFiles, gitCrlfPaths, gitExcludeFiles, gitExecutable, gitHead, gitHome,
  gitInstalled, gitLook, gitModifiedOrUntrackedListing, gitModifiedOrUntrackedPaths, GitReads, GitUnavailableError, gitUnmergedReader,
  headMovedOnlyByCommits, ProcessGit, readGitBlobIds, readGitBlobs, runGit, runGitAsync, searchGitFrom,
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
    // A stopped listing names no file changed, and answers that it cannot tell rather than none.
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

  it.runIf(process.platform === "win32")("ends the Git behind a launcher with the launcher, and answers once both have exited", async () => {
    const root = await repository({ "a.py": "1\n" });
    const pids = await mkdtemp(join(tmpdir(), "rerail-git-launched-"));
    roots.push(pids);
    const [headPid, blobsPid] = [join(pids, "head"), join(pids, "blobs")];
    // The head's read starts its Git first, then the blobs' read.
    vi.mocked(spawn).mockImplementationOnce((_git, args, options) =>
      actualChildProcess.spawn(process.execPath, ["-e", LAUNCHER, "--", headPid, ...args], options));
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
  it("answers failed for a listing Git failed, where gitModifiedOrUntrackedPaths answers none", async () => {
    const root = await repository({ "a.py": "1\n" });
    await writeFile(join(root, "b.py"), "untracked\n");
    await expect(gitModifiedOrUntrackedListing(root)).resolves.toEqual({ ended: "printed", paths: ["b.py"] });
    // Git cannot start in a folder that is not there.
    const gone = join(root, "gone");
    await expect(gitModifiedOrUntrackedListing(gone)).resolves.toEqual({ ended: "failed" });
    await expect(gitModifiedOrUntrackedPaths(gone)).resolves.toEqual([]);
  });
});

/** How many bytes the process `heldListing`'s filter leaves writes to the stderr it holds: 16 times a Linux pipe's 64 KiB. */
const LEFTOVER_BYTES = 1024 * 1024;

/**
 * What this process holds open: its descriptors, where the system lists them (Linux), and the pipes Node keeps open
 * (on every system; one a stream holds unreferenced is not among them).
 */
interface HeldHandles {
  readonly descriptors: number | undefined;
  readonly pipes: number;
}

/** What this process holds open now ({@link HeldHandles}). */
async function heldHandles(): Promise<HeldHandles> {
  const descriptors = await readdir("/proc/self/fd").then(entries => entries.length, () => undefined);
  return { descriptors, pipes: process.getActiveResourcesInfo().filter(resource => resource === "PipeWrap").length };
}

/** Resolves once this process holds no more than it held `before` ({@link HeldHandles}); fails after 5 s. */
async function handlesBack(before: HeldHandles): Promise<void> {
  await vi.waitFor(async () => {
    const now = await heldHandles();
    expect(now.pipes).toBeLessThanOrEqual(before.pipes);
    expect(now.descriptors ?? 0).toBeLessThanOrEqual(before.descriptors ?? 0);
  }, { timeout: 5_000, interval: 20 });
}

/** `path` as a POSIX shell, Git for Windows' among them, takes it. */
function shellPath(path: string): string {
  return path.replaceAll("\\", "/");
}

/**
 * Lists the changed files of a repository whose clean filter leaves a process behind, outside the repository and with
 * no parent left, that holds Git's stderr while a file is there, and answers what the read answered within 5 s, or that
 * it was still waiting. Git runs the filter to compare a.py, changed on disk, with the index. With `pastLimit` Git then
 * waits for b.py's filter ({@link hangAfterHeld}), and once both have started the clock moves past the read's time
 * limit: the read ends Git at its next look. The wait ends once the listing is answered, and so does removing the
 * repository. The process then writes `LEFTOVER_BYTES` to the stderr it holds, in a subshell of its own, and writes how
 * that subshell ended to `wrote` (`wrote 0`: every byte went; `wrote 141`: a broken pipe's SIGPIPE ended it). Each run
 * of the filter writes `+` to `loops` before it starts its process, which writes `-` there as it ends: the test waits
 * for that, and for b.py's filter to end, so no process outlives it. The process looks for the file every 20 ms, so
 * that wait is short. `before` is what this process held open as the read started, after a first read, so what Node
 * opens once for its first child process is in it.
 */
async function heldListing({ pastLimit = false } = {}): Promise<{ answer: unknown; wrote: string; before: HeldHandles }> {
  const root = await repository({ ".gitattributes": "a.py filter=held\nb.py filter=hang\n", "a.py": "print(1)\n", "b.py": "print(1)\n" });
  const inGit = (name: string): string => join(root, ".git", name);
  const [held, loops, wrote, started, hung] = [inGit("held"), inGit("loops"), inGit("wrote"), inGit("started"), inGit("hung")];
  const waitForFile = `while [ -e '${shellPath(held)}' ]; do sleep 0.02; done`;
  const write = `(head -c ${LEFTOVER_BYTES} /dev/zero >&2); echo "wrote $?" >> '${shellPath(wrote)}'`;
  const leftover = `echo + >> '${shellPath(started)}'; ${waitForFile}; ${write}; echo - >> '${shellPath(loops)}'`;
  git(root, "config", "filter.held.clean", `echo + >> '${shellPath(loops)}'; ((cd / && ${leftover}) </dev/null >/dev/null &); cat`);
  // Without a command for filter.hang, Git takes b.py as it is, and b.py is unchanged unless `pastLimit`.
  if (pastLimit) await hangAfterHeld(root, held, hung);
  await writeFile(held, "");
  await writeFile(join(root, "a.py"), "print(2)\n");
  await gitHead(root);
  const before = await heldHandles();
  if (pastLimit) fakeReadTimers();
  let late: NodeJS.Timeout | undefined;
  let answer: unknown;
  try {
    const read = gitModifiedOrUntrackedPaths(root);
    if (pastLimit) await moveClockPastLimit([started, hung]);
    answer = await Promise.race([read, new Promise(settle => { late = setTimeout(settle, 5_000, "still waiting at 5 s"); })]);
  } finally {
    clearTimeout(late);
    await rm(held, { force: true });
    await vi.waitFor(async () => expect(await loopsRunning(loops)).toBe(0), { timeout: 5_000, interval: 10 });
    await hungGone(hung);
  }
  return { answer, wrote: (await readFile(wrote, "utf8")).trim(), before };
}

/**
 * In Git's place for b.py's clean filter, a Node program: it writes its pid to the file named second, then stays,
 * reading nothing, until the file named first is gone, so Git waits for it. A shell waiting in a loop of `sleep` there
 * is ended at the read's time limit while it may be starting a `sleep`: on Windows `taskkill /T` then left a shell
 * running past the test once in 20 runs.
 */
const HANGS_UNTIL_GONE =
  "const fs = require('node:fs'); const [held, hung] = process.argv.slice(2); fs.writeFileSync(hung, String(process.pid));" +
  " setInterval(() => { if (!fs.existsSync(held)) process.exit(0); }, 20);";

/** Has `root`'s clean filter for b.py run {@link HANGS_UNTIL_GONE} with `held` and `hung`, and changes b.py so Git runs it. */
async function hangAfterHeld(root: string, held: string, hung: string): Promise<void> {
  const script = join(root, ".git", "hang.js");
  await writeFile(script, HANGS_UNTIL_GONE);
  const command = [process.execPath, script, held, hung].map(part => `'${shellPath(part)}'`).join(" ");
  git(root, "config", "filter.hang.clean", `exec ${command}`);
  await writeFile(join(root, "b.py"), "print(2)\n");
}

/** Resolves once the program {@link HANGS_UNTIL_GONE} named in `hung` has ended, at once when none started; fails after 5 s. */
async function hungGone(hung: string): Promise<void> {
  const pid = Number(await readFile(hung, "utf8").catch(() => ""));
  if (pid > 0) await vi.waitFor(() => expect(processState(pid)).toBe("gone"), { timeout: 5_000, interval: 10 });
}

/** Once each of `files` holds something, moves the clock and the read's look past its time limit. */
async function moveClockPastLimit(files: readonly string[]): Promise<void> {
  for (const file of files) {
    await vi.waitFor(async () => expect(await readFile(file, "utf8").catch(() => "")).not.toBe(""), { timeout: 10_000, interval: 10 });
  }
  clockAhead(11_000);
  vi.advanceTimersByTime(1_000);
}

/** How many of the processes `heldListing`'s filter started have not written their end to `loops`. */
async function loopsRunning(loops: string): Promise<number> {
  const marks = (await readFile(loops, "utf8").catch(() => "")).split("\n");
  return marks.filter(mark => mark === "+").length - marks.filter(mark => mark === "-").length;
}

/** How far `clockAhead` has moved `performance.now()` in this test: one spy adds it, however often the clock moves. */
const clock = { ahead: 0 };

/** From now on `performance.now()` answers `ms` later than it did: the clock a read finds after the event loop stalled. */
function clockAhead(ms: number): void {
  if (clock.ahead === 0) {
    const now = performance.now.bind(performance);
    vi.spyOn(performance, "now").mockImplementation(() => now() + clock.ahead);
  }
  clock.ahead += ms;
}

/**
 * The timers a Git read looks at its time limit by, faked so a test moves them on (`vi.advanceTimersByTime`) and no test
 * waits the limit out; with `limitTimer`, `setTimeout` too, the timer a read whose limit were a plain timer would end by.
 * The clock stays real: `clockAhead` moves it.
 */
function fakeReadTimers({ limitTimer = false } = {}): void {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", ...(limitTimer ? ["setTimeout", "clearTimeout"] as const : [])] });
}

/** The last `count` Gits the reads started, oldest first. */
function lastGits(count: number): ChildProcess[] {
  return vi.mocked(spawn).mock.results.slice(-count).flatMap(result => result.type === "return" ? [result.value] : []);
}

/**
 * How long a stall goes on once the Gits it waits for have exited, as a real stall goes on after Git finished. On Windows
 * Node learns of an exit from a wait thread, which may tell it a moment later; on Linux the signal is in by then.
 */
const STALL_AFTER_EXIT_MS = process.platform === "win32" ? 500 : 0;

/**
 * A stall of `ms` once the current pass of the event loop reaches its immediates: it blocks the event loop until each of
 * `gits` has exited (at most 5 s: on macOS an exited process this one has not reaped still answers as running) and
 * `STALL_AFTER_EXIT_MS` more, then moves the clock and the faked timers `ms` on. Those timers come due before the event
 * loop handles the exits, as after a stall in which Git finished.
 */
function stallUntilExited(gits: readonly ChildProcess[], ms: number): void {
  const exited = (child: ChildProcess) => child.pid === undefined || processState(child.pid) === "gone";
  setImmediate(() => {
    const until = Date.now() + 5_000;
    const blocked = new Int32Array(new SharedArrayBuffer(4));
    while (!gits.every(exited) && Date.now() < until) Atomics.wait(blocked, 0, 0, 5);
    Atomics.wait(blocked, 0, 0, STALL_AFTER_EXIT_MS);
    clockAhead(ms);
    vi.advanceTimersByTime(ms);
  });
}

/** Runs `node -e script` in place of the next Git a read starts; resolves with that process. */
function gitInPlace(script: string): Promise<ChildProcess> {
  const started = Promise.withResolvers<ChildProcess>();
  vi.mocked(spawn).mockImplementationOnce((_git, args, options) => {
    const child = actualChildProcess.spawn(process.execPath, ["-e", script, "--", ...args], options);
    started.resolve(child);
    return child;
  });
  return started.promise;
}

/** In Git's place: prints a path and stays until its input ends (the test's, or the test process's). */
const PRINTS_AND_WAITS = "require('node:fs').writeSync(1, 'a.py\\0'); process.stdin.on('end', () => process.exit(0)).resume()";

/** Resolves once `child`'s output has given its first bytes. */
async function firstOutput(child: ChildProcess): Promise<void> {
  await new Promise(settle => child.stdout?.once("data", settle));
}

/**
 * Once `child` has printed, tells the read its output ended, as Node does when Git closes it. No program can close its
 * own output and go on running on every platform: on Windows libuv keeps descriptors 0 to 2 open.
 */
async function outputEnds(child: ChildProcess): Promise<void> {
  await firstOutput(child);
  child.stdout?.emit("end");
}

/** Resolves once the look a faked tick asked for has run: it runs one loop turn after the tick, before this. */
async function afterLook(): Promise<void> {
  await new Promise(settle => setImmediate(settle));
}

/** Answers that the read was still waiting, once `ms` has passed on a real timer. */
async function stillWaiting(ms: number): Promise<string> {
  return await new Promise<string>(settle => { setTimeout(settle, ms, `still waiting at ${ms / 1000} s`).unref(); });
}

describe("Git reads and their time limit", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    clock.ahead = 0;
  });

  it("answers with what Git printed when the event loop stalls past the time limit after Git finished", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    await writeFile(join(root, "a.py"), "print(2)\n");
    const head = git(root, "rev-parse", "HEAD").trim();
    vi.mocked(endProcessTree).mockClear();
    fakeReadTimers({ limitTimer: true });
    const reads = Promise.all([gitModifiedOrUntrackedPaths(root), gitHead(root)]);
    // Both Gits finish during the stall, and the event loop handles their exits only once it ends, past the 10 s limit.
    stallUntilExited(lastGits(2), 10_500);
    await expect(reads).resolves.toEqual([["a.py"], head]);
    // Neither read was ended: on Windows that ran `taskkill /T /F` on the pid of a Git already gone.
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("answers once Git has exited and its output has ended, while a process it started goes on writing to Git's stderr", async () => {
    // The process the clean filter left ends only once the read has answered, so the answer did not wait for it. It then
    // writes a MiB, more than a pipe holds, to the stderr it holds: every byte goes, with no wait and no error. Once it
    // has ended, nothing of the read is held open.
    const listing = await heldListing();
    expect(listing.answer).toEqual(["a.py"]);
    expect(listing.wrote).toBe("wrote 0");
    await handlesBack(listing.before);
  }, 30_000);

  it("lets a process Git started write to Git's stderr after the read ended Git at its time limit, and keeps nothing of the read", async () => {
    // Git waits for the clean filter past the read's limit, so the read ends Git there; the process the filter left
    // goes on, outside Git's process tree, and writes a MiB to the stderr it holds as before.
    const listing = await heldListing({ pastLimit: true });
    expect(listing.answer).toBeUndefined();
    expect(listing.wrote).toBe("wrote 0");
    await handlesBack(listing.before);
  }, 30_000);

  it("ends a read whose Git prints nothing at its first look past the time limit on the clock, after a stall", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    fakeReadTimers();
    // In Git's place, a program that prints nothing and stays until it is ended, as a Git kept waiting does.
    const started = gitInPlace("process.stdin.resume()");
    const read = gitModifiedOrUntrackedPaths(root);
    await started;
    // As if the event loop had stalled for 11 s as the read started: its first look, 1 s in by its timer, is 12 s in on
    // the clock. A read whose limit were a timer of 10 s would still be waiting at 2 s.
    clockAhead(11_000);
    vi.advanceTimersByTime(1_000);
    await expect(Promise.race([read, stillWaiting(2_000)])).resolves.toBeUndefined();
  }, 30_000);

  it("answers with what Git printed when the stall that took the read past the time limit on the clock ended after Git finished", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    const head = git(root, "rev-parse", "HEAD").trim();
    vi.mocked(endProcessTree).mockClear();
    fakeReadTimers();
    const read = gitHead(root);
    clockAhead(10_000);
    // Git finishes during a stall of 1 s, which takes the clock past the limit. The read's tick runs first after it,
    // before the event loop handles Git's exit.
    stallUntilExited(lastGits(1), 1_000);
    await expect(read).resolves.toBe(head);
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("ends a read whose Git still prints at its first look past the time limit on the clock, and answers that it ran late", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    fakeReadTimers();
    // In Git's place, a program that prints a path every 20 ms until it is ended, as a Git listing a large checkout does.
    const printing = gitInPlace("setInterval(() => process.stdout.write('a.py\\0'), 20)");
    const read = gitModifiedOrUntrackedPaths(root);
    await firstOutput(await printing);
    clockAhead(10_000);
    vi.advanceTimersByTime(1_000);
    // The read's next look finds it past the limit on the clock, with output still coming: it ends there, as a timer of
    // the time limit ended it, and a walk asks the listing once more off its path. A read whose limit were a timer of
    // 10 s would still be waiting at 2 s.
    await expect(Promise.race([read, stillWaiting(2_000)])).resolves.toBeUndefined();
  }, 30_000);

  it("answers with what Git printed when its output ended before its look past the time limit, and it exited after", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    vi.mocked(endProcessTree).mockClear();
    fakeReadTimers();
    const started = gitInPlace(PRINTS_AND_WAITS);
    const read = gitModifiedOrUntrackedPaths(root);
    const child = await started;
    await outputEnds(child);
    clockAhead(10_000);
    vi.advanceTimersByTime(1_000);
    await afterLook();
    // That look saw Git's output end since the look before, so the read went on once more; Git exits now.
    child.stdin?.end();
    await expect(read).resolves.toEqual(["a.py"]);
    // The look did not end it: on Windows that starts `taskkill /T /F`, slower than Git's exit here, which answers too.
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("ends a read whose Git's output ended before one look past the time limit, and which has not exited by the next", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    fakeReadTimers();
    const started = gitInPlace(PRINTS_AND_WAITS);
    const read = gitModifiedOrUntrackedPaths(root);
    await outputEnds(await started);
    clockAhead(10_000);
    vi.advanceTimersByTime(1_000);
    await afterLook();
    // The next look finds nothing new since: Git is past the limit and not done, so the read ends it and answers late.
    vi.advanceTimersByTime(1_000);
    await expect(Promise.race([read, stillWaiting(2_000)])).resolves.toBeUndefined();
  }, 30_000);
});

describe("runGitAsync", () => {
  /** In Git's place: prints nothing and stays until it is ended, as a Git kept waiting does. */
  const HANGS = "setTimeout(() => {}, 30_000)";

  it("answers Git's status and what it printed, with `input` on its stdin", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    const head = git(root, "rev-parse", "HEAD").trim();
    await expect(runGitAsync(root, ["cat-file", "--batch-check=%(objectname)"], { input: "HEAD\n" }))
      .resolves.toEqual({ status: 0, stdout: `${head}\n`, stderr: "" });
    const failed = await runGitAsync(root, ["rev-parse", "--verify", "no-such-ref"]);
    expect(failed).toMatchObject({ status: 128, stdout: "" });
    expect(failed.stderr).not.toBe("");
    expect(failed.error).toBeUndefined();
  });

  it("answers a null status with an error once Git prints more than maxBuffer", async () => {
    const root = await repository({ "a.py": "x".repeat(4_000) });
    const result = await runGitAsync(root, ["cat-file", "blob", "HEAD:a.py"], { maxBuffer: 1_000 });
    expect(result.status).toBeNull();
    expect(result.error?.message).toMatch(/maxBuffer/u);
  });

  it("ends a Git that hangs at its timeout, with every process it started, and answers a null status once it has exited", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    vi.mocked(endProcessTree).mockClear();
    const started = gitInPlace(HANGS);
    const run = runGitAsync(root, ["status"], { timeout: 300 });
    const child = await started;
    const result = await Promise.race([run, stillWaiting(3_000)]);
    expect(result).toMatchObject({ status: null, error: expect.objectContaining({ message: expect.stringMatching(/time limit/u) }) });
    expect(endProcessTree).toHaveBeenCalledExactlyOnceWith(child);
    expect(ending(child)).not.toEqual({ exitCode: null, signalCode: null });
  }, 30_000);

  it("ends a Git its signal stops, and starts none once the signal is aborted", async () => {
    const root = await repository({ "a.py": "print(1)\n" });
    vi.mocked(endProcessTree).mockClear();
    const started = gitInPlace(HANGS);
    const stopping = new AbortController();
    // A timeout this long never ends the run here: only the signal does.
    const run = runGitAsync(root, ["status"], { timeout: 60_000, signal: stopping.signal });
    const child = await started;
    stopping.abort();
    const result = await Promise.race([run, stillWaiting(3_000)]);
    expect(result).toMatchObject({ status: null, error: expect.objectContaining({ message: expect.stringMatching(/stopped/u) }) });
    expect(endProcessTree).toHaveBeenCalledExactlyOnceWith(child);
    expect(ending(child)).not.toEqual({ exitCode: null, signalCode: null });

    const spawned = vi.mocked(spawn).mock.calls.length;
    await expect(runGitAsync(root, ["status"], { signal: stopping.signal })).resolves.toMatchObject({ status: null, stdout: "" });
    expect(vi.mocked(spawn).mock.calls.length).toBe(spawned);
  }, 30_000);

  it.runIf(process.platform === "win32")("ends the Git behind a launcher at its timeout, and answers once both have exited", async () => {
    const root = await repository({ "a.py": "1\n" });
    const pids = await mkdtemp(join(tmpdir(), "rerail-git-launched-"));
    roots.push(pids);
    const pidFile = join(pids, "git");
    vi.mocked(spawn).mockImplementationOnce((_git, args, options) =>
      actualChildProcess.spawn(process.execPath, ["-e", LAUNCHER, "--", pidFile, ...args], options));
    const run = runGitAsync(root, ["status"], { timeout: 300 });
    const launched = await launchedGit(pidFile);
    await expect(Promise.race([run, stillWaiting(5_000)])).resolves.toMatchObject({ status: null });

    // The run settles only once the whole tree has exited, so the Git the launcher started is not waited for here.
    const running = processState(launched) === "gone" ? [] : [launched];
    for (const pid of running) process.kill(pid);
    expect(running).toEqual([]);
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
    expect((await gitExcludeFiles(root))?.[1]).toBe(join(home, "xdg", "git", "ignore"));
  });

  it("follows core.excludesFile, with `~` read as the home, and gives a linked worktree the common info/exclude", async () => {
    const { root, home } = await isolatedRepository();
    git(root, "config", "core.excludesFile", "~/ignores");
    git(root, "worktree", "add", "--quiet", join(home, "linked"));
    expect(await gitExcludeFiles(join(home, "linked"))).toEqual([join(root, ".git", "info", "exclude"), join(home, "ignores")]);
  });

  it("answers undefined outside a Git checkout, and when Git does not answer: no files is not its answer", async () => {
    const { root, home } = await isolatedRepository();
    expect(await gitExcludeFiles(home)).toBeUndefined();
    expect(await gitConfigFiles(home)).toBeUndefined();
    const stopped = new AbortController();
    stopped.abort();
    expect(await gitExcludeFiles(root, { signal: stopped.signal })).toBeUndefined();
    expect(await gitConfigFiles(root, { signal: stopped.signal })).toBeUndefined();
  });

  it("names the configuration files that can set core.excludesFile: the repository's, the worktree's and the global ones", async () => {
    const { root, home } = await isolatedRepository();
    vi.stubEnv("GIT_CONFIG_GLOBAL", "");
    expect(await gitConfigFiles(root)).toEqual([
      join(root, ".git", "config"), join(root, ".git", "config.worktree"), join(home, ".gitconfig"), join(home, ".config", "git", "config"),
    ]);
    vi.stubEnv("GIT_CONFIG_GLOBAL", join(home, "global"));
    expect((await gitConfigFiles(root))?.slice(2)).toEqual([join(home, "global")]);
  });

  it("finds the global files in Git's own home, which on Windows without HOME is HOMEDRIVE and HOMEPATH", async () => {
    const { root, home } = await isolatedRepository();
    vi.stubEnv("GIT_CONFIG_GLOBAL", "");
    const share = join(home, "share");
    await mkdir(share);
    const globals = [join(share, ".gitconfig"), join(share, ".config", "git", "config")];
    // Git's home, not the account's: HOME here names another folder than the one homedir() answers.
    expect((await gitConfigFiles(root, { env: { ...process.env, HOME: share } }))?.slice(2)).toEqual(globals);
    if (process.platform === "win32") {
      const { HOME: _home, ...withoutHome } = process.env;
      const drive = share.slice(0, 2);
      expect((await gitConfigFiles(root, { env: { ...withoutHome, HOMEDRIVE: drive, HOMEPATH: share.slice(2) }, platform: "win32" }))?.slice(2)).toEqual(globals);
    }
  });
});

describe("gitBehindLauncher", () => {
  const git = String.raw`C:\Program Files\Git`;
  const launcher = String.raw`${git}\cmd\git.exe`;
  const search = { platform: "win32" as const, home: String.raw`C:\Users\me`, path: undefined, cwd: undefined };
  const installed = (...paths: string[]) => {
    const files = [launcher, String.raw`${git}\etc\gitconfig`, ...paths];
    return { exists: (path: string) => files.includes(path), runnable: (path: string) => files.includes(path), realpath: (path: string) => path, developerDir: () => undefined };
  };

  it("runs Git for Windows' own git.exe, not the launcher that starts it as a child a timeout would leave running", () => {
    const real = String.raw`${git}\mingw64\bin\git.exe`;
    expect(gitBehindLauncher(launcher, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(String.raw`${git}\bin\git.exe`, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(launcher, search, installed(String.raw`${git}\clangarm64\bin\git.exe`))).toBe(String.raw`${git}\clangarm64\bin\git.exe`);
    // The real one is itself, and a Git laid out otherwise (a scoop shim, a launcher without its Git) is run as found.
    expect(gitBehindLauncher(real, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(String.raw`D:\scoop\shims\git.exe`, search, installed(real))).toBe(String.raw`D:\scoop\shims\git.exe`);
    expect(gitBehindLauncher(launcher, search, installed())).toBe(launcher);
  });

  // Joined, not written in a template: there `\u` starts a Unicode escape, and a parser that holds to that skips the
  // whole file.
  const ucrt = win32.join(git, "ucrt64", "bin", "git.exe");

  it("runs the git.exe of the x64 build of Git for Windows from 2.56 on, in ucrt64", () => {
    expect(gitBehindLauncher(launcher, search, installed(ucrt))).toBe(ucrt);
    expect(gitBehindLauncher(String.raw`${git}\bin\git.exe`, search, installed(ucrt))).toBe(ucrt);
    expect(gitBehindLauncher(ucrt, search, installed(ucrt))).toBe(ucrt);
  });

  // The same answers as before ucrt64 was known: these pass on the code without it too.
  it("keeps running another build's git.exe where an install also holds ucrt64", () => {
    for (const build of ["mingw64", "clangarm64", "mingw32"]) {
      const other = win32.join(git, build, "bin", "git.exe");
      expect(gitBehindLauncher(launcher, search, installed(other, ucrt)), build).toBe(other);
      expect(gitBehindLauncher(launcher, search, installed(ucrt, other)), build).toBe(other);
    }
  });

  it.runIf(process.platform === "win32")("is what Rerail runs on this Windows machine", () => {
    expect(gitExecutable()).toBe(gitBehindLauncher(gitExecutable()));
    expect(gitExecutable().toLowerCase()).not.toMatch(/\\cmd\\git\.exe$/u);
  });

  it.runIf(process.platform === "win32")("goes behind this machine's Git for Windows launcher to the git.exe the launcher itself runs", ({ skip }) => {
    // Git's exec path is `<Git>\<build>\libexec\git-core`. The install comes from the Git Rerail runs, whichever `PATH`
    // entry found it (Git Bash's names the build's own `bin` before `cmd`). The build comes from the launcher's own
    // answer, so on an install that holds two builds Rerail must pick the one Git for Windows itself runs.
    const execPath = (program: string) => execFileSync(program, ["--exec-path"], { encoding: "utf8" }).trim();
    const install = win32.dirname(win32.dirname(win32.dirname(execPath(gitExecutable()))));
    const machineLauncher = win32.join(install, "cmd", "git.exe");
    skip(!existsSync(machineLauncher), "this machine's Git is not a Git for Windows install");
    const build = win32.dirname(win32.dirname(execPath(machineLauncher)));
    expect(gitBehindLauncher(machineLauncher)).toBe(win32.join(build, "bin", "git.exe"));
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

describe("gitLook", () => {
  it("looks again by what it read first: a .git gone or made since moves no PATH entry in or out", () => {
    const files = new Set(["/home/me/app/.git"]);
    const disk = { exists: (path: string) => files.has(path), runnable: () => false, realpath: (path: string) => path, developerDir: () => undefined };
    // Relative, in node_modules, in the repository it runs in, in another folder, outside both.
    const path = "tools:/home/me/app/node_modules/.bin:/home/me/app/bin:/home/me/web/bin:/opt/git/bin";
    const inRepository = gitLook({ platform: "linux", home: "/home/me", path, cwd: "/home/me/app/src" }, disk);
    const inNone = gitLook({ platform: "linux", home: "/home/me", path, cwd: "/home/me/web" }, disk);
    // No Git runs, so each look lists every candidate it tries.
    const first = [inRepository().looked, inNone().looked];
    expect(first.map(looked => looked.slice(0, 3))).toEqual([
      ["/home/me/web/bin/git", "/opt/git/bin/git", "/usr/bin/git"],
      ["/home/me/app/bin/git", "/home/me/web/bin/git", "/opt/git/bin/git"],
    ]);
    files.delete("/home/me/app/.git");
    files.add("/home/me/web/.git");
    expect([inRepository().looked, inNone().looked]).toEqual(first);
  });
});

describe("ProcessGit", () => {
  /** A search that finds each of `paths` in turn, then the last one again, and counts its looks. */
  const searches = (...paths: (string | undefined)[]) => {
    let looks = 0;
    const find = () => ({ path: paths[Math.min(looks++, paths.length - 1)], looked: ["/opt/git/bin/git", "/usr/bin/git"] });
    return { find, looks: () => looks };
  };

  it("makes its look at the first ask and keeps it and the Git it finds, or says where it looked when none is installed", () => {
    const search = searches("/opt/git/bin/git", "/usr/bin/git");
    let decided = 0;
    const git = new ProcessGit(() => {
      decided += 1;
      return search.find;
    });
    expect(decided).toBe(0);
    expect([git.path(), git.path()]).toEqual(["/opt/git/bin/git", "/opt/git/bin/git"]);
    expect(git.afterMissing("/opt/git/bin/git")).toBe("/usr/bin/git");
    expect([decided, search.looks()]).toEqual([1, 2]);
    expect(() => new ProcessGit(() => searches(undefined).find).path()).toThrow(GitUnavailableError);
  });

  it("looks again once the Git it kept is gone, once for every start that failed on it", () => {
    // Git for Windows 2.56 removes the mingw64 git.exe that a process started on 2.55 kept.
    const [before, after] = [String.raw`C:\Program Files\Git\mingw64\bin\git.exe`, win32.join(String.raw`C:\Program Files\Git`, "ucrt64", "bin", "git.exe")];
    const search = searches(before, after);
    const git = new ProcessGit(() => search.find);
    expect(git.path()).toBe(before);
    expect(git.afterMissing(before)).toBe(after);
    // A start that failed on the same Git at the same time takes the Git found, with no look of its own.
    expect(git.afterMissing(before)).toBe(after);
    expect([git.path(), search.looks()]).toEqual([after, 2]);
  });

  it("keeps the Git it has when the look finds none or the same one, so the next start that fails looks again", () => {
    const search = searches("/opt/git/bin/git", undefined, "/opt/git/bin/git", "/usr/bin/git");
    const git = new ProcessGit(() => search.find);
    expect(git.path()).toBe("/opt/git/bin/git");
    // No Git anywhere, as while an installer replaces it: this start fails, and Rerail still has a Git to try.
    expect(git.afterMissing("/opt/git/bin/git")).toBeUndefined();
    expect(git.path()).toBe("/opt/git/bin/git");
    // The same path, back: the next call starts it, this one does not start it twice.
    expect(git.afterMissing("/opt/git/bin/git")).toBeUndefined();
    expect(git.afterMissing("/opt/git/bin/git")).toBe("/usr/bin/git");
    expect([git.path(), search.looks()]).toEqual(["/usr/bin/git", 4]);
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

/** The most bytes of Git's output `gitHead` reads: past them it answers none. */
const READ_LIMIT = 1024 * 1024;

/** The most bytes of Git's output a listing of changed files or of unmerged entries reads. */
const LISTING_LIMIT = 64 * 1024 * 1024;

/**
 * How long a test waits for the answer to a read whose Git printed past its limit: well under the read's time limit of
 * 10 s, so an answer by then came from the output limit and not from the time limit.
 */
const PAST_LIMIT_WAIT_MS = 5_000;

/** In Git's place: prints `bytes` bytes of `fill`, then stays until its input ends, as a Git still at work does. */
function printsAndStays(bytes: number, fill: string): string {
  return `process.stdout.write(Buffer.alloc(${bytes}, ${JSON.stringify(fill)})); process.stdin.resume()`;
}

/** How `child` ended: by itself, with its exit code, or by a signal. */
function ending(child: ChildProcess): { exitCode: number | null; signalCode: NodeJS.Signals | null } {
  return { exitCode: child.exitCode, signalCode: child.signalCode };
}

/** Has the next Git a read starts run as itself: a stand-in `gitInPlace` queues after this takes the Git after it. */
function realGitNext(): void {
  vi.mocked(spawn).mockImplementationOnce(actualChildProcess.spawn);
}

// These tests come last in the file: after the two listings of 64 MiB, this process holds more memory for a while, and
// each Git a later test started took about half as long again on Linux.
describe("Git reads and their output limit", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    clock.ahead = 0;
  });

  it("answers none when Git prints past the MiB a read takes, and ends that Git at once with SIGTERM", async () => {
    const root = await repository({ "a.py": "1\n" });
    vi.mocked(endProcessTree).mockClear();
    // A MiB of two-byte characters and one byte more: 1,048,577 bytes in 524,289 characters, as the limit counts bytes.
    const started = gitInPlace(`process.stdout.write('\\u00e9'.repeat(${READ_LIMIT / 2}) + 'a'); process.stdin.resume()`);
    const read = gitHead(root);
    const child = await started;
    await expect(Promise.race([read, stillWaiting(PAST_LIMIT_WAIT_MS)])).resolves.toBeUndefined();
    // The read settles once its Git has exited, so none is left running; the time limit did not end it.
    expect(ending(child)).toEqual({ exitCode: null, signalCode: "SIGTERM" });
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("answers in whole what Git printed when that is one byte under the MiB a read takes, or exactly that MiB", async () => {
    const root = await repository({ "a.py": "1\n" });
    gitInPlace(`process.stdout.write('a'.repeat(${READ_LIMIT - 1}))`);
    gitInPlace(`process.stdout.write('a'.repeat(${READ_LIMIT}))`);
    const answers = await Promise.all([gitHead(root), gitHead(root)]);
    // Lengths, and whether each is all `a`: a diff of a MiB would hide what failed.
    expect(answers.map(answer => answer?.length)).toEqual([READ_LIMIT - 1, READ_LIMIT]);
    expect(answers.map(answer => /^a*$/u.test(answer ?? "b"))).toEqual([true, true]);
  }, 30_000);

  it("reads a character whole when Git prints its bytes in two pieces", async () => {
    const root = await repository({ "a.py": "1\n" });
    // In Git's place: prints `a` and the first byte of `é`, and once the test writes to its input, the other byte and `b`.
    const started = gitInPlace([
      "const fs = require('node:fs'); const text = Buffer.from('a\\u00e9b'); fs.writeSync(1, text.subarray(0, 2));",
      "process.stdin.once('data', () => { fs.writeSync(1, text.subarray(2)); process.exit(0); });",
    ].join(" "));
    const read = gitHead(root);
    const child = await started;
    // The read has had the first piece alone: nothing more is printed until this.
    await firstOutput(child);
    child.stdin?.write("\n");
    await expect(read).resolves.toBe("aéb");
  }, 30_000);

  it("lists no file when Git lists past the 64 MiB a listing takes, and ends that Git at once with SIGTERM", async () => {
    const root = await repository({ "a.py": "1\n" });
    vi.mocked(endProcessTree).mockClear();
    const started = gitInPlace(printsAndStays(LISTING_LIMIT + 1, "a.py\0"));
    const read = gitModifiedOrUntrackedPaths(root);
    const child = await started;
    await expect(Promise.race([read, stillWaiting(PAST_LIMIT_WAIT_MS)])).resolves.toEqual([]);
    expect(ending(child)).toEqual({ exitCode: null, signalCode: "SIGTERM" });
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("cannot say which entries are unmerged when Git lists past the 64 MiB a listing takes, and ends that Git at once", async () => {
    const root = await repository({ "a.py": "1\n" });
    vi.mocked(endProcessTree).mockClear();
    // The reader's first Git finds the index file; the second lists the entries.
    realGitNext();
    const started = gitInPlace(printsAndStays(LISTING_LIMIT + 1, `100644 ${"0".repeat(40)} 1\ta.py\0`));
    const read = gitUnmergedReader()(root);
    const child = await started;
    await expect(Promise.race([read, stillWaiting(PAST_LIMIT_WAIT_MS)])).resolves.toBeUndefined();
    expect(ending(child)).toEqual({ exitCode: null, signalCode: "SIGTERM" });
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("ends a Git that goes on printing past the limit and ignores SIGTERM at once, by closing its output", async () => {
    const root = await repository({ "a.py": "1\n" });
    vi.mocked(endProcessTree).mockClear();
    // In Git's place: prints 64 KiB at a time, 16 MiB at most, and exits with status 0 once a write fails; SIGTERM does
    // not end it. The read answers none all the same: it had more than it takes.
    const started = gitInPlace([
      "process.on('SIGTERM', () => {}); process.stdout.on('error', () => process.exit(0)); process.stdin.resume();",
      "const chunk = Buffer.alloc(65536, 'a'); let left = 256;",
      "const more = () => { if (left-- > 0) process.stdout.write(chunk, error => { if (!error) more(); }); }; more();",
    ].join(" "));
    const read = gitHead(root);
    const child = await started;
    await expect(Promise.race([read, stillWaiting(PAST_LIMIT_WAIT_MS)])).resolves.toBeUndefined();
    // Its next write found the output closed. Windows ends a process on SIGTERM whatever it asked for.
    expect(ending(child)).toEqual(process.platform === "win32" ? { exitCode: null, signalCode: "SIGTERM" } : { exitCode: 0, signalCode: null });
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);

  it("ends a Git that printed past the limit, ignores SIGTERM and prints no more at the read's time limit", async () => {
    const root = await repository({ "a.py": "1\n" });
    vi.mocked(endProcessTree).mockClear();
    // A timer made before the read's timers are faked runs on the real clock.
    const late = stillWaiting(PAST_LIMIT_WAIT_MS);
    fakeReadTimers({ limitTimer: true });
    const started = gitInPlace(`process.on('SIGTERM', () => {}); ${printsAndStays(READ_LIMIT + 1, "a")}`);
    const read = gitHead(root);
    const child = await started;
    if (process.platform !== "win32") {
      // Once the read has sent SIGTERM, the clock and the timers move past its time limit of 10 s.
      await vi.waitFor(() => expect(child.killed).toBe(true), { timeout: PAST_LIMIT_WAIT_MS, interval: 10 });
      clockAhead(11_000);
      vi.advanceTimersByTime(11_000);
    }
    await expect(Promise.race([read, late])).resolves.toBeUndefined();
    // Windows ends a process on SIGTERM whatever it asked for; elsewhere the time limit sends SIGKILL.
    expect(ending(child)).toEqual({ exitCode: null, signalCode: process.platform === "win32" ? "SIGTERM" : "SIGKILL" });
    expect(endProcessTree).toHaveBeenCalledTimes(process.platform === "win32" ? 0 : 1);
  }, 30_000);

  it("reads a listing past the 64 MiB a listing takes as late when its Git ignores SIGTERM and prints no more, and ends that Git at the read's time limit", async ({ skip }) => {
    skip(process.platform !== "linux", "Linux only: Windows ends a process on SIGTERM whatever it asked for, so there the listing ends at once and lists no file, as the 64 MiB listing test above holds; no CI job runs these tests on macOS, where this case has not run");
    const root = await repository({ "a.py": "1\n" });
    vi.mocked(endProcessTree).mockClear();
    // A timer made before the read's timers are faked runs on the real clock.
    const late = stillWaiting(PAST_LIMIT_WAIT_MS);
    fakeReadTimers({ limitTimer: true });
    const started = gitInPlace(`process.on('SIGTERM', () => {}); ${printsAndStays(LISTING_LIMIT + 1, "a.py\0")}`);
    const read = gitModifiedOrUntrackedPaths(root);
    const child = await started;
    // Once the read has sent SIGTERM, the clock and the timers move past its time limit of 10 s.
    await vi.waitFor(() => expect(child.killed).toBe(true), { timeout: PAST_LIMIT_WAIT_MS, interval: 10 });
    clockAhead(11_000);
    vi.advanceTimersByTime(11_000);
    // Undefined, not none: Git did not say which files changed, so a walk asks the listing once more off its path.
    await expect(Promise.race([read, late])).resolves.toBeUndefined();
    expect(ending(child)).toEqual({ exitCode: null, signalCode: "SIGKILL" });
    expect(endProcessTree).toHaveBeenCalledTimes(1);
  }, 30_000);

  it("answers what Git printed when it writes more to its stderr than a read takes, since that stderr is the null device", async () => {
    const root = await repository({ "a.py": "1\n" });
    vi.mocked(endProcessTree).mockClear();
    // In Git's place: writes 2 MiB to its stderr, then `abc` to its output, and exits with status 0. Had the read kept a
    // pipe for that stderr and not read it, this program would wait at its write there until the read's time limit.
    const started = gitInPlace(`const fs = require('node:fs'); fs.writeSync(2, Buffer.alloc(${2 * READ_LIMIT}, 'e')); fs.writeSync(1, 'abc');`);
    const read = gitHead(root);
    const child = await started;
    await expect(Promise.race([read, stillWaiting(PAST_LIMIT_WAIT_MS)])).resolves.toBe("abc");
    expect(ending(child)).toEqual({ exitCode: 0, signalCode: null });
    expect(endProcessTree).not.toHaveBeenCalled();
  }, 30_000);
});
