/**
 * The product's way to `.rerail` rules: describe rule files, and run policies over a checkout. Each call starts a
 * short-lived child process (`rr-child-process.ts`) that loads the addon: the engine's run path keeps process-wide
 * state (its thread pool, its store), and killing a process is the only sure way to bound a run's time and memory. A
 * run past a budget is reported `killed`, never thrown.
 */
import { resolve } from "node:path";
import type { RrDescription, RrFinding, RrRunRequest, RrRunResult } from "../native/native.js";
import { PACKAGE_ROOT } from "../package-root.js";
import { describeBudgets, RR_DESCRIBE_DEADLINE, type RrBudgets, rrBudgets } from "./rr-budgets.js";
import { type RrChildOutcome, type RrKillReason, runChild, runChildSync } from "./rr-child-process.js";

export type { RrDescription, RrFinding, RrRunRequest } from "../native/native.js";
export type { RrKillReason } from "./rr-child-process.js";
export type { RrBudgets } from "./rr-budgets.js";

/** A run of `.rerail` policies (absolute paths, or relative to this process's directory) over the checkout at `root`. */
export interface RrRun {
  root: string;
  policies: string[];
  /** Root-relative files the run reads alone; omitted: the whole checkout. */
  inputFiles?: string[];
  /** This run's deadline, in place of the default; `RERAIL_RR_DEADLINE_MS`, when set, still wins (`rrBudgets`). */
  deadlineMs?: number;
  signal?: AbortSignal;
}

export type RrRunOutcome =
  | { status: "completed"; findings: RrFinding[]; failures: { policy: string | null; message: string }[]; files: number; wallMs: number; peakRssBytes: number }
  | {
      status: "killed"; reason: RrKillReason; wallMs: number; peakRssBytes: number;
      /** What the run was given; `reason` names the one it passed. */
      budgets: RrBudgets;
    }
  | { status: "failed"; message: string };

/** The budgets `read` gives (`rrBudgets`, `describeBudgets`), or the error an override gives. */
function budgets(read: () => RrBudgets): RrBudgets | Error {
  try {
    return read();
  } catch (error) {
    return error instanceof Error ? error : new Error(String(error));
  }
}

/** Why a child gave no answer, in words. */
function noAnswer(outcome: Exclude<RrChildOutcome, { status: "completed" }>, budget: RrBudgets): string {
  if (outcome.status === "failed") return outcome.message;
  const limits: Record<RrKillReason, string> = {
    deadline: `it ran past its deadline of ${budget.deadlineMs} ms`,
    memory: `its memory passed ${budget.memoryBytes} bytes`,
    aborted: "it was aborted",
  };
  return `the .rerail engine was stopped: ${limits[outcome.reason]}`;
}

/**
 * The description of each of `paths`, from a child's answer, or a row for each saying why there is none. A describe
 * past its deadline says so and names the setting that raises it: the catalog leaves the pack out with that error.
 */
function descriptions(paths: string[], outcome: RrChildOutcome, budget: RrBudgets): RrDescription[] {
  if (outcome.status === "completed") return outcome.value as RrDescription[];
  const error = outcome.status === "killed" && outcome.reason === "deadline"
    ? `the engine did not finish reading it within ${budget.deadlineMs} ms, the limit for reading rule files (${RR_DESCRIBE_DEADLINE} raises it)`
    : noAnswer(outcome, budget);
  return paths.map(path => ({ path, ok: false, error }));
}

/** Describe requests with absolute paths: the child runs in another directory. */
function describeRequest(paths: string[]): () => { kind: "describe"; paths: string[] } {
  return () => ({ kind: "describe", paths: paths.map(path => resolve(path)) });
}

/**
 * What each `.rerail` file of `paths` states of itself (`describeRr`), read and checked by the engine in one child
 * process for all of them. Never rejects: a file that does not read or check, or a child that fails or is killed,
 * gives `ok: false` rows that say why.
 */
export async function describeRerailPolicies(paths: string[]): Promise<RrDescription[]> {
  const budget = budgets(describeBudgets);
  if (budget instanceof Error) return paths.map(path => ({ path, ok: false, error: budget.message }));
  return descriptions(paths, await runChild(describeRequest(paths), { cwd: PACKAGE_ROOT, budgets: budget }), budget);
}

/**
 * `describeRerailPolicies` for a caller that cannot wait for a promise: this process waits for the child, at most the
 * describe's own deadline (`describeBudgets`).
 */
export function describeRerailPoliciesSync(paths: string[]): RrDescription[] {
  const budget = budgets(describeBudgets);
  if (budget instanceof Error) return paths.map(path => ({ path, ok: false, error: budget.message }));
  return descriptions(paths, runChildSync(describeRequest(paths), { cwd: PACKAGE_ROOT, budgets: budget }), budget);
}

/** The run's request for the child, with absolute paths, writing into the child's temporary directory `out`. */
function runRequest(run: RrRun): (out: string) => { kind: "run"; run: RrRunRequest } {
  const request = { root: resolve(run.root), policies: run.policies.map(path => resolve(path)), ...(run.inputFiles === undefined ? {} : { inputFiles: run.inputFiles }) };
  return out => ({ kind: "run", run: { ...request, out } });
}

/**
 * The policies of `run` over its checkout, in a child process whose working directory is the checkout: its findings,
 * each with its span, and the policies that failed; or `killed` with the budget it passed, or `failed`.
 */
export async function runRerailPolicies(run: RrRun): Promise<RrRunOutcome> {
  const budget = budgets(() => rrBudgets(run.deadlineMs));
  if (budget instanceof Error) return { status: "failed", message: budget.message };
  const outcome = await runChild(runRequest(run), { cwd: resolve(run.root), budgets: budget, signal: run.signal });
  if (outcome.status === "killed") return { ...outcome, budgets: budget };
  if (outcome.status !== "completed") return outcome;
  const { findings, failures, files } = outcome.value as RrRunResult;
  return { status: "completed", findings, failures, files, wallMs: outcome.wallMs, peakRssBytes: outcome.peakRssBytes };
}
