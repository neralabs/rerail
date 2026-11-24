/**
 * Starts every Git process Rerail runs: the reads that do not block ({@link startGit}) and the runs that do
 * ({@link runGitSync}). On Linux, Node starts a child by forking the whole process (libuv's `uv_spawn` calls `fork()`
 * there, and `posix_spawn` only on macOS), and waits on the calling thread until the copy has run `exec`. Copying and
 * dropping the page tables of a daemon of several gigabytes holds its event loop for tens to hundreds of milliseconds
 * per start, and no hook of any session is answered meanwhile. In a process
 * that turns it on ({@link startGitWithoutCopying}, the daemon on Linux), Git starts through the engine's `posix_spawn`,
 * which runs the child in this process's memory until `exec` and copies nothing (`native/src/git_start.rs`). Git gets
 * the same program, arguments, environment, working folder, stdio, signal state and descriptors as Node gives it, and
 * the reader gets a child with the events, streams and kill of Node's. On Windows the daemon starts a read's Git in a
 * starter thread (`git-thread.ts`), where a slow process start holds no hook's answer. Elsewhere, and for any start
 * the engine cannot make, Git starts through `child-process.ts` as before.
 */
import type { ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import { closeSync } from "node:fs";
import { Socket } from "node:net";
import { constants } from "node:os";
import type { Readable, Writable } from "node:stream";
import { getSystemErrorName } from "node:util";
import { endProcessTree, spawn, spawnSync } from "./child-process.js";
import { endInThread, startGitOnCallingThread, threadStart } from "./git-thread.js";

/** A Git process as its readers use it: Node's child, or one the engine or the starter thread started. */
export interface GitProcess<Stderr extends Readable | null = Readable | null> extends EventEmitter {
  readonly pid: number | undefined;
  readonly exitCode: number | null;
  readonly signalCode: NodeJS.Signals | null;
  readonly stdin: Writable;
  readonly stdout: Readable;
  readonly stderr: Stderr;
  kill(signal?: NodeJS.Signals | number): boolean;
}

/** A child the engine started: its pid and this process's ends of its stdio and exit pipe (`native/src/git_start.rs`). */
export interface EngineStartedGit {
  pid: number;
  stdin: number;
  stdout: number;
  /** -1 when the child's stderr is the null device. */
  stderr: number;
  exited: number;
}

/**
 * How a reaped child ended. Neither is set when it was not reaped: it has not exited yet (`running`), or it was not this
 * process's child to reap.
 */
export interface EngineGitExit {
  code?: number | null | undefined;
  signal?: number | null | undefined;
  running?: boolean | undefined;
}

/** How a blocking run through the engine ended (`native/src/git_start/run.rs`). */
export interface EngineGitRun {
  status?: number | null | undefined;
  signal?: number | null | undefined;
  stdout: Buffer;
  stderr: Buffer;
  timedOut: boolean;
  pastMaxBuffer: boolean;
  /** The errno of a wait on the outputs that failed, after which the run ended the child as at a limit. */
  error?: number | null | undefined;
}

/** The engine's calls for it, as `native.ts` types them; absent in an engine older than this module. */
export interface EngineGitStarts {
  startGit?: (program: string, args: string[], env: string[], pipeStderr: boolean) => EngineStartedGit;
  reapGit?: (pid: number) => EngineGitExit;
  runGit?: (program: string, args: string[], env: string[], timeoutMs: number | undefined, maxBuffer: number) => EngineGitRun;
}

type Starts = Required<EngineGitStarts>;

let engine: Starts | undefined;

/**
 * Starts Git through the engine from now on in this process, on Linux, when `load` gives an engine that can. A process
 * of a few megabytes copies itself in no time, so only a large one, the daemon, turns this on; one whose engine cannot
 * be loaded starts Git as before.
 */
export function startGitWithoutCopying(load: () => EngineGitStarts, platform: NodeJS.Platform = process.platform): boolean {
  if (platform !== "linux") return false;
  try {
    const { startGit, reapGit, runGit } = load();
    if (typeof startGit === "function" && typeof reapGit === "function" && typeof runGit === "function") engine = { startGit, reapGit, runGit };
  } catch { /* No engine: Git starts through Node, as it did before. */ }
  return engine !== undefined;
}

/**
 * Starts Git through Node on the asking thread from now on in this process, as neither {@link startGitWithoutCopying}
 * nor `startGitInThread` ran.
 */
export function startGitThroughNode(): void {
  engine = undefined;
  startGitOnCallingThread();
}

/**
 * `executable` with `args` and `env`, its stdin and stdout on pipes and its stderr on one too (`"pipe"`) or on the null
 * device (`"ignore"`), as `child_process.spawn` starts it with those options.
 */
export function startGit(executable: string, args: readonly string[], env: NodeJS.ProcessEnv, stderr: "pipe"): GitProcess<Readable>;
export function startGit(executable: string, args: readonly string[], env: NodeJS.ProcessEnv, stderr: "ignore"): GitProcess<null>;
export function startGit(executable: string, args: readonly string[], env: NodeJS.ProcessEnv, stderr: "pipe" | "ignore"): GitProcess {
  const started = engine === undefined ? undefined : startedByEngine(engine, executable, args, env, stderr === "pipe");
  if (started !== undefined && engine !== undefined) return engineGit(started, engine.reapGit);
  const threaded = threadStart(executable, args, env, stderr);
  if (threaded !== undefined) return threaded;
  const child: ChildProcess = spawn(executable, [...args], { env, stdio: ["pipe", "pipe", stderr] });
  // spawn with these stdio options always gives stdin and stdout, and stderr exactly when it is "pipe".
  return child as GitProcess;
}

/**
 * Ends `child`, a Git {@link startGit} started, with every process it started (`endProcessTree`), where it still runs.
 * A child a starter thread holds is ended there, the one thread that knows whether it still runs (`git-thread.ts`).
 */
export function endGitTree(child: GitProcess): void {
  if (!endInThread(child)) endProcessTree(child);
}

/**
 * The engine's start, or undefined when Node should make it: a program named without a folder, which Node looks for on
 * the child's PATH, and any start the engine refuses or fails (a NUL byte, no memory, too many open files), which Node
 * then makes and reports as it did before.
 */
function startedByEngine(starts: Starts, executable: string, args: readonly string[], env: NodeJS.ProcessEnv, pipeStderr: boolean): EngineStartedGit | undefined {
  if (!executable.includes("/")) return undefined;
  try {
    return starts.startGit(executable, [...args], environment(env), pipeStderr);
  } catch (error) {
    return refusedByEngine(error, "startGit");
  }
}

/**
 * Undefined when `error` is the engine's own report that `call` started no process, so Node may start Git; any other
 * error is thrown on. The engine fails a call only before `posix_spawn` has started anything, and names the call first
 * (`failed` in `native/src/git_start.rs`). An error from anywhere else, such as the engine's answer failing to reach
 * JavaScript, may come once Git has started, and Git must not run a second time for it.
 */
function refusedByEngine(error: unknown, call: "startGit" | "runGit"): undefined {
  if (error instanceof Error && error.message.startsWith(`${call}: `)) return undefined;
  throw error;
}

/**
 * The engine's child as a {@link GitProcess}. When its descriptors cannot all become sockets, the error is thrown with
 * none left open and the child is reaped once it has exited: Git has started, so Node does not start it again, as
 * Node's own `spawn` throws on with its child left to end.
 */
function engineGit(started: EngineStartedGit, reap: (pid: number) => EngineGitExit): GitProcess {
  try {
    return new EngineGit(started, reap);
  } catch (error) {
    reapOnceExited(started.pid, reap, () => undefined);
    throw error;
  }
}

/** What a blocking Git run answers: `spawnSync`'s status, signal, output as UTF-8, and error. */
export interface GitRunResult {
  status: number | null;
  /** Empty for a signal Node has no name for, as `spawnSync` gives it, whatever its own type says. */
  signal: NodeJS.Signals | "" | null;
  /** Null where `spawnSync` could not start the program. */
  stdout: string | null;
  stderr: string | null;
  error?: Error;
}

/** A blocking run's limits, as `spawnSync` takes them; without a `maxBuffer`, spawnSync's 1 MiB. */
export interface GitRunLimits {
  timeout?: number | undefined;
  maxBuffer?: number | undefined;
}

const SPAWN_SYNC_MAX_BUFFER = 1024 * 1024;

/**
 * `executable` with `args` and `env`, run to its end while this thread waits, with no input, as `spawnSync` runs it
 * with `encoding: "utf8"` and `limits`: through the engine where {@link startGitWithoutCopying} turned it on, so the
 * wait is Git's run and no copy of this process; else, and for any run the engine cannot start, through `spawnSync`.
 */
export function runGitSync(executable: string, args: readonly string[], env: NodeJS.ProcessEnv, limits: GitRunLimits = {}): GitRunResult {
  const ran = engine === undefined ? undefined : ranByEngine(engine, executable, args, env, limits);
  if (ran !== undefined) return ran;
  const result = spawnSync(executable, [...args], { encoding: "utf8", ...limits, env });
  return { status: result.status, signal: result.signal, stdout: result.stdout, stderr: result.stderr, ...(result.error === undefined ? {} : { error: result.error }) };
}

/**
 * The engine's run as `spawnSync` answers it, or undefined when `spawnSync` should run Git: a program named without a
 * folder, limits the engine's parameters cannot hold ({@link engineLimits}), and a run the engine refused before it
 * started anything. What follows Git's run (its answer reaching JavaScript, its output becoming strings) is outside the
 * fallback: a throw there is thrown on, as `spawnSync`'s own throws when its output cannot become a string, and Git has
 * run once.
 */
function ranByEngine(starts: Starts, executable: string, args: readonly string[], env: NodeJS.ProcessEnv, limits: GitRunLimits): GitRunResult | undefined {
  const held = engineLimits(limits);
  if (!executable.includes("/") || held === undefined) return undefined;
  let ran: EngineGitRun;
  try {
    ran = starts.runGit(executable, [...args], environment(env), ...held);
  } catch (error) {
    return refusedByEngine(error, "runGit");
  }
  return answerOf(ran, executable, args);
}

/** The largest number the engine's `u32` parameters hold. */
const U32_MAX = 0xffff_ffff;

/** True for a number from 0 to {@link U32_MAX}, which N-API hands a `u32` parameter as it is; false for NaN. */
function inU32(limit: number): boolean {
  return limit >= 0 && limit <= U32_MAX;
}

/**
 * `limits` as the engine takes them, a time limit (undefined for none, as `spawnSync` reads 0) and an output limit, or
 * undefined when either does not fit its `u32` and `spawnSync` should take them, which runs Git with what it accepts and
 * refuses the rest as it always has. N-API would make Infinity or NaN 0 and wrap a number of 2^32 or more, so
 * `maxBuffer: Infinity`, which spawnSync takes as no limit, would end every run at its first byte; so would 0, which it
 * takes as no limit too. A time limit fits when it is an integer from 0 to 2^32 - 1 ms; an output limit when it is
 * above 0 and at most 2^32 - 1, its fraction dropped, which passes and fails the same byte counts.
 */
function engineLimits({ timeout, maxBuffer = SPAWN_SYNC_MAX_BUFFER }: GitRunLimits): [number | undefined, number] | undefined {
  const timeoutMs = engineTimeout(timeout);
  return timeoutMs === null || maxBuffer === 0 || !inU32(maxBuffer) ? undefined : [timeoutMs, maxBuffer];
}

/** `timeout` as the engine takes it: undefined for none (absent or 0), null when it is not an integer it holds. */
function engineTimeout(timeout: number | undefined): number | undefined | null {
  if (timeout === undefined || timeout === 0) return undefined;
  return Number.isInteger(timeout) && inU32(timeout) ? timeout : null;
}

/** The engine's run as `spawnSync` answers it. */
function answerOf(ran: EngineGitRun, executable: string, args: readonly string[]): GitRunResult {
  const [status, signal] = runEndOf(ran);
  const error = runError(ran, executable, args);
  return { status, signal, stdout: ran.stdout.toString("utf8"), stderr: ran.stderr.toString("utf8"), ...(error === undefined ? {} : { error }) };
}

/**
 * The error `spawnSync` gives a run its time limit, its output limit or a failed wait ended, as `spawnSync` gives one
 * whose read fails: `spawnSync <file> ETIMEDOUT`, `ENOBUFS`, or the failure's code.
 */
function runError(ran: EngineGitRun, executable: string, args: readonly string[]): Error | undefined {
  const errno = runErrno(ran);
  if (errno === undefined) return undefined;
  const [code, syscall] = [getSystemErrorName(errno), `spawnSync ${executable}`];
  return Object.assign(new Error(`${syscall} ${code}`), { errno, code, syscall, path: executable, spawnargs: [...args] });
}

/** What ended the run before its end, as libuv numbers an errno (negative), or undefined when nothing did. */
function runErrno(ran: EngineGitRun): number | undefined {
  if (ran.timedOut) return -constants.errno.ETIMEDOUT;
  if (ran.pastMaxBuffer) return -constants.errno.ENOBUFS;
  return typeof ran.error === "number" ? -ran.error : undefined;
}

/** `env` as Node hands it to a child: `NAME=value` for each variable that is set. */
function environment(env: NodeJS.ProcessEnv): string[] {
  return Object.entries(env).flatMap(([name, value]) => (value === undefined ? [] : [`${name}=${value}`]));
}

/**
 * Each signal number's name as Node reports a child's end by it: the first name `os.constants.signals` lists for it.
 * Linux gives two numbers a second name (SIGIOT for 6, SIGPOLL for 29), listed after SIGABRT and SIGIO, and Node names
 * a child's end by the first. A Map keeps the last entry for a key, so the entries go in from the last.
 */
const SIGNAL_NAMES = new Map(Object.entries(constants.signals).reverse().map(([name, number]) => [number, name as NodeJS.Signals]));

/**
 * The exit code, or the signal's name, of a reaped child, as Node's child reports them; both null when it was not this
 * process's to reap. A signal Node has no name for (the real-time ones, 34 to 64 on Linux) leaves its child exit code
 * 0 and no signal, since libuv reports exit status 0 for any end by a signal and Node keeps the code when the name is
 * empty.
 */
function endOf({ code, signal }: EngineGitExit): [number | null, NodeJS.Signals | null] {
  if (signal === undefined || signal === null) return [code ?? null, null];
  const name = SIGNAL_NAMES.get(signal);
  return name === undefined ? [0, null] : [null, name];
}

/**
 * A blocking run's status and signal as `spawnSync` reports them: no status once a signal ended it, and the signal's
 * name, empty for a signal Node has no name for.
 */
function runEndOf({ status, signal }: EngineGitRun): [number | null, NodeJS.Signals | "" | null] {
  if (signal === undefined || signal === null) return [status ?? null, null];
  return [null, SIGNAL_NAMES.get(signal) ?? ""];
}

/**
 * How long a child whose exit pipe closed while it still ran waits to be reaped again. The pipe closes early only when
 * the engine's wait on the child fails, which no measured run has met; it then still ends as Node's child ends.
 */
const REAP_AGAIN_MS = 20;

/**
 * Reaps `pid` once it has exited and hands `reaped` how it ended: at once when it has, else again every
 * {@link REAP_AGAIN_MS} until it has, as Node's child stays a running child, open to a kill, until it exits.
 */
function reapOnceExited(pid: number, reap: (pid: number) => EngineGitExit, reaped: (exit: EngineGitExit) => void): void {
  const exit = reap(pid);
  if (exit.running === true) setTimeout(reapOnceExited, REAP_AGAIN_MS, pid, reap, reaped);
  else reaped(exit);
}

/** The sockets on a started child's descriptors; `stderr` is null where its stderr is the null device. */
interface GitSockets {
  exited: Socket;
  stdin: Socket;
  stdout: Socket;
  stderr: Socket | null;
}

/**
 * A socket on each of `started`'s descriptors, as `new Socket` makes them: the exit pipe's, stdin's, stdout's, then
 * stderr's. When one cannot be made, those made are destroyed and the other descriptors closed before the error is
 * thrown on, so the child reads the end of its input, meets closed outputs and ends.
 */
function socketsOf(started: EngineStartedGit): GitSockets {
  const made: Socket[] = [];
  const on = (fd: number, readable: boolean): Socket => {
    const socket = new Socket({ fd, readable });
    made.push(socket);
    return socket;
  };
  try {
    return { exited: on(started.exited, true), stdin: on(started.stdin, false), stdout: on(started.stdout, true), stderr: started.stderr < 0 ? null : on(started.stderr, true) };
  } catch (error) {
    for (const socket of made) socket.destroy();
    for (const fd of [started.exited, started.stdin, started.stdout, started.stderr].slice(made.length)) closeOpen(fd);
    throw error;
  }
}

/** Closes `fd` when it is one (stderr's is -1 on the null device). */
function closeOpen(fd: number): void {
  if (fd >= 0) closeSync(fd);
}

/** As Node does once a child exits: output no reader took is let flow, so it ends and `close` comes. */
function flowUnread(outputs: readonly (Readable | null)[]): void {
  for (const output of outputs) if (output !== null && output.listenerCount("data") === 0) output.resume();
}

/**
 * A child the engine started, with what Node's `ChildProcess` gives a reader: its pid; stdin, stdout and stderr as
 * sockets on the same kind of descriptors; `exit` with its code or signal, then `close` once stdout and stderr have
 * closed too; `exitCode`, `signalCode` and `kill`. The engine leaves the child unreaped until this reads that it exited,
 * on this thread, so a kill before then reaches this child and no other process that took its pid.
 */
class EngineGit extends EventEmitter implements GitProcess {
  readonly pid: number;
  readonly stdin: Socket;
  readonly stdout: Socket;
  readonly stderr: Socket | null;
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;
  killed = false;
  #reaped = false;
  #closesNeeded = 1;
  #closesGot = 0;

  constructor(started: EngineStartedGit, reap: (pid: number) => EngineGitExit) {
    super();
    this.pid = started.pid;
    const { exited, stdin, stdout, stderr } = socketsOf(started);
    [this.stdin, this.stdout, this.stderr] = [stdin, stdout, stderr];
    for (const output of [this.stdout, this.stderr]) {
      if (output === null) continue;
      this.#closesNeeded += 1;
      output.on("close", () => this.#maybeClose());
    }
    // The pipe only closes; an error closes it too. Either may come while the child still runs (the engine's wait on it
    // failed): it is then reaped once it has exited, and stays open to a kill until then.
    exited.on("error", () => undefined);
    exited.on("close", () => reapOnceExited(this.pid, reap, exit => this.#exited(exit)));
    exited.resume();
    process.nextTick(() => this.emit("spawn"));
  }

  kill(signal: NodeJS.Signals | number = "SIGTERM"): boolean {
    if (this.#reaped) return false;
    try {
      process.kill(this.pid, signal);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ESRCH") this.emit("error", error);
      return false;
    }
    this.killed = true;
    return true;
  }

  #exited(exit: EngineGitExit): void {
    this.#reaped = true;
    [this.exitCode, this.signalCode] = endOf(exit);
    this.stdin.destroy();
    this.emit("exit", this.exitCode, this.signalCode);
    process.nextTick(flowUnread, [this.stdout, this.stderr]);
    this.#maybeClose();
  }

  #maybeClose(): void {
    this.#closesGot += 1;
    if (this.#closesGot === this.#closesNeeded) this.emit("close", this.exitCode, this.signalCode);
  }
}
