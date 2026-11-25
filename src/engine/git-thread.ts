/**
 * Starts the Git that does not block ({@link startGitInThread}) in starter threads of their own, in a process that
 * turns it on: the daemon on Windows. Node creates a child with `CreateProcess` on the thread that asks for it, and the
 * call returns only once Windows has made the process. Where making a process takes seconds (a machine whose every
 * process start stalls in slices of about 0.83 s, 1.1 to 1.3 s at the median), each start holds the asking thread that
 * long, and the dozen or two reads a worktree asks of Git hold the daemon's event loop past the 30 s a hook waits for
 * the engine's answer: no hook, status or health probe is read meanwhile. Started here, a slow start holds only a
 * starter thread (`git-starter/git-starter.ts`); the daemon's thread hands it the program, its arguments and environment and goes
 * on answering, and gets back the child's pid, output and end as Node's child gives them. A blocking run
 * (`runGitSync`) still starts on the thread that waits for its end anyway.
 */
import { EventEmitter } from "node:events";
import { join } from "node:path";
import { Readable, Writable } from "node:stream";
import { Worker } from "node:worker_threads";
import type { ChildProcess } from "node:child_process";
import { assertProgramPath } from "./child-process.js";
import type { GitProcess } from "./git-start.js";
import { applyAsked, type ChildAsked, type ChildRelayed, claimed, DROPPED, type DescribedError, type Relayed, startAsked, type StartAsked, type ThreadPosted } from "./git-starter/git-starter.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "./package-root.js";

/**
 * A starter thread, the children it holds for this thread by id, how many starts it is still making (the end of a tree
 * among them, `endTree`), and whether it runs.
 */
interface Starter {
  readonly worker: Worker;
  readonly children: Map<number, ThreadGit>;
  next: number;
  starting: number;
  ready: boolean;
}

/**
 * The most starter threads: a new worktree's reads come as a burst of twenty starts or so, and one thread makes them one
 * after another, so a read a hook waits for waits behind the burst. Where every process start stalls in the same slice
 * of time, threads that start at once wait that one slice together: four make four starts in it. Each costs a V8
 * isolate of a few megabytes.
 */
const STARTER_THREADS = 4;

/**
 * How many starter threads in a row may fail before they run: one that fails, or that cannot be made, is replaced, as
 * its failure may pass (a moment short of memory); once this many have failed with none run between them, no thread is
 * made again until threads are turned on again, and starts go to the threads that run, or to the asking thread while
 * none runs.
 */
const MOST_FAILED_IN_A_ROW = 2;

const starters: Starter[] = [];
let on = false;
/** The starter threads that failed before they ran since one last ran ({@link MOST_FAILED_IN_A_ROW}). */
let failedInARow = 0;

/**
 * Starts Git that does not block in starter threads from now on in this process, on Windows (`platform`), where a
 * process start can hold the asking thread for seconds. Answers whether it is on.
 */
export function startGitInThread(platform: NodeJS.Platform = process.platform): boolean {
  on = platform === "win32";
  failedInARow = 0;
  return on;
}

/** Starts Git on the asking thread from now on, as {@link startGitInThread} never ran; threads started stay for the life of the process. */
export function startGitOnCallingThread(): void {
  on = false;
}

/**
 * `executable` started in a starter thread with Node's stdio for a read (stdin and stdout on pipes, stderr on one or
 * on the null device), or undefined when this process starts Git on the asking thread: it is not on, no thread can
 * start, or a value holds a NUL byte, which Node refuses at once by a throw the reader expects.
 */
export function threadStart(executable: string, args: readonly string[], env: NodeJS.ProcessEnv, stderr: "pipe" | "ignore"): GitProcess | undefined {
  if (!startsInThread([executable, ...args, ...Object.entries(env).flat()])) return undefined;
  assertProgramPath([executable, args, {}]);
  const thread = starterFor();
  if (thread === undefined) return undefined;
  const strings = Object.fromEntries(Object.entries(env).filter((entry): entry is [string, string] => entry[1] !== undefined));
  return new ThreadGit({ op: "start", file: executable, args: [...args], env: strings, stderr }, thread);
}

