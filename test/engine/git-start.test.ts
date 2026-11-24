import { execFileSync } from "node:child_process";
import { once } from "node:events";
import { closeSync, constants as fsConstants, existsSync, openSync, readdirSync, readFileSync } from "node:fs";
import { copyFile, mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { constants, tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../../src/engine/child-process.js", { spy: true });

import { spawn, spawnSync } from "../../src/engine/child-process.js";
import { findGit, gitCrlfPaths, gitHead, gitModifiedOrUntrackedPaths, readGitBlobIds, readGitBlobs, runGit, runGitAsync, type RunGitOptions } from "../../src/engine/git.js";
import {
  runGitSync,
  startGit,
  startGitThroughNode,
  startGitWithoutCopying,
  type EngineGitExit,
  type EngineGitStarts,
  type EngineStartedGit,
  type GitProcess,
  type GitRunLimits,
  type GitRunResult,
} from "../../src/engine/git-start.js";
import { startGitInThread } from "../../src/engine/git-thread.js";
import { readGitInstallation } from "../../src/engine/native/git-access.js";
import { loadNativeBinding } from "../../src/engine/native/native.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { blockingRuns, blockingRunsThroughEngine, lacksPrlimit } from "./support/blocking-git-runs.js";
import { daemonProcess, packageRoot, until, watchedEnvironment } from "../support/processes/daemon-home.js";

const LINUX = process.platform === "linux";
const WINDOWS = process.platform === "win32";
const folders: string[] = [];

afterEach(() => {
  startGitThroughNode();
  vi.mocked(spawn).mockClear();
  vi.mocked(spawnSync).mockClear();
});

afterAll(async () => {
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
});

async function folder(name: string): Promise<string> {
  const made = await mkdtemp(join(tmpdir(), `rerail-git-start-${name}-`));
  folders.push(made);
  return made;
}

/** A repository whose one commit holds a text file, a CRLF file by attribute, and a binary file; one file is changed after. */
async function repository(): Promise<string> {
  const root = await folder("repo");
  const git = (...args: string[]): string => execFileSync("git", ["-C", root, ...args], { encoding: "utf8" });
  git("init", "--quiet");
  git("config", "user.email", "test@example.invalid");
  git("config", "user.name", "test");
  await writeFile(join(root, ".gitattributes"), "*.bat text eol=crlf\n");
  await writeFile(join(root, "a.py"), "print(1)\n");
  await writeFile(join(root, "run.bat"), "echo 1\n");
  await writeFile(join(root, "blob.bin"), Buffer.from([0, 1, 2, 255, 0, 10, 13]));
  git("add", ".");
  git("commit", "--quiet", "-m", "seed");
  await writeFile(join(root, "a.py"), "print(2)\n");
  await writeFile(join(root, "new.py"), "print(3)\n");
  return root;
}

/** Every answer a walk and a boundary ask of Git, read once. */
async function answers(root: string): Promise<unknown> {
  const revisions = ["HEAD:a.py", "HEAD:blob.bin", ":run.bat", "HEAD:missing.py"];
  return {
    head: await gitHead(root),
    blobs: Object.fromEntries([...(await readGitBlobs(root, revisions, { maxBlobBytes: 1 << 20 }))].map(([at, blob]) => [at, blob.toString("hex")])),
    ids: Object.fromEntries(await readGitBlobIds(root, revisions)),
    crlf: await gitCrlfPaths(root, ["run.bat", "a.py"]),
    changed: await gitModifiedOrUntrackedPaths(root),
    hashed: await runGitAsync(root, ["hash-object", "--stdin"], { input: "some text\n" }),
    failed: await runGitAsync(root, ["rev-parse", "--verify", "no-such-ref"]),
    ran: runGit(root, ["cat-file", "-p", "HEAD:blob.bin"]),
    ranAndFailed: runGit(root, ["rev-parse", "--verify", "no-such-ref"]),
    listed: runGit(root, ["ls-files", "-z"], { maxBuffer: 64 }),
  };
}

/** What `child` printed, once it has closed, with how it ended. */
async function ended(child: GitProcess): Promise<{ stdout: string; code: number | null; signal: NodeJS.Signals | null }> {
  let stdout = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => { stdout += chunk; });
  const [code, signal] = (await once(child, "close")) as [number | null, NodeJS.Signals | null];
  return { stdout, code, signal };
}

/** The children of this process that have exited and are not reaped. */
function zombies(): number {
  return readdirSync("/proc").filter(name => /^\d+$/u.test(name)).filter(pid => {
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      const [state, parent] = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
      return state === "Z" && Number(parent) === process.pid;
    } catch {
      return false;
    }
  }).length;
}

