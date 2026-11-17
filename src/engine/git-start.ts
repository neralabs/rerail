/**
 * Starts the Git processes that Rerail reads without blocking. On Linux, Node starts a child by forking the whole
 * process (libuv's `uv_spawn` calls `fork()` there, and `posix_spawn` only on macOS), and waits on the calling thread
 * until the copy has run `exec`. Copying and dropping the page tables of a daemon of several gigabytes holds its event
 * loop for tens to hundreds of milliseconds per start, and no hook of any session is answered meanwhile. In a process
 * that turns it on ({@link startGitWithoutCopying}, the daemon on Linux), Git starts through the engine's `posix_spawn`,
 * which runs the child in this process's memory until `exec` and copies nothing (`native/src/git_start.rs`). Git gets
 * the same program, arguments, environment, working folder, stdio, signal state and descriptors as Node gives it, and
 * the reader gets a child with the events, streams and kill of Node's. Elsewhere, and for any start the engine cannot
 * make, Git starts through `child-process.ts` as before.
 */
import type { ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import { Socket } from "node:net";
import { constants } from "node:os";
import type { Readable, Writable } from "node:stream";
import { spawn, spawnSync } from "./child-process.js";

/** A Git process as its readers use it: Node's child, or one the engine started. */
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

/** How a reaped child ended; neither is set when it was not this process's to reap. */
export interface EngineGitExit {
  code?: number | null | undefined;
  signal?: number | null | undefined;
}

/** How a blocking run through the engine ended (`native/src/git_start/run.rs`). */
export interface EngineGitRun {
  status?: number | null | undefined;
  signal?: number | null | undefined;
  stdout: Buffer;
  stderr: Buffer;
  timedOut: boolean;
  pastMaxBuffer: boolean;
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

/** Starts Git through Node from now on in this process, as {@link startGitWithoutCopying} never ran. */
export function startGitThroughNode(): void {
  engine = undefined;
}

/**
 * `executable` with `args` and `env`, its stdin and stdout on pipes and its stderr on one too (`"pipe"`) or on the null
 * device (`"ignore"`), as `child_process.spawn` starts it with those options.
 */
export function startGit(executable: string, args: readonly string[], env: NodeJS.ProcessEnv, stderr: "pipe"): GitProcess<Readable>;
export function startGit(executable: string, args: readonly string[], env: NodeJS.ProcessEnv, stderr: "ignore"): GitProcess<null>;
export function startGit(executable: string, args: readonly string[], env: NodeJS.ProcessEnv, stderr: "pipe" | "ignore"): GitProcess {
  const started = engine === undefined ? undefined : startedByEngine(engine, executable, args, env, stderr === "pipe");
  if (started !== undefined && engine !== undefined) return new EngineGit(started, engine.reapGit);
  const child: ChildProcess = spawn(executable, [...args], { env, stdio: ["pipe", "pipe", stderr] });
  // spawn with these stdio options always gives stdin and stdout, and stderr exactly when it is "pipe".
  return child as GitProcess;
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
  } catch {
    return undefined;
  }
}

/** What a blocking Git run answers: `spawnSync`'s status, signal, output as UTF-8, and error. */
export interface GitRunResult {
  status: number | null;
  signal: NodeJS.Signals | null;
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

function ranByEngine(starts: Starts, executable: string, args: readonly string[], env: NodeJS.ProcessEnv, limits: GitRunLimits): GitRunResult | undefined {
  if (!executable.includes("/")) return undefined;
  try {
    return answerOf(starts.runGit(executable, [...args], environment(env), limits.timeout || undefined, limits.maxBuffer ?? SPAWN_SYNC_MAX_BUFFER), executable, args);
  } catch {
    return undefined;
  }
}

/** The engine's run as `spawnSync` answers it. */
function answerOf(ran: EngineGitRun, executable: string, args: readonly string[]): GitRunResult {
  const [status, signal] = endOf({ code: ran.status, signal: ran.signal });
  const error = runError(ran, executable, args);
  return { status, signal, stdout: ran.stdout.toString("utf8"), stderr: ran.stderr.toString("utf8"), ...(error === undefined ? {} : { error }) };
}

/** The error `spawnSync` gives a run its time limit or its output limit ended: `spawnSync <file> ETIMEDOUT` or `ENOBUFS`. */
function runError(ran: EngineGitRun, executable: string, args: readonly string[]): Error | undefined {
  const code = ran.timedOut ? "ETIMEDOUT" : ran.pastMaxBuffer ? "ENOBUFS" : undefined;
  if (code === undefined) return undefined;
  const syscall = `spawnSync ${executable}`;
  return Object.assign(new Error(`${syscall} ${code}`), { errno: -constants.errno[code], code, syscall, path: executable, spawnargs: [...args] });
}

/** `env` as Node hands it to a child: `NAME=value` for each variable that is set. */
function environment(env: NodeJS.ProcessEnv): string[] {
  return Object.entries(env).flatMap(([name, value]) => (value === undefined ? [] : [`${name}=${value}`]));
}

const SIGNAL_NAMES = new Map(Object.entries(constants.signals).map(([name, number]) => [number, name as NodeJS.Signals]));

/** The exit code, or the signal's name, of a reaped child; both null when it was not this process's to reap. */
function endOf({ code, signal }: EngineGitExit): [number | null, NodeJS.Signals | null] {
  if (signal !== undefined && signal !== null) return [null, SIGNAL_NAMES.get(signal) ?? null];
  return [code ?? null, null];
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
    this.stdin = new Socket({ fd: started.stdin, readable: false });
    this.stdout = new Socket({ fd: started.stdout, readable: true });
    this.stderr = started.stderr < 0 ? null : new Socket({ fd: started.stderr, readable: true });
    for (const output of [this.stdout, this.stderr]) {
      if (output === null) continue;
      this.#closesNeeded += 1;
      output.on("close", () => this.#maybeClose());
    }
    const exited = new Socket({ fd: started.exited, readable: true });
    // The pipe only closes; an error closes it too, and the child is reaped either way.
    exited.on("error", () => undefined);
    exited.on("close", () => this.#exited(reap(this.pid)));
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
