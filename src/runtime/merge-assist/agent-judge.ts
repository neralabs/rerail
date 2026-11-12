/**
 * Merge Assist's judge: runs the person's own coding agent CLI headless, with no tools, for the quick call that puts a
 * conflict in one line (docs/merge-assist-v1-build.md §9.5). Answers must match a JSON schema and are used only as text.
 * Calls are cached by a hash of their inputs and limited per repository and hour. See §6 and §8.3.
 */
import { spawn as nodeSpawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, open, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { and, count, eq, gte } from "drizzle-orm";
import { z } from "zod";
import type { RerailHost } from "../../engine/adapter-contract.js";
import { JUDGE_RUNNER_NAMES, JUDGE_RUNNERS, type JudgeReadiness, type JudgeRunnerId } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistJudgments } from "../../engine/storage/schema.js";
import { resolveExecutable } from "../npm.js";

/** Judgment calls one repository may make in a window. */
export const JUDGE_LIMIT_PER_HOUR = 40;
export const JUDGE_WINDOW_MINUTES = 60;
/** How long one call may run before it is stopped. */
export const JUDGE_TIMEOUT_MS = 120_000;
/** The most a call may write to stdout, and the largest answer file read. */
export const JUDGE_OUTPUT_LIMIT_BYTES = 1024 * 1024;
/** Grace a stopped child gets after SIGTERM before SIGKILL: the judge's, `gh`'s and the conflict hook command's. */
export const KILL_GRACE_MS = 2_000;
/** How much stderr is kept while a child runs, and how much of it a failure reports. */
const STDERR_KEEP_CHARS = 4_000;
const DETAIL_TAIL_CHARS = 300;

const RUNNER_COMMANDS: Record<JudgeRunnerId, string> = { claude: "claude", codex: "codex" };
/** The session's coding agent judges when it can; other hosts have no judge of their own and use the default order. */
const HOST_RUNNERS: Partial<Record<RerailHost, JudgeRunnerId>> = { "claude-code": "claude", codex: "codex" };

export interface JudgeTask<T> {
  /** `summary` is a conflict's status line (the conflict hook's quick call). */
  readonly kind: "summary";
  readonly repositoryKey: string;
  /** The complete prompt, untrusted data already fenced. */
  readonly prompt: string;
  readonly jsonSchema: Record<string, unknown>;
  /** Validates the answer; throws when it is invalid. */
  readonly parse: (value: unknown) => T;
  /** sha256 of the task kind and its inputs: the same key is answered from the cache, with no call and no budget. */
  readonly cacheKey: string;
}

export type JudgeOutcome<T> =
  | { readonly ok: true; readonly value: T; readonly runner: JudgeRunnerId; readonly cached: boolean }
  | { readonly ok: false; readonly reason: "unavailable" | "budget" | "timeout" | "failed" | "invalid"; readonly detail: string };

export interface AgentJudge {
  readiness(): JudgeReadiness;
  budget(repositoryKey: string): { used: number; limit: number; windowMinutes: number };
  judge<T>(task: JudgeTask<T>): Promise<JudgeOutcome<T>>;
}

export interface JudgeOptions {
  readonly database: RerailDatabase;
  /** The session's host: claude-code judges with claude, codex with codex; any other host uses the default order. */
  readonly preferredHost?: RerailHost;
  readonly env?: NodeJS.ProcessEnv;
  readonly limitPerHour?: number;
  readonly timeoutMs?: number;
  readonly spawn?: typeof nodeSpawn;
  readonly now?: () => number;
}

type Failure = Extract<JudgeOutcome<never>, { ok: false }>;
type RunOutcome = { readonly ok: true; readonly value: unknown } | Failure;

interface Detected {
  readonly readiness: JudgeReadiness;
  readonly executable: string | null;
}

