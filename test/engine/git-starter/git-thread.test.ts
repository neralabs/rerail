import { execFile as nodeExecFile, execFileSync } from "node:child_process";
import { once } from "node:events";
import { existsSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { promisify } from "node:util";
import type { Worker, WorkerOptions } from "node:worker_threads";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

/**
 * Every thread this file's modules start, and what each start in a starter thread runs first: `preload`, which makes a
 * start wait where its environment asks (RERAIL_TEST_START_MARK, a file written as the start begins;
 * RERAIL_TEST_START_WAIT, files of which one must exist before Node makes the process), and `failing`, which throws
 * before the thread runs, in each of the next `failNext` threads made. Each of the next `refuseNext` threads asked for
 * is not made: its constructor throws, as Node's does when it cannot start a thread, and `refused` counts them.
 */
const threads = vi.hoisted(() => ({ made: [] as { worker: Worker; entry: string }[], preload: "", failing: "", failNext: 0, refuseNext: 0, refused: 0 }));

vi.mock("node:worker_threads", async importOriginal => {
  const actual = await importOriginal<typeof import("node:worker_threads")>();
  class Recorded extends actual.Worker {
    constructor(entry: string | URL, options: WorkerOptions = {}) {
      if (threads.refuseNext > 0) {
        [threads.refuseNext, threads.refused] = [threads.refuseNext - 1, threads.refused + 1];
        throw new Error("no thread can start now");
      }
      const fails = threads.failNext > 0;
      threads.failNext -= fails ? 1 : 0;
      const preloads = [threads.preload, fails ? threads.failing : ""].filter(path => path !== "").flatMap(path => ["--require", path]);
      super(entry, { ...options, execArgv: [...(options.execArgv ?? []), ...preloads] });
      threads.made.push({ worker: this, entry: String(entry) });
    }
  }
  return { ...actual, Worker: Recorded, default: { ...actual, Worker: Recorded } };
});
vi.mock("../../../src/engine/child-process.js", { spy: true });
vi.mock("../../../src/engine/git-start.js", { spy: true });

import { endProcessTree, spawn } from "../../../src/engine/child-process.js";
import { execFileInThread } from "../../../src/engine/git-starter/exec-in-thread.js";
import { runGitAsync } from "../../../src/engine/git.js";
import { startGit, startGitThroughNode, type GitProcess } from "../../../src/engine/git-start.js";
import { startGitInThread } from "../../../src/engine/git-thread.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { until } from "../../support/processes/daemon-home.js";

/**
 * The preload: a start whose environment names files waits, without a turn of the thread's loop, as Node's start does,
 * and its mark holds the id of the thread that made it; with RERAIL_TEST_OUTPUT_FAILS its stdout's read fails. While
 * `ends-held` exists beside the preload, each end of a tree a thread makes waits there the same way (`endsHeld`).
 */
const PRELOAD = String.raw`const cp = require("node:child_process");
const { existsSync, writeFileSync } = require("node:fs");
const { basename, join } = require("node:path");
const { threadId } = require("node:worker_threads");
const spawnOf = cp.spawn;
const nap = new Int32Array(new SharedArrayBuffer(4));
const endsHeld = join(__dirname, "ends-held");
function holdEnd() {
  if (!existsSync(endsHeld)) return;
  writeFileSync(endsHeld + ".holding", "");
  const until = Date.now() + 20000;
  while (existsSync(endsHeld) && Date.now() < until) Atomics.wait(nap, 0, 0, 10);
}
cp.spawn = function spawn(file, args, options) {
  if (/^taskkill/i.test(basename(String(file)))) holdEnd();
  const env = (options && options.env) || {};
  if (env.RERAIL_TEST_START_MARK) writeFileSync(env.RERAIL_TEST_START_MARK, String(threadId));
  const wait = env.RERAIL_TEST_START_WAIT ? env.RERAIL_TEST_START_WAIT.split(${JSON.stringify(delimiter)}) : [];
  const until = Date.now() + 20000;
  while (wait.length > 0 && !wait.some(path => existsSync(path)) && Date.now() < until) Atomics.wait(nap, 0, 0, 10);
  if (wait.length > 0 && !wait.some(path => existsSync(path))) writeFileSync(wait[0] + ".gave-up", "");
  const child = spawnOf.apply(this, arguments);
  if (env.RERAIL_TEST_OUTPUT_FAILS) process.nextTick(() => child.stdout.destroy(Object.assign(new Error("read EIO"), { code: "EIO" })));
  return child;
};
// Off Windows a tree's end is the child's kill; on Windows it is taskkill's start, held above.
const killOf = cp.ChildProcess.prototype.kill;
cp.ChildProcess.prototype.kill = function kill() {
  holdEnd();
  return killOf.apply(this, arguments);
};
require("node:module").syncBuiltinESMExports();
`;

/** The file whose being there holds each end of a tree a starter thread makes, and the file the first end held writes. */
function endsHeld(): { held: string; holding: string } {
  const held = join(dirname(threads.preload), "ends-held");
  return { held, holding: `${held}.holding` };
}

const MIB = 1024 * 1024;

/**
 * A program that prints 64 MiB in 64 KiB writes, each once the one before it was taken, and after each write notes in
 * the file its first argument names how much it has printed; it ends once its output is closed.
 */
const PRINTER = String.raw`const { writeFileSync } = require("node:fs");
const noted = process.argv[2];
const chunk = Buffer.alloc(64 * 1024, 120);
let printed = 0;
function print() {
  while (printed < 64 * 1024 * 1024) {
    const more = process.stdout.write(chunk);
    printed += chunk.length;
    writeFileSync(noted, String(printed));
    if (!more) return void process.stdout.once("drain", print);
  }
}
process.stdout.on("error", () => process.exit(0));
print();
`;

const folders: string[] = [];

beforeAll(async () => {
  const preloads = await folder("preload");
  [threads.preload, threads.failing] = [join(preloads, "starts.cjs"), join(preloads, "fails.cjs")];
  await writeFile(threads.preload, PRELOAD);
  await writeFile(threads.failing, 'throw new Error("this starter thread cannot run");\n');
});

afterEach(() => {
  startGitThroughNode();
  threads.failNext = 0;
  threads.refuseNext = 0;
  vi.mocked(spawn).mockClear();
  vi.mocked(endProcessTree).mockClear();
  vi.unstubAllEnvs();
});

afterAll(async () => {
  await Promise.all(folders.splice(0).map(async made => await removeFolder(made)));
});

async function folder(name: string): Promise<string> {
  const made = await mkdtemp(join(tmpdir(), `rerail-git-thread-${name}-`));
  folders.push(made);
  return made;
}

/** A repository with one commit; answers its folder and that commit. */
async function repository(): Promise<{ root: string; head: string }> {
  const root = await folder("repo");
  const git = (...args: string[]): string => execFileSync("git", ["-C", root, ...args], { encoding: "utf8" }).trim();
  git("init", "--quiet");
  git("-c", "user.email=test@example.invalid", "-c", "user.name=test", "commit", "--quiet", "--allow-empty", "-m", "seed");
  return { root, head: git("rev-parse", "HEAD") };
}

/** `path` as Git's shell reads it in an alias, on Windows too. */
function slashed(path: string): string {
  return path.replaceAll("\\", "/");
}

/**
 * A shell loop that waits until `path` exists, or until the folder `path` is in is gone, so that a case which never
 * writes `path` (its read never answered) leaves no loop running once its folders are removed.
 */
function waitFor(path: string): string {
  return `while [ -d '${slashed(dirname(path))}' ] && [ ! -e '${slashed(path)}' ]; do sleep 0.05; done`;
}

/** Git's arguments for an alias that waits for `path` ({@link waitFor}), then exits. */
function waitingFor(path: string): string[] {
  return ["-c", `alias.wait=!${waitFor(path)}`, "wait"];
}

/** The Git the last start made, once it runs. */
async function lastStarted(): Promise<GitProcess> {
  const child = vi.mocked(startGit).mock.results.at(-1)?.value as GitProcess;
  if (child.pid === undefined) await once(child, "spawn");
  return child;
}

/** Holds this thread, with no turn of its event loop, until no process has `pid`: at most 20 s. */
function holdUntilGone(pid: number): void {
  const nap = new Int32Array(new SharedArrayBuffer(4));
  const alive = (): boolean => {
    try {
      process.kill(pid, 0);
      return true;
    } catch {
      return false;
    }
  };
  for (let waited = 0; alive() && waited < 20_000; waited += 10) Atomics.wait(nap, 0, 0, 10);
}

/** Every starter thread this file's modules started, as they started it. */
function starters(): Worker[] {
  return threads.made.filter(({ entry }) => entry.includes("git-starter-thread") || entry.includes('require("node:child_process")')).map(({ worker }) => worker);
}

/** Ends every starter thread this file started, as a thread that fails or is ended does. */
async function endStarters(): Promise<void> {
  await Promise.all(starters().map(async worker => await worker.terminate()));
}

/** What `run` writes to stderr about starter threads, the daemon's log, line by line; stderr is left as it was. */
async function loggedWhile<T>(run: () => Promise<T>): Promise<{ result: T; logged: string[] }> {
  const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  try {
    const result = await run();
    return { result, logged: written.mock.calls.map(([line]) => String(line)).filter(line => line.includes("starter thread")) };
  } finally {
    written.mockRestore();
  }
}

describe("Git started in starter threads, when a read ends or a thread does", () => {
  it("never starts the Git of a read ended while its start waits behind another's, so no process that Git would start runs", async () => {
    const { root } = await repository();
    const at = await folder("behind");
    const [release, launched, answered] = [join(at, "release"), join(at, "launched"), join(at, "answered")];
    expect(startGitInThread("win32")).toBe(true);
    const held = async (wait: string, mark = ""): Promise<number | null> => (await runGitAsync(root, ["--version"], { env: { RERAIL_TEST_START_WAIT: wait, RERAIL_TEST_START_MARK: mark } })).status;
    // Four held starts take the four threads; the read's start waits behind one of them. Three more bring each thread to
    // two, and the last start waits behind the read's in its thread until the process the read's Git starts has run or
    // the read has answered.
    const marks = [0, 1, 2, 3].map(index => join(at, `holding-${index}`));
    const first = marks.map(async mark => await held(release, mark));
    const stop = new AbortController();
    const read = runGitAsync(root, ["-c", `alias.launch=!echo started > '${slashed(launched)}'; sleep 30`, "launch"], { signal: stop.signal });
    const rest = [held(release), held(release), held(release), held([launched, answered].join(delimiter))];
    stop.abort();
    // Every thread has loaded and holds its first start before any Git starts. On Windows a process started while
    // another thread starts one inherits that one's pipe ends: tsx's esbuild service, started as a thread loads, would
    // keep a Git's output open past the read's time limit.
    await until(() => marks.every(mark => existsSync(mark)), 20_000, "the four held starts");
    await writeFile(release, "");
    const answer = await read;
    await writeFile(answered, "");
    const others = await Promise.all([...first, ...rest]);
    expect({ status: answer.status, error: answer.error?.message, launched: existsSync(launched), others, gaveUp: readdirSync(at).filter(name => name.endsWith(".gave-up")) })
      .toEqual({ status: null, error: "Git was ended: the read was stopped", launched: false, others: [0, 0, 0, 0, 0, 0, 0, 0], gaveUp: [] });
  }, 60_000);

  it("ends no process by its pid on the asking thread once the starter thread has seen the read's Git exit", async () => {
    const { root } = await repository();
    const done = join(await folder("exited"), "done");
    expect(startGitInThread("win32")).toBe(true);
    const stop = new AbortController();
    const read = runGitAsync(root, waitingFor(done), { signal: stop.signal });
    const child = await lastStarted();
    // From here to the stop this thread takes no turn: Git exits, the starter thread sees it, and its word of it waits.
    writeFileSync(done, "");
    holdUntilGone(child.pid ?? 0);
    stop.abort();
    const answer = await read;
    expect({ endedByPid: vi.mocked(endProcessTree).mock.calls.length, startedHere: vi.mocked(spawn).mock.calls.length, error: answer.error?.message })
      .toEqual({ endedByPid: 0, startedHere: 0, error: "Git was ended: the read was stopped" });
  }, 30_000);

  it("answers a read whose starter thread ended while its Git ran, and starts the next read in a new thread", async () => {
    const { root, head } = await repository();
    const done = join(await folder("lost"), "done");
    expect(startGitInThread("win32")).toBe(true);
    const read = runGitAsync(root, waitingFor(done));
    await lastStarted();
    const made = starters().length;
    // Every starter thread still running from this file's earlier cases ends too, each with the same line.
    const { logged } = await loggedWhile(endStarters);
    const answer = await Promise.race([read, new Promise<"pending">(resolve => setTimeout(() => resolve("pending"), 15_000))]);
    // That Git is held by no handle of this process now; it ends by itself.
    await writeFile(done, "");
    const next = await runGitAsync(root, ["rev-parse", "HEAD"]);
    expect({ answer: answer === "pending" ? answer : { status: answer.status, error: answer.error?.message }, next: next.stdout.trim(), startedHere: vi.mocked(spawn).mock.calls.length, newThreads: starters().length - made, logged: [...new Set(logged)] }).toEqual({
      answer: { status: null, error: "Rerail's Git starter thread ended (exit 1)" },
      next: head,
      startedHere: 0,
      newThreads: 1,
      logged: ["rerail: lost a Git starter thread: Rerail's Git starter thread ended (exit 1)\n"],
    });
  }, 60_000);

  it("replaces a starter thread that failed before it ran, and the new one starts the read it held", async () => {
    const { root, head } = await repository();
    await endStarters();
    expect(startGitInThread("win32")).toBe(true);
    const made = starters().length;
    threads.failNext = 1;
    const first = await runGitAsync(root, ["rev-parse", "HEAD"]);
    const second = await runGitAsync(root, ["rev-parse", "HEAD"]);
    expect({ heads: [first.stdout.trim(), second.stdout.trim()], startedHere: vi.mocked(spawn).mock.calls.length, newThreads: starters().length - made })
      .toEqual({ heads: [head, head], startedHere: 0, newThreads: 2 });
  }, 60_000);

  it("starts on the asking thread the reads of two threads that failed in a row before they ran, and every read after", async () => {
    const { root } = await repository();
    await endStarters();
    const expected = execFileSync("git", ["hash-object", "--stdin"], { cwd: root, input: "some text\n", encoding: "utf8" });
    threads.failNext = Number.POSITIVE_INFINITY;
    expect(startGitInThread("win32")).toBe(true);
    const made = starters().length;
    const { result: [first, second], logged } = await loggedWhile(async () => [
      await runGitAsync(root, ["hash-object", "--stdin"], { input: "some text\n" }),
      await runGitAsync(root, ["hash-object", "--stdin"], { input: "some text\n" }),
    ]);
    expect({ first: first?.stdout, second: second?.stdout, startedHere: vi.mocked(spawn).mock.calls.length, newThreads: starters().length - made, logged }).toEqual({
      first: expected,
      second: expected,
      startedHere: 2,
      newThreads: 2,
      logged: [
        "rerail: a Git starter thread failed before it ran: this starter thread cannot run\n",
        "rerail: a Git starter thread failed before it ran: this starter thread cannot run\n",
        "rerail: no Git starter thread is made again: 2 in a row failed before they ran, so Git starts on the asking thread while none runs\n",
      ],
    });
  }, 60_000);

  it("counts a starter thread that cannot be made as one that failed before it ran, and says so in the log", async () => {
    const { root, head } = await repository();
    await endStarters();
    threads.refused = 0;
    threads.refuseNext = Number.POSITIVE_INFINITY;
    expect(startGitInThread("win32")).toBe(true);
    const { result: heads, logged } = await loggedWhile(async () => {
      const read = async (): Promise<string> => (await runGitAsync(root, ["rev-parse", "HEAD"])).stdout.trim();
      return [await read(), await read(), await read()];
    });
    expect({ heads, startedHere: vi.mocked(spawn).mock.calls.length, asked: threads.refused, logged }).toEqual({
      heads: [head, head, head],
      startedHere: 3,
      asked: 2,
      logged: [
        "rerail: a Git starter thread failed before it ran: no thread can start now\n",
        "rerail: a Git starter thread failed before it ran: no thread can start now\n",
        "rerail: no Git starter thread is made again: 2 in a row failed before they ran, so Git starts on the asking thread while none runs\n",
      ],
    });
  }, 60_000);

  it("counts only the threads that failed in a row: once one has run, a thread that fails is replaced again", async () => {
    const { root, head } = await repository();
    await endStarters();
    const at = await folder("in-a-row");
    const [starting, release] = [join(at, "starting"), join(at, "release")];
    expect(startGitInThread("win32")).toBe(true);
    const made = starters().length;
    // The first thread fails and its replacement runs the read; then that thread holds a start until `release` exists.
    threads.failNext = 1;
    const first = await runGitAsync(root, ["rev-parse", "HEAD"]);
    const held = runGitAsync(root, ["rev-parse", "HEAD"], { env: { RERAIL_TEST_START_MARK: starting, RERAIL_TEST_START_WAIT: release } });
    await until(() => existsSync(starting), 20_000, "the held start");
    // The next read finds it busy and makes a thread that fails too: one failure since a thread ran, so it is replaced.
    threads.failNext = 1;
    const third = await runGitAsync(root, ["rev-parse", "HEAD"]);
    await writeFile(release, "");
    const answers = [first, await held, third].map(answer => answer.stdout.trim());
    expect({ answers, startedHere: vi.mocked(spawn).mock.calls.length, newThreads: starters().length - made })
      .toEqual({ answers: [head, head, head], startedHere: 0, newThreads: 4 });
  }, 60_000);

  it("starts in another starter thread a start that a thread which ran never took before it ended", async () => {
    const { root, head } = await repository();
    await endStarters();
    const at = await folder("untaken");
    const [starting, release] = [join(at, "starting"), join(at, "release")];
    expect(startGitInThread("win32")).toBe(true);
    const made = starters().length;
    const read = async (env: Record<string, string>): Promise<{ status: number | null; stdout: string; error: string | undefined }> => {
      const answer = await runGitAsync(root, ["rev-parse", "HEAD"], { env });
      return { status: answer.status, stdout: answer.stdout.trim(), error: answer.error?.message };
    };
    const expected = execFileSync("git", ["hash-object", "--stdin"], { cwd: root, input: "some text\n", encoding: "utf8" });
    // Four held starts take the four threads; the fifth, whose input is written and ended while it waits, waits in the
    // first thread behind the start that holds it.
    const first = read({ RERAIL_TEST_START_MARK: starting, RERAIL_TEST_START_WAIT: release });
    const others = [read({ RERAIL_TEST_START_WAIT: release }), read({ RERAIL_TEST_START_WAIT: release }), read({ RERAIL_TEST_START_WAIT: release })];
    const untaken = runGitAsync(root, ["hash-object", "--stdin"], { input: "some text\n" });
    await until(() => existsSync(starting), 20_000, "the first start");
    await starters()[made]?.terminate();
    await writeFile(release, "");
    const answers = { first: await first, others: await Promise.all(others), untaken: (await untaken).stdout };
    expect({ ...answers, startedHere: vi.mocked(spawn).mock.calls.length, newThreads: starters().length - made }).toEqual({
      first: { status: null, stdout: "", error: "Rerail's Git starter thread ended (exit 1)" },
      others: [0, 0, 0].map(status => ({ status, stdout: head, error: undefined })),
      untaken: expected,
      startedHere: 0,
      newThreads: 5,
    });
  }, 60_000);

  it("hands input written after a start moved to the child in the thread it moved to", async () => {
    const { root } = await repository();
    await endStarters();
    const at = await folder("written-after");
    const [starting, release] = [join(at, "starting"), join(at, "release")];
    expect(startGitInThread("win32")).toBe(true);
    const made = starters().length;
    const held = (env: Record<string, string>): Promise<unknown> => runGitAsync(root, ["rev-parse", "HEAD"], { env });
    // Four held starts take the four threads; the fifth waits in the first thread, behind the start that holds it.
    const others = [held({ RERAIL_TEST_START_MARK: starting, RERAIL_TEST_START_WAIT: release }), ...[1, 2, 3].map(() => held({ RERAIL_TEST_START_WAIT: release }))];
    const counter = "let n = 0; process.stdin.on('data', c => { n += c.length; }); process.stdin.on('end', () => process.stdout.write(String(n)));";
    const child = startGit(process.execPath, ["-e", counter], process.env, "pipe");
    let stdout = "";
    child.stdout.setEncoding("utf8").on("data", (chunk: string) => { stdout += chunk; });
    const closed = once(child, "close");
    await until(() => existsSync(starting), 20_000, "the first start");
    await starters()[made]?.terminate();
    // The fifth start has moved to a new thread before its input is written.
    await until(() => starters().length - made === 5, 20_000, "the start moved");
    child.stdin.end("x".repeat(1_000));
    const answered = await Promise.race([closed.then(() => stdout), new Promise(resolve => setTimeout(() => resolve("no answer in 20 s"), 20_000))]);
    await writeFile(release, "");
    await Promise.all(others);
    expect({ answered, startedHere: vi.mocked(spawn).mock.calls.length }).toEqual({ answered: "1000", startedHere: 0 });
  }, 60_000);

  it("starts on the asking thread a start its ended thread never took, once threads are turned off", async () => {
    const { root, head } = await repository();
    await endStarters();
    const at = await folder("turned-off");
    const [starting, release] = [join(at, "starting"), join(at, "release")];
    expect(startGitInThread("win32")).toBe(true);
    const made = starters().length;
    const read = async (env: Record<string, string> = {}): Promise<string> => (await runGitAsync(root, ["rev-parse", "HEAD"], { env })).stdout.trim();
    // Four held starts take the four threads; the fifth waits in the first thread, behind the start that holds it.
    const first = read({ RERAIL_TEST_START_MARK: starting, RERAIL_TEST_START_WAIT: release });
    const others = [1, 2, 3].map(async () => await read({ RERAIL_TEST_START_WAIT: release }));
    const untaken = read();
    await until(() => existsSync(starting), 20_000, "the first start");
    startGitThroughNode();
    await starters()[made]?.terminate();
    const moved = await untaken;
    await writeFile(release, "");
    await Promise.all([first, ...others]);
    expect({ moved, startedHere: vi.mocked(spawn).mock.calls.length, newThreads: starters().length - made }).toEqual({ moved: head, startedHere: 1, newThreads: 4 });
  }, 60_000);

  it("keeps the exit of a read's Git that exited before its starter thread ended, while a process it started held its output", async () => {
    const { root } = await repository();
    const at = await folder("held");
    const [go, done] = [join(at, "go"), join(at, "done")];
    expect(startGitInThread("win32")).toBe(true);
    // Git exits with 0 once `go` exists, and leaves a process it started holding its output until `done` exists.
    const holds = `(${waitFor(done)}) & ${waitFor(go)}; exit 0`;
    try {
      const read = runGitAsync(root, ["-c", `alias.hold=!${holds}`, "hold"]);
      const child = await lastStarted();
      const exits: (number | null)[] = [];
      child.on("exit", (code: number | null) => {
        exits.push(code);
      });
      const exited = once(child, "exit");
      await writeFile(go, "");
      await exited;
      await endStarters();
      const answer = await read;
      expect({ exits, exitCode: child.exitCode, status: answer.status, error: answer.error?.message })
        .toEqual({ exits: [0], exitCode: 0, status: null, error: "Rerail's Git starter thread ended (exit 1)" });
    } finally {
      await writeFile(done, "");
    }
  }, 60_000);

  it("counts a read's time limit from its Git's start, not from its wait for a thread to start it", async () => {
    const { root, head } = await repository();
    const at = await folder("limit");
    const [starting, release] = [join(at, "starting"), join(at, "release")];
    expect(startGitInThread("win32")).toBe(true);
    const read = runGitAsync(root, ["rev-parse", "HEAD"], { timeout: 2_000, env: { RERAIL_TEST_START_MARK: starting, RERAIL_TEST_START_WAIT: release } });
    await until(() => existsSync(starting), 20_000, "the start's wait");
    // Twice the read's limit on the clock, while its start waits.
    await new Promise(resolve => setTimeout(resolve, 4_000));
    await writeFile(release, "");
    const answer = await read;
    expect({ status: answer.status, stdout: answer.stdout.trim() }).toEqual({ status: 0, stdout: head });
  }, 30_000);

  it("makes two starts asked together at the same time, in two threads", async () => {
    const { root, head } = await repository();
    const at = await folder("overlap");
    const [a, b] = [join(at, "a"), join(at, "b")];
    expect(startGitInThread("win32")).toBe(true);
    // Each start waits, inside Node's start, until the other has begun: one thread making both would wait 20 s.
    const reads = await Promise.all([
      runGitAsync(root, ["rev-parse", "HEAD"], { env: { RERAIL_TEST_START_MARK: a, RERAIL_TEST_START_WAIT: b } }),
      runGitAsync(root, ["rev-parse", "HEAD"], { env: { RERAIL_TEST_START_MARK: b, RERAIL_TEST_START_WAIT: a } }),
    ]);
    expect({ heads: reads.map(read => read.stdout.trim()), gaveUp: readdirSync(at).filter(name => name.endsWith(".gave-up")) }).toEqual({ heads: [head, head], gaveUp: [] });
  }, 60_000);

  it("reads Git's output no further ahead of a busy reader than a window, so a read cut at its cap leaves Git near the cap", async () => {
    const { root } = await repository();
    const at = await folder("cap");
    const [printer, noted] = [join(at, "print.cjs"), join(at, "printed")];
    await writeFile(printer, PRINTER);
    expect(startGitInThread("win32")).toBe(true);
    const print = `alias.print=!"${slashed(process.execPath)}" "${slashed(printer)}" "${slashed(noted)}"`;
    const read = runGitAsync(root, ["-c", print, "print"], { maxBuffer: MIB });
    const child = await lastStarted();
    // A busy daemon: 1 ms of other work on this thread for each 4 KiB the read is handed, with no turn of its loop.
    const nap = new Int32Array(new SharedArrayBuffer(4));
    child.stdout.on("data", (chunk: string) => {
      Atomics.wait(nap, 0, 0, Math.ceil(Buffer.byteLength(chunk) / 4096));
    });
    const answer = await read;
    // Its reader took 1 MiB and a chunk; past that, the window, the thread's own buffer and the pipe's hold under 2 MiB.
    const printed = Number(readFileSync(noted, "utf8"));
    expect({ status: answer.status, code: (answer.error as NodeJS.ErrnoException | undefined)?.code, printed: printed <= 4 * MIB ? "at most 4 MiB" : `${printed} bytes` })
      .toEqual({ status: null, code: "ENOBUFS", printed: "at most 4 MiB" });
  }, 60_000);

  it("makes a start in another thread while one ends a read's tree, and starts in that one again once it has", async () => {
    const { root, head } = await repository();
    await endStarters();
    const at = await folder("ending");
    const [first, next, after, done] = [join(at, "first"), join(at, "next"), join(at, "after"), join(at, "done")];
    const { held, holding } = endsHeld();
    const threadOf = (mark: string): string => readFileSync(mark, "utf8");
    expect(startGitInThread("win32")).toBe(true);
    const stop = new AbortController();
    const read = runGitAsync(root, waitingFor(done), { signal: stop.signal, env: { RERAIL_TEST_START_MARK: first } });
    await lastStarted();
    // The end of the read's tree waits in its thread, as a slow start of taskkill does there on Windows.
    await writeFile(held, "");
    try {
      stop.abort();
      await until(() => existsSync(holding), 20_000, "the end of the read's tree");
      const answered = runGitAsync(root, ["rev-parse", "HEAD"], { env: { RERAIL_TEST_START_MARK: next } });
      await until(() => existsSync(next), 20_000, "the next start, made while the end waits");
      rmSync(held);
      await writeFile(done, "");
      const answers = [(await answered).stdout.trim(), (await read).error?.message];
      await runGitAsync(root, ["rev-parse", "HEAD"], { env: { RERAIL_TEST_START_MARK: after } });
      expect({ answers, nextInAnother: threadOf(next) !== threadOf(first), afterInFirst: threadOf(after) === threadOf(first) })
        .toEqual({ answers: [head, "Git was ended: the read was stopped"], nextInAnother: true, afterInFirst: true });
    } finally {
      rmSync(held, { force: true });
      rmSync(holding, { force: true });
      await writeFile(done, "");
    }
  }, 60_000);

  it("fails an output whose read failed in the starter thread, as Node's child fails it, and never ends it as whole", async () => {
    expect(startGitInThread("win32")).toBe(true);
    const child = startGit(process.execPath, ["-e", "process.stdout.write('printed')"], { ...process.env, RERAIL_TEST_OUTPUT_FAILS: "1" }, "pipe");
    const closed = once(child, "close");
    const output = new Promise<unknown>(resolve => {
      child.stdout.on("error", (error: NodeJS.ErrnoException) => resolve({ error: error.message, code: error.code }));
      child.stdout.on("end", () => resolve("ended"));
    });
    child.stdout.resume();
    child.stdin.on("error", () => undefined);
    child.stdin.end();
    expect(await output).toEqual({ error: "read EIO", code: "EIO" });
    await closed;
  }, 30_000);
});

/** How a run of `run` with Node's `-e script` and `timeoutMs` ended: its output, or the fields of its error. */
async function ranAs(run: (file: string, args: string[], timeoutMs?: number) => Promise<{ stdout: string; stderr: string }>, script: string, timeoutMs?: number): Promise<unknown> {
  try {
    const { stdout, stderr } = await run(process.execPath, ["-e", script], timeoutMs);
    return { stdout, stderr };
  } catch (error) {
    const { message, code, killed, signal, cmd, stdout, stderr } = error as NodeJS.ErrnoException & Record<string, unknown>;
    return { message, code, killed, signal, cmd, stdout, stderr };
  }
}

describe("a program run to its end in a starter thread", () => {
  it("ends as Node's execFile ends it, with its output, a failure's error and a time limit's kill, and starts on no asking thread", async () => {
    const scripts: [string, number?][] = [
      ["process.stdout.write('out'); process.stderr.write('note')"],
      ["process.stdout.write('out'); process.stderr.write('refused'); process.exit(2)"],
      // Prints nothing: what a child prints before its kill depends on how soon a loaded machine starts it.
      ["setInterval(() => undefined, 1000)", 1_000],
    ];
    const byNode = promisify(nodeExecFile);
    const node = async (file: string, args: string[], timeoutMs?: number) => await byNode(file, args, { encoding: "utf8", ...(timeoutMs === undefined ? {} : { timeout: timeoutMs }) });
    const asNodeRuns = await Promise.all(scripts.map(async ([script, timeoutMs]) => await ranAs(node, script, timeoutMs)));
    expect(startGitInThread("win32")).toBe(true);
    // A starter thread's preload writes the file a start's environment names as it makes that start; this thread has no
    // preload. Each run takes the environment as it starts, so each names a file of its own.
    const at = await folder("exec");
    const marks = scripts.map((_, index) => join(at, `started-${index}`));
    const inThread = await Promise.all(scripts.map(async ([script, timeoutMs], index) => {
      vi.stubEnv("RERAIL_TEST_START_MARK", marks[index]);
      return await ranAs(execFileInThread, script, timeoutMs);
    }));
    vi.unstubAllEnvs();
    expect({ inThread, inAThread: marks.map(mark => existsSync(mark)) }).toEqual({ inThread: asNodeRuns, inAThread: [true, true, true] });
    expect(inThread[2]).toMatchObject({ killed: true, code: null, signal: "SIGTERM" });
  }, 60_000);
});
