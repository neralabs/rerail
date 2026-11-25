/**
 * What a starter thread does (`../git-thread.ts` asks, `git-starter-thread.ts` runs it): it starts each child it is asked
 * for through `child-process.ts`, keeps it by the id the asking thread gave, relays its pid, output, error, exit and
 * close in the order Node emits them, reads its output no further ahead of the reader than {@link RELAY_WINDOW}, and
 * writes, ends, kills, closes the reading end of its stdio or ends its whole tree when asked. The thread holds the child's handle until Node has handled its exit, so whether the child still
 * runs is decided there, and a tree is ended there from a child that still runs (`endProcessTree`). The asking thread
 * starts a child the same way when a thread ended before it took the start and no other thread can take it.
 */
import type { ChildProcess } from "node:child_process";
import type { Readable } from "node:stream";
import { endProcessTree, spawn } from "../child-process.js";

/**
 * A start's claim, one Int32 shared by the asking thread and the starter thread: WAITING until the thread takes the
 * start (TAKEN) or the asking thread drops it (DROPPED). Each side moves it with `Atomics.compareExchange` from WAITING,
 * so the side that comes first holds and the other learns so at once: a start dropped is never made.
 */
export const WAITING = 0;
export const TAKEN = 1;
export const DROPPED = 2;

/** A start the asking thread asks of a starter thread. */
export interface StartAsked {
  readonly op: "start";
  readonly id: number;
  readonly file: string;
  readonly args: readonly string[];
  readonly env: Readonly<Record<string, string>>;
  readonly stderr: "pipe" | "ignore";
  readonly claim: Int32Array;
}

/**
 * What the asking thread asks of one output of a child: close its reading end (`unread`), or count `bytes` more of it
 * as taken by the reader (`took`, {@link RELAY_WINDOW}).
 */
export type OutputAsked =
  | { readonly id: number; readonly op: "unread"; readonly fd: 1 | 2 }
  | { readonly id: number; readonly op: "took"; readonly fd: 1 | 2; readonly bytes: number };

/** What the asking thread asks of a child once its start is asked. */
export type ChildAsked =
  | { readonly id: number; readonly op: "write"; readonly chunk: Uint8Array }
  | { readonly id: number; readonly op: "end" | "end-tree" }
  | { readonly id: number; readonly op: "kill"; readonly signal: NodeJS.Signals | number }
  | OutputAsked;

/**
 * The most bytes of one output a starter relays ahead of what the asking thread's reader has taken: there it stops
 * reading that output until the reader takes more. A child that prints more than its reader takes then waits on its
 * full pipe, as it waits for a reader on the asking thread, and a reader that stops at its cap (`maxBuffer`) finds the
 * child at most this far past it, however long the asking thread was busy. Four of the 64 KiB a pipe read gives.
 */
export const RELAY_WINDOW = 256 * 1024;

/** Bytes of each output a starter relayed that the asking thread has not taken yet. */
const untaken = new WeakMap<Readable, number>();

/** A Node error as a message can carry it. */
export type DescribedError = Record<string, unknown> & { message: string };

/** What a starter relays of one child. */
export type ChildRelayed =
  | { id: number; ev: "spawn"; pid: number | undefined }
  | { id: number; ev: "data"; fd: 1 | 2; chunk: Uint8Array }
  | { id: number; ev: "end"; fd: 1 | 2; error?: DescribedError }
  | { id: number; ev: "error"; error: DescribedError; code: number | null }
  | { id: number; ev: "exit" | "close"; code: number | null; signal: NodeJS.Signals | null };

/** What a starter thread posts: its children's events, and once it runs, `ready` (id 0, which names no child). */
export type Relayed = ChildRelayed | { id: 0; ev: "ready" };

/**
 * What a starter thread posts on its port: {@link Relayed}, and `ending-tree` once it has done an `end-tree`, whose
 * start of `taskkill` on Windows has returned then: the asking thread counts that start as one the thread still makes.
 */
export type ThreadPosted = Relayed | { id: number; ev: "ending-tree" };

/** Where a starter relays to: the asking thread's port, or, for a start made on the asking thread, its child. */
export type Post = (message: Relayed, transfer?: ArrayBuffer[]) => void;

/** The port a starter thread serves: `parentPort` in the thread. */
export interface StarterPort {
  postMessage(message: ThreadPosted, transfer: ArrayBuffer[]): void;
  on(event: "message", listener: (asked: StartAsked | ChildAsked) => void): unknown;
}

/** Whether `claim` moved from WAITING to `to` now; false where the other side moved it first. */
export function claimed(claim: Int32Array, to: typeof TAKEN | typeof DROPPED): boolean {
  return Atomics.compareExchange(claim, 0, WAITING, to) === WAITING;
}