/** Whether a start of these values goes to a starter thread: threads are on, and no value holds NUL. */
function startsInThread(values: readonly (string | undefined)[]): boolean {
  return on && !values.some(value => value?.includes("\0") === true);
}

/** What a start asks for, apart from the id and claim of the thread asked to make it. */
type Start = Omit<StartAsked, "id" | "claim">;

/**
 * Asks `thread` to make `start` for `child`, under the thread's next id and a claim of its own: the thread counts it as
 * a start it is still making, and keeps this process up while it holds a child. Answers the start as asked.
 */
function ask(thread: Starter, start: Start, child: ThreadGit): StartAsked {
  const asked: StartAsked = { ...start, id: thread.next, claim: new Int32Array(new SharedArrayBuffer(4)) };
  thread.next += 1;
  thread.starting += 1;
  thread.children.set(asked.id, child);
  thread.worker.ref();
  thread.worker.postMessage(asked);
  return asked;
}

/**
 * Ends `child` with every process it started, in the starter thread that holds it (`end-tree`), when it is one this
 * module started; false for any other child, which its caller ends itself.
 */
export function endInThread(child: GitProcess): boolean {
  if (!(child instanceof ThreadGit)) return false;
  child.endTree();
  return true;
}

/**
 * The thread a start goes to while threads are on: one starting nothing, else a new one while fewer than
 * {@link STARTER_THREADS} run and fewer than {@link MOST_FAILED_IN_A_ROW} failed in a row, else the one with the fewest
 * starts waiting. Undefined when threads are off, or none runs and none can start. A thread made before a failure may
 * not have run yet and fail the same way: a start it holds then moves on once more, and its failure counts too.
 */
function starterFor(): Starter | undefined {
  if (!on) return undefined;
  const idle = starters.find(thread => thread.starting === 0);
  if (idle !== undefined) return idle;
  const made = failedInARow < MOST_FAILED_IN_A_ROW && starters.length < STARTER_THREADS ? startStarter() : undefined;
  return made ?? starters.reduce<Starter | undefined>((least, thread) => (least === undefined || thread.starting < least.starting ? thread : least), undefined);
}

/**
 * The starter thread's entry: as the package ships it, or, where this module runs from its TypeScript source as the
 * tests run it, that source through tsx.
 */
function starterWorker(): Worker {
  if (!import.meta.url.endsWith(".ts")) return new Worker(join(PACKAGE_ROOT, ENTRY_POINTS.gitStarter));
  return new Worker(new URL("./git-starter/git-starter-thread.ts", import.meta.url), { execArgv: ["--import", "tsx"] });
}

/**
 * A starter thread, started now; undefined when this process cannot start a thread, which counts as a thread that
 * failed before it ran, and Git starts as before.
 */
function startStarter(): Starter | undefined {
  let worker: Worker;
  try {
    worker = starterWorker();
  } catch (error) {
    failedBeforeRunning(error instanceof Error ? error : new Error(String(error)));
    return undefined;
  }
  const made: Starter = { worker, children: new Map(), next: 1, starting: 0, ready: false };
  worker.unref();
  worker.on("message", (relayed: ThreadPosted) => {
    if (relayed.ev === "ending-tree") made.starting -= 1;
    else if (relayed.ev !== "ready") made.children.get(relayed.id)?.relayed(relayed);
    else {
      made.ready = true;
      failedInARow = 0;
    }
  });
  const lost = (error: Error): void => {
    threadLost(made, error);
  };
  worker.on("error", lost);
  worker.on("exit", code => lost(new Error(`Rerail's Git starter thread ended (exit ${code})`)));
  starters.push(made);
  return made;
}