/** The first runner on PATH: the preferred one, then `JUDGE_RUNNERS` in order. Looks at PATH only; runs nothing. */
function detectRunner(env: NodeJS.ProcessEnv, preferredHost: RerailHost | undefined): Detected {
  const preferred = preferredHost === undefined ? undefined : HOST_RUNNERS[preferredHost];
  const order = preferred === undefined ? [...JUDGE_RUNNERS] : [preferred, ...JUDGE_RUNNERS.filter(runner => runner !== preferred)];
  for (const runner of order) {
    const executable = resolveExecutable(RUNNER_COMMANDS[runner], env);
    if (executable !== undefined) return { readiness: { available: true, runner, problem: null }, executable };
  }
  const names = JUDGE_RUNNERS.map(runner => JUDGE_RUNNER_NAMES[runner]).join(" or ");
  const commands = JUDGE_RUNNERS.map(runner => `\`${RUNNER_COMMANDS[runner]}\``).join(" nor ");
  return {
    readiness: { available: false, runner: null, problem: `Merge Assist judges with ${names}, and neither ${commands} is on PATH.` },
    executable: null,
  };
}

export function createAgentJudge(options: JudgeOptions): AgentJudge {
  return new DatabaseAgentJudge(options);
}

class DatabaseAgentJudge implements AgentJudge {
  readonly #database: RerailDatabase;
  readonly #preferredHost: RerailHost | undefined;
  readonly #env: NodeJS.ProcessEnv;
  readonly #limit: number;
  readonly #timeoutMs: number;
  readonly #spawn: typeof nodeSpawn;
  readonly #now: () => number;
  /** Calls running now, per repository: they count against the budget before they are recorded. */
  readonly #running = new Map<string, number>();
  #detected: Detected;

  constructor(options: JudgeOptions) {
    this.#database = options.database;
    this.#preferredHost = options.preferredHost;
    this.#env = options.env ?? process.env;
    this.#limit = options.limitPerHour ?? JUDGE_LIMIT_PER_HOUR;
    this.#timeoutMs = options.timeoutMs ?? JUDGE_TIMEOUT_MS;
    this.#spawn = options.spawn ?? nodeSpawn;
    this.#now = options.now ?? Date.now;
    this.#detected = detectRunner(this.#env, this.#preferredHost);
  }

  readiness(): JudgeReadiness {
    this.#detected = detectRunner(this.#env, this.#preferredHost);
    return this.#detected.readiness;
  }

  budget(repositoryKey: string): { used: number; limit: number; windowMinutes: number } {
    return { used: this.#recordedCalls(repositoryKey), limit: this.#limit, windowMinutes: JUDGE_WINDOW_MINUTES };
  }

  async judge<T>(task: JudgeTask<T>): Promise<JudgeOutcome<T>> {
    const cached = this.#cached(task);
    if (cached !== undefined) return cached;
    const { readiness, executable } = (this.#detected = detectRunner(this.#env, this.#preferredHost));
    if (readiness.runner === null || executable === null) {
      return { ok: false, reason: "unavailable", detail: readiness.problem ?? "No agent CLI is available to judge." };
    }
    const running = this.#running.get(task.repositoryKey) ?? 0;
    if (this.#recordedCalls(task.repositoryKey) + running >= this.#limit) {
      return {
        ok: false,
        reason: "budget",
        detail: `Merge Assist has used its ${this.#limit} judgment calls for this repository in the last ${JUDGE_WINDOW_MINUTES} minutes.`,
      };
    }
    const runner = readiness.runner;
    this.#running.set(task.repositoryKey, running + 1);
    let outcome: RunOutcome;
    try {
      outcome = await this.#call(runner, executable, task);
    } finally {
      const left = (this.#running.get(task.repositoryKey) ?? 1) - 1;
      if (left === 0) this.#running.delete(task.repositoryKey);
      else this.#running.set(task.repositoryKey, left);
    }
    if (!outcome.ok) {
      this.#record(`failed:${randomUUID()}`, task.repositoryKey, `${task.kind}:failed`, runner, { reason: outcome.reason, detail: outcome.detail });
      return outcome;
    }
    let value: T;
    try {
      value = task.parse(outcome.value);
    } catch (error) {
      const invalid: Failure = { ok: false, reason: "invalid", detail: `${JUDGE_RUNNER_NAMES[runner]}'s answer did not match the schema: ${problemText(error)}` };
      // The rejected answer stays in the local judgment record only, so a person can see what the CLI said.
      this.#record(`failed:${randomUUID()}`, task.repositoryKey, `${task.kind}:failed`, runner, {
        reason: invalid.reason, detail: invalid.detail, answer: sanitizeTail(JSON.stringify(outcome.value) ?? ""),
      });
      return invalid;
    }
    this.#record(task.cacheKey, task.repositoryKey, task.kind, runner, outcome.value);
    return { ok: true, value, runner, cached: false };
  }

  /** A recorded answer for the task's key that still passes its parser; anything else is a miss. */
  #cached<T>(task: JudgeTask<T>): JudgeOutcome<T> | undefined {
    const row = this.#database.select().from(mergeAssistJudgments)
      .where(and(eq(mergeAssistJudgments.key, task.cacheKey), eq(mergeAssistJudgments.task, task.kind)))
      .get();
    if (row === undefined) return undefined;
    const runner = JUDGE_RUNNERS.find(known => known === row.runner);
    if (runner === undefined) return undefined;
    try {
      return { ok: true, value: task.parse(JSON.parse(row.resultJson)), runner, cached: true };
    } catch {
      return undefined;
    }
  }