describe.runIf(LINUX)("Git started through the engine, on Linux", () => {
  it("answers every read a walk and a boundary make as Git started through Node answers it", async () => {
    const root = await repository();
    const throughNode = await answers(root);
    vi.mocked(spawn).mockClear();
    vi.mocked(spawnSync).mockClear();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const throughEngine = await answers(root);
    expect(throughEngine).toEqual(throughNode);
    // Node started none of them.
    expect([vi.mocked(spawn).mock.calls.length, vi.mocked(spawnSync).mock.calls.length]).toEqual([0, 0]);
  }, 30_000);

  it("gives Git the signal state, descriptors, environment and folder Node gives it", async () => {
    const script = "grep -E '^Sig(Blk|Ign|Cgt)' /proc/self/status; ls /proc/self/fd | wc -l; env | sort; pwd; test -t 0 || echo not-a-terminal";
    const env = { PATH: process.env.PATH, LANG: "C", RERAIL_GIT_START: "a b=c" };
    const node = await ended(startGit("/bin/sh", ["-c", script], env, "ignore"));
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const engine = await ended(startGit("/bin/sh", ["-c", script], env, "ignore"));
    expect(engine).toEqual(node);
    expect(engine.stdout).toContain("RERAIL_GIT_START=a b=c");
  });

  it("ends a Git with the signal a read's time limit or stop sends, and sends no signal once it is reaped", async () => {
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const child = startGit("/bin/sh", ["-c", "read line"], process.env, "pipe");
    expect(child.kill("SIGKILL")).toBe(true);
    expect(await ended(child)).toEqual({ stdout: "", code: null, signal: "SIGKILL" });
    // Once reaped, its pid can name another process. process.kill stands in for one that took it: it takes the signal.
    const kills = vi.spyOn(process, "kill").mockImplementation(() => true);
    try {
      expect([child.exitCode, child.signalCode, child.kill("SIGTERM"), kills.mock.calls]).toEqual([null, "SIGKILL", false, []]);
    } finally {
      kills.mockRestore();
    }
  });

  it("writes Git's input, closes it, and reads its output and its stderr to their ends", async () => {
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const child = startGit("/bin/sh", ["-c", "cat; echo warned >&2; exit 3"], process.env, "pipe");
    let stderr = "";
    child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
    child.stdin.end("x".repeat(300_000));
    const exited = once(child, "exit");
    const answer = await ended(child);
    expect([answer.stdout.length, answer.code, answer.signal, stderr]).toEqual([300_000, 3, null, "warned\n"]);
    expect(await exited).toEqual([3, null]);
  });

  it("leaves no child unreaped and no descriptor open after its reads", async () => {
    const root = await repository();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    await gitHead(root);
    const open = readdirSync("/proc/self/fd").length;
    for (let read = 0; read < 20; read += 1) await Promise.all([gitHead(root), readGitBlobs(root, ["HEAD:a.py"], { maxBlobBytes: 1 << 20 })]);
    expect([zombies(), readdirSync("/proc/self/fd").length]).toEqual([0, open]);
  }, 30_000);

  it("runs a blocking Git as spawnSync does: its output, its exit, and SIGTERM at its time limit or past its output limit", async () => {
    const files = await folder("limits");
    const throughNode = blockingRuns("limits", files);
    const throughEngine = blockingRunsThroughEngine("limits", files);
    expect(throughEngine).toEqual(throughNode);
    expect(throughEngine).toMatchObject({ late: { signal: "SIGTERM", error: { code: "ETIMEDOUT" } }, long: { signal: "SIGTERM", error: { code: "ENOBUFS" } }, plain: { status: 5, stdout: "no-input\n" }, exact: { status: 0, signal: null } });
  }, 60_000);

  it("ends a blocking Git at its time limit as spawnSync does while a process Git started still holds its output", async () => {
    const files = await folder("held");
    const throughNode = blockingRuns("time-limit-held", files);
    const throughEngine = blockingRunsThroughEngine("time-limit-held", files);
    expect(throughEngine).toEqual(throughNode);
    const timedOut = { stdout: "started\n", error: { code: "ETIMEDOUT" } };
    expect(throughEngine).toMatchObject({
      running: { held: true, answer: { ...timedOut, status: null, signal: "SIGTERM" } },
      exited: { held: true, answer: { ...timedOut, status: 0, signal: null } },
      closed: { ...timedOut, status: null, signal: "SIGTERM" },
      printing: { status: null, signal: "SIGTERM", error: { code: "ETIMEDOUT" } },
      writing: { answer: { ...timedOut, status: 0, signal: null }, write: "141\n" },
    });
  }, 60_000);

  it("ends a blocking Git past its output limit as spawnSync does while a process Git started still holds its output", async () => {
    const files = await folder("held");
    const throughNode = blockingRuns("output-limit-held", files);
    const throughEngine = blockingRunsThroughEngine("output-limit-held", files);
    expect(throughEngine).toEqual(throughNode);
    expect(throughEngine).toMatchObject({ held: true, answer: { stdout: "\0".repeat(1001), status: null, signal: "SIGTERM", error: { code: "ENOBUFS" } } });
  }, 60_000);

  it("starts through Node a start the engine fails and a program named without a folder", async () => {
    const root = await repository();
    // The engine's own report of a start it refused names the call first (`failed` in native/src/git_start.rs).
    const refusing = {
      startGit: (): never => { throw new Error("startGit: Resource temporarily unavailable (os error 11)"); },
      reapGit: () => ({}),
      runGit: (): never => { throw new Error("runGit: Resource temporarily unavailable (os error 11)"); },
    };
    expect(startGitWithoutCopying(() => refusing)).toBe(true);
    expect(await gitHead(root)).toBe(execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim());
    expect(runGit(root, ["rev-parse", "HEAD"]).stdout).toBe(`${await gitHead(root)}\n`);
    expect([vi.mocked(spawn).mock.calls.length, vi.mocked(spawnSync).mock.calls.length]).toEqual([2, 1]);
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    expect((await ended(startGit("sh", ["-c", "echo bare"], process.env, "ignore"))).stdout).toBe("bare\n");
    expect(spawn).toHaveBeenCalledTimes(3);
  }, 30_000);

  it("leaves to Node a start the engine refuses, as Node answers it: an argument holding a NUL byte", () => {
    const refusals = (): unknown[] => [
      thrown(() => startGit("/bin/sh", ["-c", "echo a\0b"], process.env, "ignore")),
      thrown(() => runGitSync("/bin/sh", ["-c", "echo a\0b"], process.env)),
    ];
    const throughNode = refusals();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    expect(refusals()).toEqual(throughNode);
    expect(throughNode).toEqual([{ code: "ERR_INVALID_ARG_VALUE" }, { code: "ERR_INVALID_ARG_VALUE" }]);
  });

  it("runs a blocking Git with the variables its call sets over the process's, as the asks for the engine's Git reads need", async () => {
    // A system and a global file of the process's own, which each ask for the engine's Git reads sets aside (it removes
    // GIT_CONFIG_SYSTEM and reads the global file from the null device), so that Git names the files it was built to read.
    const files = await folder("installation");
    await writeFile(join(files, "system"), "[probe]\n\tsystem = yes\n");
    await writeFile(join(files, "global"), "[probe]\n\tglobal = yes\n");
    const root = await repository();
    const starts = engineStarts();
    let runs = 0;
    expect(startGitWithoutCopying(() => ({ ...starts, runGit: (...call) => { runs += 1; return starts.runGit(...call); } }))).toBe(true);
    const unset = readGitInstallation();
    try {
      vi.stubEnv("GIT_CONFIG_SYSTEM", join(files, "system"));
      vi.stubEnv("GIT_CONFIG_GLOBAL", join(files, "global"));
      expect(readGitInstallation()).toEqual(unset);
      const read = (env?: RunGitOptions["env"]): unknown => runGit(root, ["config", "--get", "probe.global"], env === undefined ? {} : { env }).stdout;
      expect([read(), read({ GIT_CONFIG_GLOBAL: "/dev/null" })]).toEqual(["yes\n", ""]);
    } finally {
      vi.unstubAllEnvs();
    }
    expect(unset).toMatchObject({ settled: true, installation: { configOrigins: expect.any(String) } });
    expect([runs, vi.mocked(spawnSync).mock.calls.length]).toEqual([6, 0]);
  }, 30_000);
});