/**
 * `thread` failed or ended, as its `error` and then its `exit` say: at the first of the two no start goes to it again,
 * and each child it held ends as Node's child does when its process is gone, or, when the thread never took its start,
 * starts in another thread or on this one ({@link ThreadGit.lost}). A thread that failed before it ran counts toward
 * {@link MOST_FAILED_IN_A_ROW} first, so the start it held makes no new thread past that. Each loss is a line on
 * stderr, the daemon's log, with its error.
 */
function threadLost(thread: Starter, error: Error): void {
  const at = starters.indexOf(thread);
  if (at < 0) return;
  starters.splice(at, 1);
  if (thread.ready) process.stderr.write(`rerail: lost a Git starter thread: ${error.message}\n`);
  else failedBeforeRunning(error);
  const children = [...thread.children.values()];
  thread.children.clear();
  for (const child of children) child.lost(error);
}

/**
 * Counts a starter thread that failed before it ran toward {@link MOST_FAILED_IN_A_ROW}, and says so on stderr, the
 * daemon's log, with its error; at the count that gives threads up, it says that too, so a log shows why Git starts on
 * the asking thread again.
 */
function failedBeforeRunning(error: Error): void {
  failedInARow += 1;
  process.stderr.write(`rerail: a Git starter thread failed before it ran: ${error.message}\n`);
  if (failedInARow !== MOST_FAILED_IN_A_ROW) return;
  process.stderr.write(`rerail: no Git starter thread is made again: ${MOST_FAILED_IN_A_ROW} in a row failed before they ran, so Git starts on the asking thread while none runs\n`);
}

/** A Node error as the starter thread described it. */
function errorOf(described: DescribedError): Error {
  const { message, ...fields } = described;
  return Object.assign(new Error(message), Object.fromEntries(Object.entries(fields).filter(([, value]) => value !== undefined)));
}

/** As Node does once a child exits: output no reader took is let flow, so it ends and `close` comes. */
function flowUnread(outputs: readonly (Readable | null)[]): void {
  for (const output of outputs) if (output !== null && output.listenerCount("data") === 0) output.resume();
}

/**
 * A child the starter thread started, with what Node's `ChildProcess` gives a reader: its pid once started; stdin,
 * stdout and stderr as streams; `spawn`, `error`, `exit` with its code or signal, then `close` once stdout and stderr
 * have ended too; `exitCode`, `signalCode` and `kill`. Each call made before the child exists waits in the thread's
 * order, after its start. `kill` answers true for any child whose exit this thread has not seen, before the starter
 * has tried, and a signal Node refuses comes back as an `error` event; no reader reads its answer. A start its reader
 * ends before the thread took it ({@link endTree}) is never made; one the thread never took because it ended starts
 * in another thread, or on this one when none can take it ({@link lost}).
 */
class ThreadGit extends EventEmitter implements GitProcess {
  pid: number | undefined;
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;
  killed = false;
  readonly stdin: Writable;
  readonly stdout: Readable;
  readonly stderr: Readable | null;
  /** The start as the thread that holds this child was asked for it, and that thread. */
  #asked: StartAsked;
  #thread: Starter;
  /** What was asked of the child before its thread answered its start, replayed should it start elsewhere. */
  #replay: ChildAsked[] = [];
  /** The child started on this thread, once its thread ended before it took the start and no other could take it. */
  #here: ChildProcess | undefined;
  /** Bytes of each output handed to its stream that the thread has not been told were taken (`#took`). */
  #untaken: Record<1 | 2, number> = { 1: 0, 2: 0 };
  #closesNeeded = 1;
  #closesGot = 0;
  #done = false;
  #startOver = false;