  #recordedCalls(repositoryKey: string): number {
    const since = this.#now() - JUDGE_WINDOW_MINUTES * 60_000;
    const row = this.#database.select({ calls: count() }).from(mergeAssistJudgments)
      .where(and(eq(mergeAssistJudgments.repositoryKey, repositoryKey), gte(mergeAssistJudgments.createdAt, since)))
      .get();
    return row?.calls ?? 0;
  }

  #record(key: string, repositoryKey: string, task: string, runner: JudgeRunnerId, result: unknown): void {
    const row = { key, repositoryKey, task, runner, resultJson: JSON.stringify(result), createdAt: this.#now() };
    this.#database.insert(mergeAssistJudgments).values(row)
      .onConflictDoUpdate({ target: mergeAssistJudgments.key, set: { repositoryKey, task, runner, resultJson: row.resultJson, createdAt: row.createdAt } })
      .run();
  }

  /** One call in a fresh private directory, removed afterwards whatever happens. */
  async #call(runner: JudgeRunnerId, executable: string, task: JudgeTask<unknown>): Promise<RunOutcome> {
    const directory = await mkdtemp(join(tmpdir(), "rerail-judge-"));
    try {
      const { args, answerPath } = await prepareInvocation(runner, task.jsonSchema, directory);
      const result = await runProcess(this.#spawn, executable, args, { cwd: directory, env: this.#env, input: task.prompt, timeoutMs: this.#timeoutMs });
      return await readAnswer(runner, result, answerPath, this.#timeoutMs);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  }
}

/** The command line for `runner`; Codex reads its schema from a file and writes its answer to another. */
async function prepareInvocation(runner: JudgeRunnerId, schema: Record<string, unknown>, directory: string): Promise<{ args: string[]; answerPath: string | null }> {
  if (runner === "claude") {
    return {
      args: [
        "-p",
        "--output-format", "json",
        "--json-schema", JSON.stringify(schema),
        // No built-in tools, no MCP tools, and no MCP servers but the (empty) ones given here.
        "--tools", "",
        "--disallowedTools", "mcp__*",
        "--strict-mcp-config",
        // The person's hooks would receive the prompt, which holds other people's text. `disableAllHooks` turns off every
        // hook but managed ones for this run, above user and project settings: https://code.claude.com/docs/en/hooks.md
        // ("Disable or remove hooks") and https://code.claude.com/docs/en/settings-reference.md#disableallhooks. Only user
        // settings load besides; `--bare` would also skip hooks but ignores the person's subscription login. Checked
        // against the documentation only: claude is not installed where this was written.
        "--settings", JSON.stringify({ disableAllHooks: true }),
        "--setting-sources", "user",
        "--no-session-persistence",
        "--max-turns", "2",
      ],
      answerPath: null,
    };
  }
  const schemaPath = join(directory, "schema.json");
  const answerPath = join(directory, "answer.json");
  await writeFile(schemaPath, JSON.stringify(schema), { mode: 0o600, flag: "wx" });
  return {
    args: [
      "exec",
      "--sandbox", "read-only",
      "--skip-git-repo-check",
      "--ephemeral",
      // The person's config.toml would bring their MCP servers, hooks, profile and model.
      "--ignore-user-config",
      "--ignore-rules",
      // Without these the model keeps a read-only shell, code execution and sub-agents.
      "--disable", "shell_tool",
      "--disable", "unified_exec",
      "--disable", "code_mode_host",
      "--disable", "multi_agent",
      "--output-schema", schemaPath,
      "-o", answerPath,
      // The prompt comes on stdin.
      "-",
    ],
    answerPath,
  };
}

type ProcessResult =
  | { readonly status: "exited"; readonly code: number; readonly stdout: string; readonly stderr: string }
  | { readonly status: "timeout" | "overflow"; readonly stderr: string }
  | { readonly status: "error"; readonly message: string };

/**
 * Runs `command` without a shell, writes `input` to its stdin and closes it. More than `JUDGE_OUTPUT_LIMIT_BYTES` on
 * stdout, or `timeoutMs` passing, stops it with SIGTERM and then SIGKILL.
 */
function runProcess(
  spawnProcess: typeof nodeSpawn,
  command: string,
  args: readonly string[],
  options: { cwd: string; env: NodeJS.ProcessEnv; input: string; timeoutMs: number },
): Promise<ProcessResult> {
  const { promise, resolve } = Promise.withResolvers<ProcessResult>();
  let child: ChildProcessWithoutNullStreams;
  try {
    child = spawnProcess(command, [...args], { cwd: options.cwd, env: options.env, shell: false, stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
  } catch (error) {
    return Promise.resolve({ status: "error", message: errorMessage(error) });
  }
  const chunks: Buffer[] = [];
  let bytes = 0;
  let stderr = "";
  let stopped: "timeout" | "overflow" | undefined;
  let killer: NodeJS.Timeout | undefined;
  let settled = false;
  const finish = (result: ProcessResult): void => {
    if (settled) return;
    settled = true;
    clearTimeout(timer);
    clearTimeout(killer);
    resolve(result);
  };
  const stop = (reason: "timeout" | "overflow"): void => {
    if (stopped !== undefined) return;
    stopped = reason;
    child.kill("SIGTERM");
    killer = setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    }, KILL_GRACE_MS);
  };
  const timer = setTimeout(() => stop("timeout"), options.timeoutMs);
  child.stdout.on("data", (chunk: Buffer) => {
    if (stopped !== undefined) return;
    bytes += chunk.length;
    if (bytes > JUDGE_OUTPUT_LIMIT_BYTES) stop("overflow");
    else chunks.push(chunk);
  });
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => {
    stderr = (stderr + chunk).slice(-STDERR_KEEP_CHARS);
  });
  // A child that exits before reading its whole prompt closes the pipe; its exit says what went wrong.
  child.stdin.on("error", () => undefined);
  child.on("error", error => finish({ status: "error", message: errorMessage(error) }));
  child.on("close", code => {
    if (stopped !== undefined) finish({ status: stopped, stderr });
    else finish({ status: "exited", code: code ?? 1, stdout: Buffer.concat(chunks).toString("utf8"), stderr });
  });
  child.stdin.end(options.input);
  return promise;
}

/** The answer a finished run gave, or why it gave none. */
async function readAnswer(runner: JudgeRunnerId, result: ProcessResult, answerPath: string | null, timeoutMs: number): Promise<RunOutcome> {
  const name = JUDGE_RUNNER_NAMES[runner];
  switch (result.status) {
    case "error":
      return { ok: false, reason: "failed", detail: `${name} could not be started: ${sanitizeTail(result.message)}` };
    case "timeout":
      return { ok: false, reason: "timeout", detail: `${name} did not answer within ${Math.round(timeoutMs / 1000)} seconds and was stopped.` };
    case "overflow":
      return { ok: false, reason: "failed", detail: `${name} wrote more than ${JUDGE_OUTPUT_LIMIT_BYTES / (1024 * 1024)} MiB of output and was stopped.` };
    case "exited":
      break;
  }
  if (runner === "claude") return claudeAnswer(result);
  if (result.code !== 0) return exitFailure(name, result.code, result.stderr);
  if (answerPath === null) return { ok: false, reason: "invalid", detail: `${name} left no answer.` };
  const text = await readCapped(answerPath);
  if (text === null) return { ok: false, reason: "invalid", detail: `${name} wrote no final message.` };
  if (text === undefined) return { ok: false, reason: "failed", detail: `${name}'s answer is larger than ${JUDGE_OUTPUT_LIMIT_BYTES / (1024 * 1024)} MiB.` };
  return parseJsonAnswer(name, text);
}

/** `claude -p --output-format json` prints one result object (or, with --verbose, an array ending in one). */
const claudeResultSchema = z.object({
  type: z.literal("result"),
  subtype: z.string().optional(),
  is_error: z.boolean().optional(),
  result: z.string().optional(),
  structured_output: z.unknown().optional(),
});

function claudeAnswer(result: Extract<ProcessResult, { status: "exited" }>): RunOutcome {
  const name = JUDGE_RUNNER_NAMES.claude;
  let parsed: unknown;
  try {
    parsed = JSON.parse(result.stdout);
  } catch {
    return result.code !== 0 ? exitFailure(name, result.code, result.stderr) : { ok: false, reason: "invalid", detail: `${name} printed no JSON result.` };
  }
  const last = Array.isArray(parsed) ? parsed.findLast(item => claudeResultSchema.safeParse(item).success) : parsed;
  const message = claudeResultSchema.safeParse(last);
  if (!message.success) {
    return result.code !== 0 ? exitFailure(name, result.code, result.stderr) : { ok: false, reason: "invalid", detail: `${name} printed no result message.` };
  }
  const { data } = message;
  if (result.code !== 0 || data.is_error === true) {
    const reason = data.subtype !== undefined && data.subtype !== "success" ? data.subtype : (data.result ?? "");
    return exitFailure(name, result.code, result.stderr.trim() === "" ? reason : result.stderr);
  }
  if (data.structured_output === undefined || data.structured_output === null) {
    return { ok: false, reason: "invalid", detail: `${name} returned no structured output.` };
  }
  return { ok: true, value: data.structured_output };
}

function parseJsonAnswer(name: string, text: string): RunOutcome {
  try {
    return { ok: true, value: JSON.parse(text.trim()) };
  } catch {
    return { ok: false, reason: "invalid", detail: `${name}'s answer is not JSON.` };
  }
}

function exitFailure(name: string, code: number, output: string): Failure {
  const tail = sanitizeTail(output);
  return { ok: false, reason: "failed", detail: `${name} failed (exit ${code})${tail === "" ? "." : `: ${tail}`}` };
}

/** The file's text; null when it does not exist or is empty, undefined when it is over the output limit. */
async function readCapped(path: string): Promise<string | null | undefined> {
  let handle;
  try {
    handle = await open(path, "r");
  } catch {
    return null;
  }
  try {
    const { size } = await handle.stat();
    if (size > JUDGE_OUTPUT_LIMIT_BYTES) return undefined;
    const text = await handle.readFile({ encoding: "utf8" });
    return text.trim() === "" ? null : text;
  } finally {
    await handle.close();
  }
}

const ANSI_SEQUENCES = /\u001B\][^\u0007\u001B]*(?:\u0007|\u001B\\)|\u001B\[[0-?]*[ -/]*[@-~]|\u001B[@-Z\\-_]/gu;

/** Output fit for a person: no escape sequences or control characters, bearer and API tokens masked, the last 300 characters. */
export function sanitizeTail(text: string): string {
  const clean = text
    .replace(ANSI_SEQUENCES, "")
    .replace(/[\p{Cc}\p{Cf}]+/gu, " ")
    .replace(/\b(Bearer\s+)[\w.~+/=-]+/giu, "$1[REDACTED]")
    .replace(/\b(?:sk|ghp|gho|ghs|github_pat|xox[abp])[-_][\w-]{8,}/gu, "[REDACTED]")
    .replace(/\s+/gu, " ")
    .trim();
  return clean.length <= DETAIL_TAIL_CHARS ? clean : `…${clean.slice(-(DETAIL_TAIL_CHARS - 1))}`;
}

function problemText(error: unknown): string {
  if (error instanceof z.ZodError) {
    return sanitizeTail(error.issues.map(issue => (issue.path.length === 0 ? issue.message : `${issue.path.map(String).join(".")}: ${issue.message}`)).join("; "));
  }
  return sanitizeTail(errorMessage(error));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