/** The engine's three calls, for a case to wrap one of them. */
function engineStarts(): Required<EngineGitStarts> {
  const { startGit: start, reapGit, runGit: run } = loadNativeBinding();
  if (start === undefined || reapGit === undefined || run === undefined) throw new Error("this engine cannot start Git");
  return { startGit: start, reapGit, runGit: run };
}

/** The code of what `act` threw, or what it returned when it threw nothing. */
function thrown(act: () => unknown): unknown {
  try {
    return { returned: act() };
  } catch (error) {
    return { code: (error as NodeJS.ErrnoException).code };
  }
}

/**
 * Scripts that end themselves: by signal 6, as a Git that aborts does (made undumpable first, so no core is written),
 * and by signal 29.
 */
const SELF_ENDINGS = [
  ["-c", "exec python3 -c 'import ctypes, os; ctypes.CDLL(None).prctl(4, 0); os.kill(os.getpid(), 6)'"],
  ["-c", "kill -29 $$"],
];

/** The signal each of {@link SELF_ENDINGS} ended with: started to be read, then run to its end while the thread waits. */
async function selfEndings(): Promise<unknown> {
  const ends: unknown[] = [];
  for (const args of SELF_ENDINGS) ends.push([(await ended(startGit("/bin/sh", args, process.env, "ignore"))).signal, runGitSync("/bin/sh", args, process.env).signal]);
  return ends;
}

