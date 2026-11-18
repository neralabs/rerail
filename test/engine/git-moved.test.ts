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
import { chmod, mkdir, mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });
vi.mock("node:fs/promises", { spy: true });

import { findGit, gitExecutable, gitHead, gitLivesOn, readGitBlobs, runGit, runGitAsync } from "../../src/engine/git.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const actual = await vi.importActual<typeof import("node:child_process")>("node:child_process");
const actualFiles = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");

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
    // As the daemon, which lives on: only such a process looks for Git again.
    gitLivesOn();
    expect(gitExecutable()).toBe(join(folders[0]!, GIT));
  });

  afterAll(async () => {
    vi.unstubAllEnvs();
    await removeFolder(root);
  });

  /**
   * `performance.now()` held by the test from now on: it moves only by `move`, until `release`. It starts on a whole
   * millisecond, so the time left of a limit reads as a whole number of them, as the moves make it.
   */
  function holdClock(): { move(ms: number): void; release(): void } {
    let now = Math.ceil(performance.now());
    const held = vi.spyOn(performance, "now").mockImplementation(() => now);
    return {
      move(ms) {
        now += ms;
      },
      release() {
        held.mockRestore();
      },
    };
  }

  /** The time limit each start of `starter` was given since the last call. */
  function limits(starter: typeof spawnSync): unknown[] {
    return vi.mocked(starter).mock.calls.map(call => (call[2] as { timeout?: number } | undefined)?.timeout);
  }

  it("runs a blocking Git call once more with the Git a look finds within the call's time limit, and keeps that Git", async () => {
    // The repository a session works in is such a call (repository-identity.ts): it finds the repository, not a folder
    // outside Git, and the session's writes are checked.
    const [kept, now] = await moveKeptGit();
    started(spawnSync);
    const result = runGit(repository, ["rev-parse", "HEAD"]);
    expect([result.status, result.stdout.trim(), started(spawnSync)]).toEqual([0, head, [kept, now]]);
    expect(gitExecutable()).toBe(now);
    started(spawn);
    await expect(gitHead(repository)).resolves.toBe(head);
    expect(started(spawn)).toEqual([now]);
  });

  it("fails a blocking Git call as its start did when its time limit leaves the look no time, and the next call finds Git", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawnSync);
    const failed = runGit(repository, ["rev-parse", "HEAD"], { timeout: 0 });
    expect([failed.status, (failed.error as NodeJS.ErrnoException | undefined)?.code, started(spawnSync)]).toEqual([null, "ENOENT", [kept]]);
    // The look goes on off the thread, and a call that does not block waits for it rather than making its own.
    started(spawn);
    const result = await runGitAsync(repository, ["rev-parse", "HEAD"]);
    expect([result.status, result.stdout.trim(), started(spawn)]).toEqual([0, head, [kept, now]]);
  });

  it("gives the look and the second start of a blocking call only what is left of the call's time limit", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawnSync);
    const clock = holdClock();
    // The look's first wait for the thread it reads the disk in takes 4 s of the call's 10 s.
    const wait = Atomics.wait.bind(Atomics);
    vi.spyOn(Atomics, "wait").mockImplementationOnce((...args) => {
      clock.move(4_000);
      return wait(...args);
    });
    try {
      const result = runGit(repository, ["rev-parse", "HEAD"]);
      expect([result.status, result.stdout.trim(), limits(spawnSync), started(spawnSync)]).toEqual([0, head, [10_000, 6_000], [kept, now]]);
    } finally {
      vi.mocked(Atomics.wait).mockRestore();
      clock.release();
    }
  });

  it("waits for a blocking call's look only for what its first start left of the call's time limit", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawnSync);
    const clock = holdClock();
    // The first start takes 7 s of the call's 10 s.
    vi.mocked(spawnSync).mockImplementationOnce((file, args, options) => {
      clock.move(7_000);
      return actual.spawnSync(program(file), args, options);
    });
    const waits: number[] = [];
    const wait = Atomics.wait.bind(Atomics);
    vi.spyOn(Atomics, "wait").mockImplementation((...args) => {
      waits.push(args[3] ?? Infinity);
      return wait(...args);
    });
    try {
      const result = runGit(repository, ["rev-parse", "HEAD"]);
      // Before the fix the look's first wait could last the call's whole 10 s again.
      expect([result.status, Math.max(...waits) <= 3_000, limits(spawnSync), started(spawnSync)]).toEqual([0, true, [10_000, 3_000], [kept, now]]);
    } finally {
      vi.mocked(Atomics.wait).mockRestore();
      clock.release();
    }
  });

  it("gives the look and the second start of a call that does not block only what is left of the call's time limit", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawn);
    const clock = holdClock();
    // The look's first question about the disk takes 6 s of the call's 10 s.
    vi.mocked(stat).mockImplementationOnce(async (...args) => {
      clock.move(6_000);
      return await actualFiles.stat(...args);
    });
    // The second start runs a program that does not end by itself, so only a time limit ends it.
    const second = Promise.withResolvers<void>();
    vi.mocked(spawn)
      .mockImplementationOnce((file, args, options) => actual.spawn(program(file), args, options))
      .mockImplementationOnce((_file, _args, options) => {
        second.resolve();
        return actual.spawn(process.execPath, ["-e", "setInterval(() => {}, 1_000)"], options);
      });
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const call = runGitAsync(repository, ["rev-parse", "HEAD"]);
      await second.promise;
      // Past the call's own limit, and past a new 10 s limit a second start of its own would end at.
      clock.move(10_000);
      vi.advanceTimersByTime(1_000);
      const result = await call;
      // The Gits started, not the taskkill.exe that ends the second one on Windows.
      const gits = started(spawn).filter(program => String(program).startsWith(root));
      expect([result.status, result.error?.message, gits]).toEqual([null, "Git was ended: it ran past its time limit of 4000 ms", [kept, now]]);
    } finally {
      vi.useRealTimers();
      clock.release();
    }
  });

  it("answers a call that does not block as its start did once the look has taken its whole time limit", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawn);
    const clock = holdClock();
    vi.mocked(stat).mockImplementationOnce(async (...args) => {
      clock.move(10_000);
      return await actualFiles.stat(...args);
    });
    try {
      const failed = await runGitAsync(repository, ["rev-parse", "HEAD"]);
      expect([failed.status, (failed.error as NodeJS.ErrnoException | undefined)?.code, started(spawn)]).toEqual([null, "ENOENT", [kept]]);
    } finally {
      clock.release();
    }
    // The look found Git and kept it: the next call starts it.
    await expect(gitHead(repository)).resolves.toBe(head);
    expect(started(spawn)).toEqual([now]);
  });

  it("waits for the look only for what its first start left of the call's time limit", async () => {
    const [kept] = await moveKeptGit();
    started(spawn);
    const clock = holdClock();
    // The first start takes 7 s of the call's 10 s; the look then waits on a folder that has not answered.
    vi.mocked(spawn).mockImplementationOnce((file, args, options) => {
      clock.move(7_000);
      return actual.spawn(program(file), args, options);
    });
    const { promise: held, resolve: answer } = Promise.withResolvers<void>();
    const { promise: asked, resolve: reached } = Promise.withResolvers<void>();
    vi.mocked(stat).mockImplementationOnce(async (...args) => {
      reached();
      await held;
      return await actualFiles.stat(...args);
    });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const call = runGitAsync(repository, ["rev-parse", "HEAD"]);
      await asked;
      await vi.advanceTimersByTimeAsync(3_000);
      const answered = await Promise.race([call, new Promise(done => setImmediate(done, "waiting"))]);
      // Answered at the 3 s its first start left, as that start did; before, the call waited its whole 10 s again.
      expect([answered === "waiting" ? answered : { status: (answered as Awaited<typeof call>).status, code: ((answered as Awaited<typeof call>).error as NodeJS.ErrnoException | undefined)?.code }, started(spawn)]).toEqual([{ status: null, code: "ENOENT" }, [kept]]);
    } finally {
      vi.useRealTimers();
      clock.release();
      answer();
    }
    // A read waits for the look the call left going, so the next case starts from the Git that look keeps.
    await expect(gitHead(repository)).resolves.toBe(head);
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

  it("starts no other Git for a read stopped while the look for another Git runs, and keeps the Git that look found", async () => {
    const [kept, now] = await moveKeptGit();
    started(spawn);
    const stop = new AbortController();
    // The look's first question about the disk: the read is stopped while the look waits for its answer.
    vi.mocked(stat).mockImplementationOnce(async (...args) => {
      stop.abort();
      return await actualFiles.stat(...args);
    });
    await expect(gitHead(repository, { signal: stop.signal })).resolves.toBeUndefined();
    expect(started(spawn)).toEqual([kept]);
    // The stopped read answered without waiting for the look, which goes on: the next read takes the Git it finds.
    await expect(gitHead(repository)).resolves.toBe(head);
    expect(started(spawn).at(-1)).toBe(now);
    await expect(gitHead(repository)).resolves.toBe(head);
    expect(started(spawn)).toEqual([now]);
  });

  it("answers a read stopped while the look waits on a folder that has not answered, and the next read takes the Git it finds", async () => {
    const [kept] = await moveKeptGit();
    started(spawn);
    const stop = new AbortController();
    // The look's first question about whether a candidate runs is held, as a share off the network holds it, and the
    // read is stopped there.
    const { promise: held, resolve: answer } = Promise.withResolvers<void>();
    const { promise: asked, resolve: reached } = Promise.withResolvers<void>();
    vi.mocked(stat).mockImplementationOnce(async (...args) => {
      stop.abort();
      reached();
      await held;
      return await actualFiles.stat(...args);
    });
    const read = gitHead(repository, { signal: stop.signal });
    try {
      await asked;
      const answered = await Promise.race([read.then(value => ({ value })), new Promise(done => setImmediate(done, "waiting"))]);
      expect([answered, started(spawn)]).toEqual([{ value: undefined }, [kept]]);
    } finally {
      answer();
    }
    await expect(gitHead(repository)).resolves.toBe(head);
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
