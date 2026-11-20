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
import type { RrBudgets } from "./rr-budgets.js";
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

const live = new Set<ChildProcess>();
let killsOnExit = false;

/** The children still running are killed as this process exits: none outlives it. */
function killLiveOnExit(): void {
  if (killsOnExit) return;
  killsOnExit = true;
  process.once("exit", () => {
    for (const child of live) child.kill("SIGKILL");
  });
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
 * This process's environment, for the child: its thread cap, and none of the runner's own settings. A store named by
 * `RERAIL_TAPE_CACHE` is left out too: the child keeps the store of the checkout it runs in (its working directory),
 * so runs of two checkouts never share one, and every `RERAIL_*` variable is part of a run's reuse key.
 */
function childEnvironment(threads: number): NodeJS.ProcessEnv {
  const kept = Object.entries(process.env).filter(([name]) => !name.startsWith("RERAIL_RR_") && name !== "RERAIL_TAPE_CACHE");
  return { ...Object.fromEntries(kept), RERAIL_THREADS: String(threads) };
}

/** A temporary directory for one child: its request written, an empty `out` for the run, its result's path. */
function prepare(requestOf: RrChildRequestOf): { directory: string; args: string[]; resultFile: string } {
  const directory = mkdtempSync(join(tmpdir(), "rerail-rr-"));
  mkdirSync(join(directory, "out"));
  const requestFile = join(directory, "request.json");
  const resultFile = join(directory, "result.json");
  writeFileSync(requestFile, JSON.stringify(requestOf(join(directory, "out"))));
  return { directory, args: [...childArguments(), requestFile, resultFile], resultFile };
}

/** What the child wrote, or why there is nothing to read. */
function readResult(resultFile: string, ended: string): RrChildResult | { failed: string } {
  if (!existsSync(resultFile)) return { failed: `the .rerail child process ${ended} without an answer` };
  return JSON.parse(readFileSync(resultFile, "utf8")) as RrChildResult;
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
      env: childEnvironment(options.budgets.threads),
      stdio: ["ignore", "ignore", "pipe", "ipc"],
    });
    live.add(child);
    return await watch(child, resultFile, options);
  } finally {
    rmSync(directory, { recursive: true, force: true });
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
      env: childEnvironment(options.budgets.threads),
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
    rmSync(directory, { recursive: true, force: true });
  }
}