/** Scripts that end themselves by a signal Node has no name for: the first and the last real-time signal, and one between. */
const UNNAMED_ENDINGS = [34, 40, 64].map(signal => ["-c", `kill -${signal} $$`]);

/** How each of {@link UNNAMED_ENDINGS} ended: as a reader of its start sees it, and as a blocking run answers it. */
async function unnamedEndings(): Promise<unknown> {
  const ends: unknown[] = [];
  for (const args of UNNAMED_ENDINGS) {
    const child = startGit("/bin/sh", args, process.env, "ignore");
    const { code, signal } = await ended(child);
    const run = runGitSync("/bin/sh", args, process.env);
    ends.push([{ code, signal, exitCode: child.exitCode, signalCode: child.signalCode }, { status: run.status, signal: run.signal }]);
  }
  return ends;
}

/**
 * What a reader sees of `child` once `seen` has come, the last event before which an exit could come early: whether it
 * exited before the kill that follows, whether that kill was sent, and how the child ended.
 */
async function endedAtKill(child: GitProcess, seen: Promise<unknown>): Promise<unknown> {
  const exits: unknown[] = [];
  child.on("exit", (...exit: unknown[]) => exits.push(exit));
  const closed = once(child, "close") as Promise<[number | null, NodeJS.Signals | null]>;
  await seen;
  const exitedFirst = exits.length > 0;
  const killed = child.kill("SIGTERM");
  const [code, signal] = await closed;
  return { exitedFirst, killed, code, signal };
}

/** A FIFO's read end at its end already: its one writer opened it and closed it. */
function endedPipe(path: string): number {
  execFileSync("mkfifo", [path]);
  const read = openSync(path, fsConstants.O_RDONLY | fsConstants.O_NONBLOCK);
  closeSync(openSync(path, "w"));
  return read;
}