/** Serves the asking thread on `port`: each start it takes, and each ask about a child it started and still holds. */
export function serveGitStarts(port: StarterPort): void {
  const children = new Map<number, ChildProcess>();
  const post: Post = (message, transfer = []) => {
    port.postMessage(message, transfer);
  };
  port.on("message", asked => {
    if (asked.op !== "start") {
      const child = children.get(asked.id);
      if (child !== undefined) applyAsked(child, asked, post);
      // Whether or not this thread still held the child: the asking thread counts each end-tree until this says so.
      if (asked.op === "end-tree") port.postMessage({ id: asked.id, ev: "ending-tree" }, []);
    } else if (claimed(asked.claim, TAKEN)) {
      const child = startAsked(asked, post);
      if (child !== undefined) children.set(asked.id, child.once("close", () => children.delete(asked.id)));
    }
  });
  post({ id: 0, ev: "ready" });
}

/**
 * Starts the child `asked` names, with the hidden window `child-process.ts` gives every start, and relays its events to
 * `post`; undefined when the start threw, which is relayed as an error and a close.
 */
export function startAsked({ id, file, args, env, stderr }: StartAsked, post: Post): ChildProcess | undefined {
  let child: ChildProcess;
  try {
    child = spawn(file, [...args], { env, stdio: ["pipe", "pipe", stderr] });
  } catch (error) {
    post({ id, ev: "error", error: described(error), code: null });
    post({ id, ev: "close", code: null, signal: null });
    return undefined;
  }
  child.on("spawn", () => post({ id, ev: "spawn", pid: child.pid }));
  child.on("error", error => post({ id, ev: "error", error: described(error), code: child.exitCode }));
  child.stdin?.on("error", () => undefined);
  relay(id, child.stdout, 1, post);
  relay(id, child.stderr, 2, post);
  child.on("exit", (code, signal) => post({ id, ev: "exit", code, signal }));
  child.on("close", (code, signal) => post({ id, ev: "close", code, signal }));
  return child;
}

/**
 * Does to `child` what `asked` asks. `end-tree` ends it with every process it started where it still runs
 * (`endProcessTree`), decided here, where Node sets the child's exit in the same step that closes its handle: a child
 * whose exit was handled is not reached by its pid, which may name another process by now.
 */
export function applyAsked(child: ChildProcess, asked: ChildAsked, post: Post): void {
  switch (asked.op) {
    case "write":
      child.stdin?.write(Buffer.from(asked.chunk));
      break;
    case "end":
      child.stdin?.end();
      break;
    case "kill":
      kill(child, asked, post);
      break;
    case "end-tree":
      endProcessTree(child);
      break;
    default:
      applyToOutput(asked.fd === 1 ? child.stdout : child.stderr, asked);
  }
}

/** Closes the reading end of `output` (`unread`), or counts what the asking thread took of it (`took`). */
function applyToOutput(output: Readable | null, asked: OutputAsked): void {
  if (output === null) return;
  if (asked.op === "unread") output.destroy();
  else owe(output, -asked.bytes);
}

/** Node's `kill` of `child`; a signal Node refuses comes back as the child's error, as the asking thread asked it. */
function kill(child: ChildProcess, { id, signal }: { id: number; signal: NodeJS.Signals | number }, post: Post): void {
  try {
    child.kill(signal);
  } catch (error) {
    post({ id, ev: "error", error: described(error), code: null });
  }
}

/**
 * Relays what `stream` (output `fd`) gives: each chunk as a copy, transferred, so no pooled buffer is detached, and
 * while less than {@link RELAY_WINDOW} of it waits untaken; then its end, or the error its read failed with, which ends
 * it too and which the asking thread's stream fails with.
 */
function relay(id: number, stream: Readable | null, fd: 1 | 2, post: Post): void {
  if (stream === null) return;
  stream.on("data", (chunk: Buffer) => {
    const copy = Uint8Array.from(chunk);
    post({ id, ev: "data", fd, chunk: copy }, [copy.buffer]);
    owe(stream, chunk.byteLength);
  });
  stream.on("end", () => post({ id, ev: "end", fd }));
  stream.on("error", error => post({ id, ev: "end", fd, error: described(error) }));
}

/**
 * Counts `bytes` more of `stream` relayed and not taken yet (fewer, when negative: the reader took them). The starter
 * reads `stream` while less than {@link RELAY_WINDOW} waits untaken, and pauses it from there.
 */
function owe(stream: Readable, bytes: number): void {
  const owed = (untaken.get(stream) ?? 0) + bytes;
  untaken.set(stream, owed);
  if (owed >= RELAY_WINDOW) stream.pause();
  else if (stream.isPaused()) stream.resume();
}

/** The fields of a Node error a reader reads, each one the error has. */
const ERROR_FIELDS = ["code", "errno", "syscall", "path", "spawnargs"] as const;

/** `error` as a message carries it: its message and the fields of {@link ERROR_FIELDS} it has. */
function described(error: unknown): DescribedError {
  if (!(error instanceof Error)) return { message: String(error) };
  const own = error as Error & Record<string, unknown>;
  return { ...Object.fromEntries(ERROR_FIELDS.filter(name => own[name] !== undefined).map(name => [name, own[name]])), message: error.message };
}
