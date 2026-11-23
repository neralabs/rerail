/**
 * The `.rerail` runner's child process, started and bounded: the same Node that runs this process, on `rr-child.js`,
 * in a temporary directory of its own that holds the request, the answer and the run's outputs, removed when the
 * child ends however it ends. The child runs with a thread cap (`RERAIL_THREADS`); past its deadline, past its memory
 * budget, or when the caller aborts, it is killed. A child never outlives this process: they are killed as it exits,
 * and a child whose parent is gone ends itself (`rr-child.ts`).
 */
import type { ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { spawn, spawnSync } from "../child-process.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../package-root.js";
import { type RrBudgets, RUNNER_VARIABLES } from "./rr-budgets.js";
import type { RrChildRequest, RrChildResult, RrMemoryReport } from "./rr-messages.js";

export type RrKillReason = "deadline" | "memory" | "aborted";

/** How a child ended: with the addon's answer, killed, or failed (the addon threw, or the child did not answer). */
export type RrChildOutcome =
  | { status: "completed"; value: unknown; wallMs: number; peakRssBytes: number }
  | { status: "killed"; reason: RrKillReason; wallMs: number; peakRssBytes: number }
  | { status: "failed"; message: string };

/** Where a child runs, what bounds it, and what may stop it early. */
export interface RrChildOptions {
  cwd: string;
  budgets: RrBudgets;
  signal?: AbortSignal | undefined;
}

/** The child's request, made with `out`: an empty directory in the child's temporary directory, for a run's outputs. */
export type RrChildRequestOf = (out: string) => RrChildRequest;

/** The most of a child's stderr kept for a failure's message. */
const STDERR_TAIL = 4096;

/** The children running, each with its temporary directory. */
const live = new Map<ChildProcess, string>();
let killsOnExit = false;

/**
 * The children still running are killed as this process exits, and their temporary directories removed: none outlives
 * it. (A process killed outright runs no exit handler: its children then end themselves, `rr-child.ts`.)
 */
function killLiveOnExit(): void {
  if (killsOnExit) return;
  killsOnExit = true;
  process.once("exit", () => {
    for (const [child, directory] of live) {
      child.kill("SIGKILL");
      removeDirectory(directory);
    }
  });
}

/**
 * A child's temporary directory removed. On Windows a child just killed may still hold a file in it for a moment, so
 * removal retries, as Node's `rm` does with `maxRetries`.
 */
function removeDirectory(directory: string): void {
  rmSync(directory, { recursive: true, force: true, maxRetries: 5 });
}

/**
 * The child's Node arguments: its bundled entry where this module runs as the package ships it, or its TypeScript
 * source through `tsx` where this module runs from its source, as the tests run it.
 */
function childArguments(): string[] {
  if (!import.meta.url.endsWith(".ts")) return [join(PACKAGE_ROOT, ENTRY_POINTS.rrChild)];
  const tsx = pathToFileURL(createRequire(import.meta.url).resolve("tsx")).href;
  return ["--import", tsx, join(PACKAGE_ROOT, "src", "engine", "rr", "rr-child.ts")];
}

/**
 * This process's environment, for the child: its thread cap, its store, and none of the runner's own settings
 * (`RUNNER_VARIABLES`); every other variable reaches it as it is, the engine's own (`RERAIL_RR_LIB`, …) among them.
 *
 * With no store (`budgets.store` false, the default; `rr-budgets.ts` says why) the child runs with
 * `RERAIL_TAPE_CACHE=off`. With one, it keeps the engine's store of the checkout it runs in: the engine picks one per
 * working directory (`$TMPDIR/rerail-stores/<build>-<directory>.sqlite`, one owner process at a time), never the
 * daemon's content store, so runs of one checkout reuse each other's work and runs of two checkouts never share a
 * store. Either way a `RERAIL_TAPE_CACHE` this process has is not passed on: the runner chooses the child's store.
 */
function childEnvironment(budgets: RrBudgets): NodeJS.ProcessEnv {
  const dropped = new Set([...RUNNER_VARIABLES, "RERAIL_TAPE_CACHE"]);
  const kept = Object.entries(process.env).filter(([name]) => !dropped.has(name));
  return { ...Object.fromEntries(kept), RERAIL_THREADS: String(budgets.threads), ...(budgets.store ? {} : { RERAIL_TAPE_CACHE: "off" }) };
}

/** A temporary directory for one child: its request written, an empty `out` for the run, its result's path. */
function prepare(requestOf: RrChildRequestOf): { directory: string; args: string[]; resultFile: string } {
  const directory = mkdtempSync(join(tmpdir(), "rerail-rr-"));
  try {
    mkdirSync(join(directory, "out"));
    const requestFile = join(directory, "request.json");
    const resultFile = join(directory, "result.json");
    writeFileSync(requestFile, JSON.stringify(requestOf(join(directory, "out"))));
    return { directory, args: [...childArguments(), requestFile, resultFile], resultFile };
  } catch (error) {
    removeDirectory(directory);
    throw error;
  }
}

/** What the child wrote, or why there is nothing to read (`ended`: how the child ended). */
function readResult(resultFile: string, ended: string): RrChildResult | { failed: string } {
  if (!existsSync(resultFile)) return { failed: `the .rerail child process ${ended} without an answer` };
  const text = readFileSync(resultFile, "utf8");
  try {
    return JSON.parse(text) as RrChildResult;
  } catch {
    // A child stopped from outside (the system's out-of-memory killer) while it wrote its answer.
    return { failed: `the .rerail child process ${ended} with its answer cut short` };
  }
}

/** The outcome of a child that was not killed, from its result file. */
function finished(resultFile: string, ended: string, wallMs: number, peakRssBytes: number): RrChildOutcome {
  const result = readResult(resultFile, ended);
  if ("failed" in result) return { status: "failed", message: result.failed };
  if (!result.ok) return { status: "failed", message: result.message };
  return { status: "completed", value: result.value, wallMs, peakRssBytes: Math.max(peakRssBytes, result.peakRssBytes) };
}

/** Whether a message from the child is a memory report (`rr-child.ts`). */
function isMemoryReport(message: unknown): message is RrMemoryReport {
  return typeof message === "object" && message !== null && "rssBytes" in message && typeof message.rssBytes === "number";
}

/** One running child, watched until it ends. */
class Watched {
  readonly #started = performance.now();
  #peakRssBytes = 0;
  #reason: RrKillReason | undefined;
  #stderr = "";

  constructor(
    readonly child: ChildProcess,
    readonly budgets: RrBudgets,
  ) {}

  kill(reason: RrKillReason): void {
    this.#reason ??= reason;
    this.child.kill("SIGKILL");
  }

  /** A memory report from the child: past the budget, it is killed. */
  report(message: unknown): void {
    if (!isMemoryReport(message)) return;
    this.#peakRssBytes = Math.max(this.#peakRssBytes, message.rssBytes);
    if (message.rssBytes > this.budgets.memoryBytes) this.kill("memory");
  }

  stderr(chunk: Buffer): void {
    this.#stderr = (this.#stderr + chunk.toString("utf8")).slice(-STDERR_TAIL);
  }

  outcome(resultFile: string, code: number | null, signal: NodeJS.Signals | null): RrChildOutcome {
    const wallMs = Math.round(performance.now() - this.#started);
    if (this.#reason !== undefined) return { status: "killed", reason: this.#reason, wallMs, peakRssBytes: this.#peakRssBytes };
    const ended = `exited (${signal ?? `code ${code}`})${this.#stderr === "" ? "" : `: ${this.#stderr.trim()}`}`;
    return finished(resultFile, ended, wallMs, this.#peakRssBytes);
  }
}

/** `child` watched to its end: killed at its deadline, past its memory budget or on `signal`. */
function watch(child: ChildProcess, resultFile: string, options: RrChildOptions): Promise<RrChildOutcome> {
  const watched = new Watched(child, options.budgets);
  const deadline = setTimeout(() => watched.kill("deadline"), options.budgets.deadlineMs);
  const abort = (): void => watched.kill("aborted");
  options.signal?.addEventListener("abort", abort, { once: true });
  child.on("message", message => watched.report(message));
  child.stderr?.on("data", (chunk: Buffer) => watched.stderr(chunk));
  const { promise, resolve } = Promise.withResolvers<RrChildOutcome>();
  child.once("error", error => resolve({ status: "failed", message: `the .rerail child process did not start: ${error.message}` }));
  child.once("close", (code, signal) => resolve(watched.outcome(resultFile, code, signal)));
  return promise.finally(() => {
    clearTimeout(deadline);
    options.signal?.removeEventListener("abort", abort);
    live.delete(child);
  });
}

/** The child started on the request `requestOf` makes, and its outcome once it ends; its temporary directory is gone by then. */
export async function runChild(requestOf: RrChildRequestOf, options: RrChildOptions): Promise<RrChildOutcome> {
  if (options.signal?.aborted === true) return { status: "killed", reason: "aborted", wallMs: 0, peakRssBytes: 0 };
  const { directory, args, resultFile } = prepare(requestOf);
  try {
    killLiveOnExit();
    const child = spawn(process.execPath, args, {
      cwd: options.cwd,
      env: childEnvironment(options.budgets),
      stdio: ["ignore", "ignore", "pipe", "ipc"],
    });
    live.set(child, directory);
    return await watch(child, resultFile, options);
  } finally {
    removeDirectory(directory);
  }
}

/**
 * `runChild` for a caller that cannot wait for a promise: the child runs to its end while this process waits, so it
 * has its deadline and thread cap but no memory watch, and nothing can abort it.
 */
export function runChildSync(requestOf: RrChildRequestOf, options: Omit<RrChildOptions, "signal">): RrChildOutcome {
  const { directory, args, resultFile } = prepare(requestOf);
  try {
    const started = performance.now();
    const ran = spawnSync(process.execPath, args, {
      cwd: options.cwd,
      env: childEnvironment(options.budgets),
      stdio: ["ignore", "ignore", "pipe"],
      timeout: options.budgets.deadlineMs,
      killSignal: "SIGKILL",
    });
    const wallMs = Math.round(performance.now() - started);
    if (ran.error !== undefined && "code" in ran.error && ran.error.code === "ETIMEDOUT") return { status: "killed", reason: "deadline", wallMs, peakRssBytes: 0 };
    if (ran.error !== undefined) return { status: "failed", message: `the .rerail child process did not start: ${ran.error.message}` };
    const stderr = ran.stderr.toString("utf8").slice(-STDERR_TAIL).trim();
    return finished(resultFile, `exited (${ran.signal ?? `code ${ran.status}`})${stderr === "" ? "" : `: ${stderr}`}`, wallMs, 0);
  } finally {
    removeDirectory(directory);
  }
}