describe.runIf(LINUX)("Git started through the engine in the states where it and Node's start could part, on Linux", () => {
  it("names the signal that ended Git as Node names it: SIGABRT for 6 and SIGIO for 29", async () => {
    const throughNode = await selfEndings();
    vi.mocked(spawn).mockClear();
    vi.mocked(spawnSync).mockClear();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const throughEngine = await selfEndings();
    expect([vi.mocked(spawn).mock.calls.length, vi.mocked(spawnSync).mock.calls.length]).toEqual([0, 0]);
    expect(throughEngine).toEqual(throughNode);
    expect(throughEngine).toEqual([["SIGABRT", "SIGABRT"], ["SIGIO", "SIGIO"]]);
  });

  it("reports an end by a signal Node has no name for as Node does: exit code 0 to a reader, no status and an empty signal from a blocking run", async () => {
    const throughNode = await unnamedEndings();
    vi.mocked(spawn).mockClear();
    vi.mocked(spawnSync).mockClear();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const throughEngine = await unnamedEndings();
    expect([vi.mocked(spawn).mock.calls.length, vi.mocked(spawnSync).mock.calls.length]).toEqual([0, 0]);
    expect(throughEngine).toEqual(throughNode);
    const unnamed = [{ code: 0, signal: null, exitCode: 0, signalCode: null }, { status: null, signal: "" }];
    expect(throughEngine).toEqual([unnamed, unnamed, unnamed]);
  });

  it("waits for the exit of a Git whose exit pipe closed while it runs, and ends it at a kill as Node's child", async () => {
    const node = startGit("/bin/sh", ["-c", "read line"], process.env, "pipe");
    const throughNode = await endedAtKill(node, once(node, "spawn"));
    const starts = engineStarts();
    const fifo = join(await folder("exit"), "exit");
    const watched: number[] = [];
    // The exit watch's pipe closes while the child runs, as when its wait fails: here a pipe at its end from the start.
    const closesEarly = (...call: Parameters<typeof starts.startGit>): EngineStartedGit => {
      const started = starts.startGit(...call);
      watched.push(started.exited);
      return { ...started, exited: endedPipe(fifo) };
    };
    // The pipe's end read, the child is reaped: what that first reap answered is the last event before an exit could
    // come early, in the same turn.
    let firstReap: (exit: EngineGitExit) => void = () => undefined;
    const reaped = new Promise<EngineGitExit>(resolve => { firstReap = resolve; });
    const reapGit = (pid: number): EngineGitExit => {
      const exit = starts.reapGit(pid);
      firstReap(exit);
      return exit;
    };
    expect(startGitWithoutCopying(() => ({ ...starts, startGit: closesEarly, reapGit }))).toBe(true);
    const engine = startGit("/bin/sh", ["-c", "read line"], process.env, "pipe");
    const throughEngine = await endedAtKill(engine, Promise.all([once(engine, "spawn"), reaped]));
    for (const fd of watched) closeSync(fd);
    expect(throughEngine).toEqual(throughNode);
    expect([throughEngine, await reaped]).toEqual([{ exitedFirst: false, killed: true, code: null, signal: "SIGTERM" }, expect.objectContaining({ running: true })]);
    expect(zombies()).toBe(0);
  });

  it("runs a blocking Git once when what follows the engine's run throws, and lets that throw out", async () => {
    const runs = join(await folder("runs"), "runs");
    const starts = engineStarts();
    const tooLong = new RangeError("Cannot create a string longer than 0x1fffffe8 characters");
    const conversion = new Error("the run's answer could not be handed to JavaScript");
    // After Git has run: its output cannot become a string (past V8's longest), or its answer cannot reach JavaScript.
    const unreadable = { toString: (): never => { throw tooLong; } } as unknown as Buffer;
    const afterRuns: Required<EngineGitStarts>["runGit"][] = [
      (...call) => ({ ...starts.runGit(...call), stdout: unreadable }),
      (...call) => { starts.runGit(...call); throw conversion; },
    ];
    const errors = afterRuns.map(runGitThen => {
      startGitWithoutCopying(() => ({ ...starts, runGit: runGitThen }));
      try {
        return runGitSync("/bin/sh", ["-c", 'echo ran >> "$1"', "sh", runs], process.env);
      } catch (error) {
        return error;
      }
    });
    expect([errors[0] === tooLong, errors[1] === conversion, readFileSync(runs, "utf8"), vi.mocked(spawnSync).mock.calls.length]).toEqual([true, true, "ran\nran\n", 0]);
  });

  it("starts Git once when what follows the engine's start throws, and leaves no descriptor open and no child unreaped", async () => {
    const files = await folder("starts");
    const [runs, plain] = [join(files, "runs"), join(files, "plain")];
    await writeFile(plain, "");
    const starts = engineStarts();
    const conversion = new Error("the start's answer could not be handed to JavaScript");
    const pids: number[] = [];
    // After Git has started: its answer cannot reach JavaScript, so its descriptors never do; or one of them cannot
    // become a socket, as a regular file's cannot.
    const afterStarts: Required<EngineGitStarts>["startGit"][] = [
      (...call) => {
        const started = starts.startGit(...call);
        pids.push(started.pid);
        for (const fd of [started.stdin, started.stdout, started.exited]) closeSync(fd);
        throw conversion;
      },
      (...call) => {
        const started = starts.startGit(...call);
        pids.push(started.pid);
        closeSync(started.stdout);
        return { ...started, stdout: openSync(plain, "r") };
      },
    ];
    const open = readdirSync("/proc/self/fd").length;
    const errors = afterStarts.map(startGitThen => {
      startGitWithoutCopying(() => ({ ...starts, startGit: startGitThen }));
      return thrown(() => startGit("/bin/sh", ["-c", 'echo ran >> "$1"; read line', "sh", runs], process.env, "ignore"));
    });
    try {
      // The first child's answer never reached this process, so nothing here reaps it: the case does.
      await until(() => typeof starts.reapGit(pids[0] ?? 0).code === "number", 10_000, "the first child's exit");
      await until(() => zombies() === 0 && readdirSync("/proc/self/fd").length === open, 10_000, "no child unreaped and no descriptor open");
    } finally {
      for (const pid of pids) {
        try {
          process.kill(pid, "SIGKILL");
        } catch { /* It has ended. */ }
      }
    }
    expect([errors, readFileSync(runs, "utf8"), vi.mocked(spawn).mock.calls.length]).toEqual([[{ code: undefined }, { code: "ERR_INVALID_FD_TYPE" }], "ran\nran\n", 0]);
  }, 30_000);

  it("runs a blocking Git with a maxBuffer or a timeout past the engine's 32 bits as spawnSync runs it", async () => {
    // The one run its output limit ends reads a FIFO this process holds open, so it runs until that limit's SIGTERM
    // whichever start ran it; its time limit only bounds it should the output limit fail. Every other run prints and
    // exits.
    const fifo = join(await folder("limits"), "held");
    execFileSync("mkfifo", [fifo]);
    const held = openSync(fifo, fsConstants.O_RDWR);
    const waits: GitRunLimits = { maxBuffer: 1.5, timeout: 20_000 };
    // 0 and a fraction too: spawnSync reads each limit's 0 in its own way, and refuses a time limit with a fraction.
    const limits = [Infinity, 2 ** 32, 2 ** 32 + 1, -1, Number.NaN, 0, 1.5].flatMap(limit => [limit === 1.5 ? waits : { maxBuffer: limit }, { timeout: limit }]);
    const run = (limit: GitRunLimits): unknown => thrown(() => runGitSync("/bin/sh", ["-c", limit === waits ? 'echo ran; exec cat "$1"' : "echo ran", "sh", fifo], process.env, limit));
    const starts = engineStarts();
    let asked: unknown;
    const recording: Required<EngineGitStarts>["runGit"] = (program, args, env, timeoutMs, maxBuffer) => {
      const ran = starts.runGit(program, args, env, timeoutMs, maxBuffer);
      asked = { timeoutMs, maxBuffer, timedOut: ran.timedOut, pastMaxBuffer: ran.pastMaxBuffer };
      return ran;
    };
    try {
      const throughNode = limits.map(run);
      expect(startGitWithoutCopying(() => ({ ...starts, runGit: recording }))).toBe(true);
      const throughEngine = limits.map(limit => {
        asked = "spawnSync";
        return [run(limit), asked];
      });
      expect(throughEngine.map(([answer]) => answer)).toEqual(throughNode);
      // What the engine was asked, and what it did: only the two runs whose limits it holds reached it.
      const taken = (limit: GitRunLimits): unknown => {
        if (limit === waits) return { timeoutMs: 20_000, maxBuffer: 1.5, timedOut: false, pastMaxBuffer: true };
        return limit.timeout === 0 ? { timeoutMs: undefined, maxBuffer: 1024 * 1024, timedOut: false, pastMaxBuffer: false } : "spawnSync";
      };
      expect(throughEngine.map(([, engine]) => engine)).toEqual(limits.map(taken));
      const { returned: cut } = throughNode[limits.indexOf(waits)] as { returned: GitRunResult };
      expect([cut.status, cut.signal, cut.stdout, (cut.error as NodeJS.ErrnoException | undefined)?.code]).toEqual([null, "SIGTERM", "ran\n", "ENOBUFS"]);
    } finally {
      closeSync(held);
    }
  }, 60_000);

  it("ends a blocking Git and says why when the engine's wait on its output fails, as spawnSync ends a run whose read fails", ({ skip }) => {
    const throughEngine = blockingRunsThroughEngine("poll-fails", tmpdir());
    if (lacksPrlimit(throughEngine)) skip("prlimit (util-linux), which this case lowers the open-file limit with, is not on PATH");
    expect(throughEngine).toMatchObject({ status: null, signal: "SIGTERM", error: { code: "EINVAL", errno: -constants.errno.EINVAL, message: "spawnSync /bin/sh EINVAL" } });
  }, 30_000);
});

