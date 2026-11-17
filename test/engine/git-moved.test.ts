/**
 * The Git a process keeps, moved under it, as an upgrade from Git for Windows 2.55 to 2.56 removes the
 * `mingw64\bin\git.exe` a running daemon kept. A test file of its own: a process keeps the first Git it finds
 * (`gitExecutable`), and here that is a stand-in this file puts on `PATH` before anything asks for Git.
 *
 * Each stand-in is an empty file Rerail takes for a Git, in one of three folders on `PATH`. A test moves the kept one to
 * the next folder; Rerail's start of the path it kept then fails as the operating system fails it (`ENOENT`), and a
 * start of the stand-in that is there runs this machine's Git in its place.
 */
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });

import { findGit, gitExecutable, gitHead, readGitBlobs, runGit, runGitAsync } from "../../src/engine/git.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const actual = await vi.importActual<typeof import("node:child_process")>("node:child_process");

const GIT = process.platform === "win32" ? "git.exe" : "git";

describe("the Git a process keeps, gone under it", () => {
  let root = "";
  let repository = "";
  let head = "";
  let folders: string[] = [];
  /** First on `PATH` and empty at the first look: a stand-in placed there later is what a look would keep. */
  let earlier = "";
  let machineGit = "";

  /** The program a start names, or this machine's Git in place of a stand-in that is there. */
  const program = (named: string): string => (named.startsWith(root) && existsSync(named) ? machineGit : named);

  async function placeStandIn(folder: string): Promise<string> {
    const standIn = join(folder, GIT);
    await writeFile(standIn, "");
    await chmod(standIn, 0o755);
    return standIn;
  }

  /**
   * Moves the Git the process keeps to the next folder on `PATH`: `[kept, now]`. The kept stand-in may be gone already,
   * when an earlier test's call did not find Git again, so each test fails on its own checks, not on this move.
   */
  async function moveKeptGit(): Promise<[string, string]> {
    const kept = gitExecutable();
    const at = folders.findIndex(folder => kept === join(folder, GIT));
    // Only a stand-in is ever removed, never this machine's Git.
    if (at < 0) throw new Error(`the Git kept is ${kept}, not a stand-in of this file`);
    await rm(kept, { force: true });
    return [kept, await placeStandIn(folders[(at + 1) % folders.length]!)];
  }

  /** The program each start of `starter` named since the last call. */
  function started(starter: (...args: never[]) => unknown): unknown[] {
    const named = vi.mocked(starter).mock.calls.map(call => call[0]);
    vi.mocked(starter).mockClear();
    return named;
  }

  beforeAll(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-git-moved-"));
    repository = join(root, "repository");
    await mkdir(repository);
    const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd: repository, encoding: "utf8" });
    git("init", "--quiet");
    await writeFile(join(repository, "a.py"), "print(1)\n");
    git("add", ".");
    git("commit", "--quiet", "-m", "seed");
    head = git("rev-parse", "HEAD").trim();
    // Looked for without keeping it, before the stand-ins go on PATH.
    machineGit = findGit().path!;
    folders = ["first", "second", "third"].map(name => join(root, name));
    earlier = join(root, "earlier");
    await Promise.all([earlier, ...folders].map(async folder => await mkdir(folder)));
    await placeStandIn(folders[0]!);
    vi.stubEnv("PATH", [earlier, ...folders, ...(process.env.PATH === undefined ? [] : [process.env.PATH])].join(delimiter));
    vi.mocked(spawnSync).mockImplementation((file, args, options) => actual.spawnSync(program(file), args, options));
    vi.mocked(spawn).mockImplementation((file, args, options) => actual.spawn(program(file), args, options));
    expect(gitExecutable()).toBe(join(folders[0]!, GIT));
  });

  afterAll(async () => {
    vi.unstubAllEnvs();
    await removeFolder(root);
  });

  it("fails a blocking Git call whose Git is gone as its start did, and has Git looked for off the thread for the calls after it", async () => {
    // A look may wait on a folder far longer than Git's time limit, and a blocking call would hold the thread through it.
    const [kept, now] = await moveKeptGit();
    started(spawnSync);
    const failed = runGit(repository, ["rev-parse", "HEAD"]);
    expect([failed.status, (failed.error as NodeJS.ErrnoException | undefined)?.code]).toEqual([null, "ENOENT"]);
    expect(started(spawnSync)).toEqual([kept]);
    // A call that does not block, failing on the same Git meanwhile, waits for that look rather than making its own.
    started(spawn);
    const result = await runGitAsync(repository, ["rev-parse", "HEAD"]);
    expect([result.status, result.stdout.trim(), started(spawn)]).toEqual([0, head, [kept, now]]);
    expect(runGit(repository, ["rev-parse", "HEAD"]).stdout.trim()).toBe(head);
    expect(started(spawnSync)).toEqual([now]);
  });

  it("runs a Git call that does not block once more with the Git found again", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawn);
    const result = await runGitAsync(repository, ["rev-parse", "HEAD"]);
    expect([result.status, result.stdout.trim()]).toEqual([0, head]);
    expect(started(spawn)).toEqual([kept, now]);
    expect(gitExecutable()).toBe(now);
  });

  it("runs a read once more with the Git found again, once for reads that failed together, and keeps that Git", async () => {
    let [kept, now] = await moveKeptGit();
    started(spawn);
    await expect(Promise.all([gitHead(repository), gitHead(repository)])).resolves.toEqual([head, head]);
    expect(started(spawn)).toEqual([kept, kept, now, now]);
    await expect(gitHead(repository)).resolves.toBe(head);
    expect(started(spawn)).toEqual([now]);

    [kept, now] = await moveKeptGit();
    started(spawn);
    const blobs = await readGitBlobs(repository, ["HEAD:./a.py"], { maxBlobBytes: 1_000 });
    expect(Object.fromEntries([...blobs].map(([revision, blob]) => [revision, blob.toString("utf8")]))).toEqual({ "HEAD:./a.py": "print(1)\n" });
    expect(started(spawn)).toEqual([kept, now]);
  });

  it("starts no other Git for a read stopped while its Git failed to start, and the next read finds it", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawn);
    const stop = new AbortController();
    vi.mocked(spawn).mockImplementationOnce((file, args, options) => {
      stop.abort();
      return actual.spawn(program(file), args, options);
    });
    await expect(gitHead(repository, { signal: stop.signal })).resolves.toBeUndefined();
    expect(started(spawn)).toEqual([kept]);
    expect(gitExecutable()).toBe(kept);
    await expect(gitHead(repository)).resolves.toBe(head);
    expect(started(spawn)).toEqual([kept, now]);
  });

  it("starts no other Git for a call that does not block, stopped while its Git failed to start, and the next call finds it", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawn);
    const stop = new AbortController();
    vi.mocked(spawn).mockImplementationOnce((file, args, options) => {
      stop.abort();
      return actual.spawn(program(file), args, options);
    });
    const stopped = await runGitAsync(repository, ["rev-parse", "HEAD"], { signal: stop.signal });
    expect([stopped.status, (stopped.error as NodeJS.ErrnoException | undefined)?.code]).toEqual([null, "ENOENT"]);
    expect(started(spawn)).toEqual([kept]);
    expect(gitExecutable()).toBe(kept);
    const result = await runGitAsync(repository, ["rev-parse", "HEAD"]);
    expect([result.status, result.stdout.trim()]).toEqual([0, head]);
    expect(started(spawn)).toEqual([kept, now]);
  });

  it("looks for Git again only when a start finds no program, not when Git exits non-zero or prints past maxBuffer", async () => {
    const kept = gitExecutable();
    // First on the first look's PATH, so a look now would keep it.
    const sooner = await placeStandIn(earlier);
    try {
      const calls = [
        runGit(repository, ["rev-parse", "--verify", "--quiet", "no-such-branch"]),
        runGit(repository, ["rev-parse", "HEAD"], { maxBuffer: 1 }),
        await runGitAsync(repository, ["rev-parse", "--verify", "--quiet", "no-such-branch"]),
        await runGitAsync(repository, ["rev-parse", "HEAD"], { maxBuffer: 1 }),
      ];
      // Git exits 1 for a ref it lacks. Past maxBuffer the call carries its error; its status is null, or 0 when Git
      // exited before Node ended it.
      expect([calls[0]!.status, calls[2]!.status]).toEqual([1, 1]);
      expect(calls.map(call => call.error === undefined)).toEqual([true, false, true, false]);
      expect(gitExecutable()).toBe(kept);
    } finally {
      await rm(sooner);
    }
  });
});