  constructor(start: Start, thread: Starter) {
    super();
    this.stdin = new Writable({
      write: (chunk: Buffer | string, encoding: BufferEncoding, written: () => void) => {
        // A copy of the bytes written alone: a Buffer from Node's pool would carry the whole pool to the thread. It is not
        // transferred, as a write before the thread took the start is kept to hand to another thread ({@link lost}).
        this.#post({ id: this.#asked.id, op: "write", chunk: Uint8Array.from(typeof chunk === "string" ? Buffer.from(chunk, encoding) : chunk) });
        written();
      },
      final: (ended: () => void) => {
        this.#post({ id: this.#asked.id, op: "end" });
        ended();
      },
    });
    this.stdout = this.#output(1);
    this.stderr = start.stderr === "pipe" ? this.#output(2) : null;
    [this.#asked, this.#thread] = [ask(thread, start, this), thread];
  }

  kill(signal: NodeJS.Signals | number = "SIGTERM"): boolean {
    if (this.#done || this.exitCode !== null || this.signalCode !== null) return false;
    this.#post({ id: this.#asked.id, op: "kill", signal });
    this.killed = true;
    return true;
  }

  /**
   * Ends the child with every process it started. A start the thread has not taken yet is dropped: it is never made,
   * and the child ends at once as a start that failed. Else the thread that holds the child decides whether it still
   * runs and ends its tree from it, after the starts asked before it: on Windows it starts `taskkill`, as slowly as any
   * start, so the thread counts that as a start it makes until it says it returned (`ending-tree`), and a new start
   * goes to a thread that makes none.
   */
  endTree(): void {
    if (this.#done) return;
    if (this.#here === undefined && claimed(this.#asked.claim, DROPPED)) process.nextTick(() => this.#dropped());
    else {
      if (this.#here === undefined) this.#thread.starting += 1;
      this.#post({ id: this.#asked.id, op: "end-tree" });
    }
  }

  /** One event of this child's, as the starter thread relayed it. */
  relayed(event: ChildRelayed): void {
    if (event.ev === "spawn") {
      this.#started();
      this.pid = event.pid;
      this.emit("spawn");
    } else if (event.ev === "data") this.#feed(event.fd, Buffer.from(event.chunk.buffer, event.chunk.byteOffset, event.chunk.byteLength));
    else if (event.ev === "end") this.#outputEnded(event.fd, event.error);
    else if (event.ev === "error") this.#failed(event.error, event.code);
    else if (event.ev === "exit") this.#exited(event.code, event.signal);
    else this.#closed();
  }

  /**
   * Its thread failed or ended before this child closed. A start the thread never took starts in another thread, or
   * on this one when none can take it, with what was asked of it so far. A child the thread had started ends as Node's
   * child ends: the thread's error, then `exit` with no code and no signal unless it had exited, then `close`. Its
   * process is no longer held by any handle of this process, so nothing ends it by its pid.
   */
  lost(error: Error): void {
    if (this.#done) return;
    if (claimed(this.#asked.claim, DROPPED)) this.#startElsewhere();
    // A start its reader dropped ends on its own ({@link endTree}).
    else if (Atomics.load(this.#asked.claim, 0) !== DROPPED) this.#ended(error);
  }

  #post(asked: ChildAsked): void {
    if (this.#here !== undefined) applyAsked(this.#here, asked, event => this.#relayedHere(event));
    else {
      if (!this.#startOver) this.#replay.push(asked);
      this.#thread.worker.postMessage(asked);
    }
  }

  /**
   * Asks the thread a new start goes to for the start its lost thread never took, or, when none can take it, starts
   * the child on this thread as a starter thread would have; then does what was asked of it so far.
   */
  #startElsewhere(): void {
    const replay = this.#replay;
    this.#replay = [];
    const thread = starterFor();
    if (thread === undefined) this.#here = startAsked(this.#asked, event => this.#relayedHere(event));
    else [this.#asked, this.#thread] = [ask(thread, this.#asked, this), thread];
    for (const asked of replay) this.#post({ ...asked, id: this.#asked.id });
  }

  /** An event of the child started on this thread ({@link lost}), handled as the thread's relay of it. */
  #relayedHere(event: Relayed): void {
    if (event.ev !== "ready") this.relayed(event);
  }

  /** Its thread is done starting it (it runs, or its start failed): the thread counts one start fewer waiting. */
  #started(): void {
    if (this.#startOver) return;
    this.#startOver = true;
    this.#replay = [];
    this.#thread.starting -= 1;
  }

  #failed(error: DescribedError, code: number | null): void {
    if (this.pid === undefined) this.#started();
    // A start that failed has its code already, as Node's child has it when it emits the error (ENOENT's -4058 on
    // Windows, -2 elsewhere).
    if (typeof code === "number") this.exitCode = code;
    this.emit("error", errorOf(error));
  }

  #exited(code: number | null, signal: NodeJS.Signals | null): void {
    [this.exitCode, this.signalCode] = [code, signal];
    this.emit("exit", code, signal);
    process.nextTick(flowUnread, [this.stdout, this.stderr]);
  }

  /** Its thread is gone with the child it started: the thread's error, `exit` unless it came, then `close`. */
  #ended(error: Error): void {
    this.emit("error", error);
    if (this.exitCode === null && this.signalCode === null) this.#exited(null, null);
    this.#closed();
  }

  /** Its start was dropped before the thread took it: no process was made, and its reader is told so. */
  #dropped(): void {
    this.emit("error", new Error("Git was not started: its read was ended before its start's turn"));
    this.#closed();
  }

  /** Hands `chunk` (null: the end) to output `fd`, unless its reader destroyed it. */
  #feed(fd: 1 | 2, chunk: Buffer | null): void {
    const output = fd === 1 ? this.stdout : this.stderr;
    if (output === null || output.destroyed) return;
    this.#untaken[fd] += chunk?.byteLength ?? 0;
    output.push(chunk);
  }

  /**
   * Output `fd` ended in the thread: its stream ends, or, where its read failed there, fails with that error as the
   * stream of Node's child fails, so no reader takes the output so far for the whole of it.
   */
  #outputEnded(fd: 1 | 2, error: DescribedError | undefined): void {
    if (error === undefined) this.#feed(fd, null);
    else (fd === 1 ? this.stdout : this.stderr)?.destroy(errorOf(error));
  }

  /**
   * Tells the thread that the reader took what output `fd` was handed so far: Node asks a stream for more (`read`) once
   * its reader has taken what it held, and the thread reads that output no further ahead than `RELAY_WINDOW`.
   */
  #took(fd: 1 | 2): void {
    const bytes = this.#untaken[fd];
    if (bytes === 0 || this.#done) return;
    this.#untaken[fd] = 0;
    this.#post({ id: this.#asked.id, op: "took", fd, bytes });
  }

  /** An output stream fed by the thread; destroyed unread, it closes the thread's end too, as Node's pipe closes. */
  #output(fd: 1 | 2): Readable {
    const output = new Readable({
      read: () => {
        this.#took(fd);
      },
      destroy: (error: Error | null, destroyed: (error: Error | null) => void) => {
        if (!this.#done) this.#post({ id: this.#asked.id, op: "unread", fd });
        destroyed(error);
      },
    });
    output.on("close", () => this.#maybeClose());
    this.#closesNeeded += 1;
    return output;
  }

  /** The thread's child closed: outputs that never ended end now, and `close` comes once they have. */
  #closed(): void {
    if (this.#done) return;
    this.#done = true;
    this.#started();
    this.#thread.children.delete(this.#asked.id);
    if (this.#thread.children.size === 0) this.#thread.worker.unref();
    for (const output of [this.stdout, this.stderr]) if (output !== null && !output.readableEnded) output.push(null);
    process.nextTick(flowUnread, [this.stdout, this.stderr]);
    this.#maybeClose();
  }

  #maybeClose(): void {
    this.#closesGot += 1;
    if (this.#closesGot === this.#closesNeeded) this.emit("close", this.exitCode, this.signalCode);
  }
}