describe("Git started through Node, off Linux or without the engine", () => {
  it("is what a process gets off Linux, and in a process whose engine cannot be loaded", () => {
    expect(startGitWithoutCopying(loadNativeBinding, "win32")).toBe(false);
    expect(startGitWithoutCopying(loadNativeBinding, "darwin")).toBe(false);
    expect(startGitWithoutCopying(() => { throw new Error("Rerail's engine for Linux on x64 is not installed"); }, "linux")).toBe(false);
    expect(startGitWithoutCopying(() => ({}), "linux")).toBe(false);
  });
});

/** What a Node script started by {@link startGit} with `input` did: its output, its end, and its events in order. */
async function scriptRun(script: string, input: string, kill = false): Promise<unknown> {
  const child = startGit(process.execPath, ["-e", script], process.env, "pipe");
  const events: string[] = [];
  for (const event of ["spawn", "exit"]) child.on(event, () => events.push(event));
  let stderr = "";
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
  child.stdin.on("error", () => undefined);
  if (kill) expect(child.kill("SIGKILL")).toBe(true);
  child.stdin.end(input);
  const answer = await ended(child);
  return { ...answer, stderr, events, pid: typeof child.pid, exit: [child.exitCode, child.signalCode] };
}

/** How a start of a program that is not there ends: its error, its pid, and its close. */
async function missingStart(): Promise<unknown> {
  const child = startGit(join(tmpdir(), "rerail-no-such-folder", "git"), ["--version"], process.env, "pipe");
  // events.once would reject its wait for close at the error; a listener of each takes both.
  const errored = new Promise<NodeJS.ErrnoException>(resolve => child.once("error", resolve));
  const closed = new Promise<unknown[]>(resolve => child.once("close", (...args: unknown[]) => resolve(args)));
  const [error, args] = await Promise.all([errored, closed]);
  return { code: error.code, syscall: error.syscall, pid: child.pid, closed: args };
}

describe("Git started in a thread of its own", () => {
  it("answers every read a walk and a boundary make as Git started on the asking thread answers it, and that thread starts none of them", async () => {
    const root = await repository();
    const onThisThread = await answers(root);
    vi.mocked(spawn).mockClear();
    expect(startGitInThread("win32")).toBe(true);
    const inThread = await answers(root);
    expect(inThread).toEqual(onThisThread);
    // The blocking runs still start here, as their caller waits for their end; no read that does not block does.
    expect(vi.mocked(spawn).mock.calls.length).toBe(0);
  }, 30_000);

  it("gives a reader Node's child: its input, output, stderr and end with its events in Node's order, and a kill", async () => {
    const script = "let n = 0; process.stdin.on('data', c => { n += c.length; }); process.stdin.on('end', () => { process.stdout.write(String(n)); process.stderr.write('warned'); process.exitCode = 3; });";
    const waiting = "setInterval(() => undefined, 1000);";
    const onThisThread = [await scriptRun(script, "x".repeat(300_000)), await scriptRun(waiting, "", true)];
    expect(startGitInThread("win32")).toBe(true);
    const inThread = [await scriptRun(script, "x".repeat(300_000)), await scriptRun(waiting, "", true)];
    expect(inThread).toEqual(onThisThread);
    expect(inThread[0]).toMatchObject({ stdout: "300000", stderr: "warned", code: 3, events: ["spawn", "exit"], pid: "number" });
  }, 30_000);

  it("answers a start that finds no program as Node's child does: ENOENT, no pid, then close", async () => {
    const onThisThread = await missingStart();
    expect(startGitInThread("win32")).toBe(true);
    expect(await missingStart()).toEqual(onThisThread);
    expect(onThisThread).toMatchObject({ code: "ENOENT", syscall: "spawn " + join(tmpdir(), "rerail-no-such-folder", "git"), pid: undefined });
  });

  it("is on where it is turned on on Windows, and nowhere else", () => {
    expect([startGitInThread("linux"), startGitInThread("darwin"), startGitInThread("win32")]).toEqual([false, false, true]);
  });
});

/**
 * Has the daemon, started from its built entry as `rerail daemon start` starts it, read HEAD 20 times through its own
 * Git module once it listens: the head it read, how many Git processes Node started meanwhile, and the system time
 * the daemon's main thread spent per read.
 */
async function daemonReads(root: string): Promise<{ head: string | undefined; nodeStarts: number; mainThreadSystemMs: number }> {
  const home = await folder("home");
  const out = join(home, "reads.json");
  const preload = join(home, "reads.cjs");
  const gitModule = pathToFileURL(join(packageRoot, "dist", "engine", "git.js")).href;
  await writeFile(preload, `const cp = require("node:child_process");
const fs = require("node:fs");
let nodeStarts = 0;
const spawnOf = cp.ChildProcess.prototype.spawn;
cp.ChildProcess.prototype.spawn = function (options) { if (/git/u.test(String(options.file))) nodeStarts += 1; return spawnOf.call(this, options); };
// The daemon's entry turns the engine on before it starts the daemon, which then listens: the reads start once it does.
const listen = require("node:net").Server.prototype.listen;
let reading = false;
require("node:net").Server.prototype.listen = function (...args) {
  if (!reading) this.once("listening", reads);
  reading = true;
  return listen.apply(this, args);
};
async function reads() {
  const { gitHead } = await import(${JSON.stringify(gitModule)});
  nodeStarts = 0;
  let head;
  const before = process.threadCpuUsage().system;
  for (let i = 0; i < 20; i += 1) head = await gitHead(${JSON.stringify(root)});
  const mainThreadSystemMs = (process.threadCpuUsage().system - before) / 20000;
  fs.writeFileSync(${JSON.stringify(out)}, JSON.stringify({ head, nodeStarts, mainThreadSystemMs }));
}
`);
  const env = watchedEnvironment(home);
  env.NODE_OPTIONS = `${env.NODE_OPTIONS ?? ""} --require ${JSON.stringify(preload)}`;
  const daemon = daemonProcess(home, env);
  try {
    await until(() => existsSync(out), 60_000, "the daemon's reads");
    return JSON.parse(await readFile(out, "utf8")) as Awaited<ReturnType<typeof daemonReads>>;
  } finally {
    daemon.kill("SIGKILL");
    await once(daemon, "close");
  }
}

/** What {@link daemonReadsAfterMoves} read: each read's head and the Git the daemon kept after it. */
interface MovedReads {
  before: { head: string; kept: string };
  blocking: { head: string; kept: string };
  streamed: { head: string; kept: string };
  /** The program of each Git start Node made from the first move on, in order. */
  nodeStarts: string[];
}

/**
 * Has the daemon, started from its built entry with the two folders of `copies` first on `PATH`, each holding a copy of
 * this machine's Git, read HEAD once; then move the first folder away and read HEAD with a blocking call, then move the
 * second away and read it with a call that does not block. Each move takes away the Git the daemon kept, so each call
 * starts a program that is no longer there before Git is looked for again.
 */
async function daemonReadsAfterMoves(root: string, copies: readonly string[]): Promise<MovedReads> {
  const home = await folder("home");
  const out = join(home, "moved.json");
  const preload = join(home, "moved.cjs");
  const gitModule = pathToFileURL(join(packageRoot, "dist", "engine", "git.js")).href;
  await writeFile(preload, `const cp = require("node:child_process");
const fs = require("node:fs");
const nodeStarts = [];
const noteGit = file => { if (String(file).split("/").pop() === "git") nodeStarts.push(String(file)); };
const spawnOf = cp.ChildProcess.prototype.spawn;
cp.ChildProcess.prototype.spawn = function (options) { noteGit(options.file); return spawnOf.call(this, options); };
const spawnSyncOf = cp.spawnSync;
cp.spawnSync = function (file, ...rest) { noteGit(file); return spawnSyncOf.call(this, file, ...rest); };
require("node:module").syncBuiltinESMExports();
const listen = require("node:net").Server.prototype.listen;
let reading = false;
require("node:net").Server.prototype.listen = function (...args) {
  if (!reading) this.once("listening", reads);
  reading = true;
  return listen.apply(this, args);
};
async function reads() {
  const git = await import(${JSON.stringify(gitModule)});
  const root = ${JSON.stringify(root)};
  const [first, second] = ${JSON.stringify(copies)};
  const before = { head: await git.gitHead(root), kept: git.gitExecutable() };
  nodeStarts.length = 0;
  fs.renameSync(first, first + "-gone");
  const blocking = { head: git.runGit(root, ["rev-parse", "HEAD"]).stdout.trim(), kept: git.gitExecutable() };
  fs.renameSync(second, second + "-gone");
  const streamed = { head: await git.gitHead(root), kept: git.gitExecutable() };
  fs.writeFileSync(${JSON.stringify(out)}, JSON.stringify({ before, blocking, streamed, nodeStarts }));
}
`);
  const env = watchedEnvironment(home);
  env.PATH = [...copies, ...(env.PATH === undefined ? [] : [env.PATH])].join(":");
  env.NODE_OPTIONS = `${env.NODE_OPTIONS ?? ""} --require ${JSON.stringify(preload)}`;
  const daemon = daemonProcess(home, env);
  try {
    await until(() => existsSync(out), 60_000, "the daemon's reads");
    return JSON.parse(await readFile(out, "utf8")) as MovedReads;
  } finally {
    daemon.kill("SIGKILL");
    await once(daemon, "close");
  }
}

describe.runIf(LINUX)("the daemon's process entry", () => {
  it("starts the daemon's Git through the engine: Node starts no Git, and the head read is Git's", async () => {
    const root = await repository();
    const reads = await daemonReads(root);
    const head = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    // The main thread's system time per read is a figure for the message, which decides nothing: it varies with the host.
    expect({ head: reads.head, nodeStarts: reads.nodeStarts }, JSON.stringify(reads)).toEqual({ head, nodeStarts: 0 });
  }, 90_000);

  it("finds Git again once the Git it kept is gone and starts that one through the engine too: Node starts only each start that found no program", async () => {
    const root = await repository();
    const parent = await folder("git-copies");
    const machineGit = findGit().path!;
    const copies = ["first", "second"].map(name => join(parent, name));
    for (const copy of copies) {
      await mkdir(copy);
      await copyFile(machineGit, join(copy, "git"));
    }
    const reads = await daemonReadsAfterMoves(root, copies);
    const head = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    const [first, second] = copies.map(copy => join(copy, "git"));
    expect(reads, JSON.stringify(reads)).toEqual({
      before: { head, kept: first },
      blocking: { head, kept: second },
      streamed: { head, kept: machineGit },
      nodeStarts: [first, second],
    });
  }, 90_000);
});

describe.runIf(WINDOWS)("the daemon's process entry, on Windows", () => {
  // A process start there can take seconds on a busy machine, and Node makes it on the thread that asks: a read's Git
  // started on the daemon's own thread holds every hook's answer for that long.
  it("starts the Git of the daemon's reads off the daemon's own thread: that thread starts none, and the head read is Git's", async () => {
    const root = await repository();
    const reads = await daemonReads(root);
    const head = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    expect({ head: reads.head, nodeStarts: reads.nodeStarts }, JSON.stringify(reads)).toEqual({ head, nodeStarts: 0 });
  }, 90_000);
});
