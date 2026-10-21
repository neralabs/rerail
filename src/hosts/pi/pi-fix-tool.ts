import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { basename } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { z } from "zod/v4";
import { internalTesting } from "../../engine/internal-testing.js";

/** The tool a `subagent` steer names on Pi; Rerail registers it with the first such steer. */
export const PI_FIX_TOOL = "rerail_fix";

/** Grace a child gets after SIGTERM before it is killed. */
const KILL_GRACE_MS = 5_000;
/** How long a child may run before it is stopped and the fix fails. */
export const PI_FIX_TIMEOUT_MS = 10 * 60_000;
/** How much of a child's stderr is kept, and so how much a failed fix's result carries. */
export const STDERR_TAIL_CHARS = 2_000;
/** The longest stdout line read as an event; a longer one (a whole-session `agent_end`) is skipped unparsed. */
const MAX_EVENT_CHARS = 8 * 1024 * 1024;
/** The child's only tools: Pi's file tools, so it cannot run shell commands. */
export const PI_FIX_CHILD_TOOLS = ["read", "edit", "write", "grep", "find", "ls"] as const;

export interface PiFixUsage {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
  turns: number;
}

export interface PiFixDetails {
  exitCode: number;
  model?: string;
  usage: PiFixUsage;
}

export interface PiFixRun {
  /** The child's last assistant text. */
  text: string;
  /** Whether the child ended in failure: a nonzero exit, or a last assistant message that errored or was aborted. */
  failed: boolean;
  /** The child's own error message, when its last assistant message carried one. */
  errorMessage?: string;
  details: PiFixDetails;
}

/** The parent session a fix inherits: its model and thinking level. */
export interface PiFixParent {
  provider?: string;
  model?: string;
  thinkingLevel?: string;
}

const parameters = Type.Object({
  task: Type.String({ description: "What to fix: the Rerail steer text, the file, and the location of the violation." }),
});

/** What the child is asked to do; the task follows it. */
const FIX_PROMPT = [
  "Fix exactly the Rerail rule violation described below, with the smallest change that satisfies the rule.",
  "Do not touch unrelated code. When done, reply with one sentence describing the change.",
].join("\n");

/**
 * The child `pi` command line for `task`: Pi's own entry point when there is one, the `pi` on PATH otherwise. The child
 * loads no extensions (neither Rerail again nor the user's, headless) and has only Pi's file tools.
 */
export function piFixInvocation(
  task: string,
  parent: PiFixParent,
  execPath: string = process.execPath,
  script: string | undefined = process.argv[1],
): { command: string; args: string[] } {
  const args = ["--mode", "json", "-p", "--no-session", "--no-extensions", "--tools", PI_FIX_CHILD_TOOLS.join(",")];
  if (parent.provider !== undefined) args.push("--provider", parent.provider);
  if (parent.model !== undefined) args.push("--model", parent.model);
  if (parent.thinkingLevel !== undefined) args.push("--thinking", parent.thinkingLevel);
  args.push(`${FIX_PROMPT}\n\n${task}`);
  if (script !== undefined && !script.startsWith("/$bunfs/root/") && existsSync(script)) {
    return { command: execPath, args: [script, ...args] };
  }
  // A compiled `pi` binary is its own entry point; a bare runtime without one falls back to PATH.
  if (!/^(node|bun)(\.exe)?$/u.test(basename(execPath).toLowerCase())) return { command: execPath, args };
  return { command: "pi", args };
}

const count = z.number().catch(0);
/** The one child event a fix reads: an assistant message's end, with whatever usage and text it reports. */
const assistantEnd = z.object({
  type: z.literal("message_end"),
  message: z.object({
    role: z.literal("assistant"),
    content: z.union([z.string(), z.array(z.object({ type: z.string(), text: z.unknown() }))]).catch([]),
    model: z.string().optional().catch(undefined),
    stopReason: z.string().optional().catch(undefined),
    errorMessage: z.string().optional().catch(undefined),
    usage: z.object({
      input: count, output: count, cacheRead: count, cacheWrite: count,
      cost: z.object({ total: count }).catch({ total: 0 }),
    }).optional().catch(undefined),
  }),
});

/**
 * Reads a child's `--mode json` stdout as it streams, keeping only what a fix reports: its last assistant text, the
 * model it ran on, and usage summed over its assistant messages.
 */
export class PiFixReader {
  readonly #usage: PiFixUsage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 };
  #text = "";
  #model: string | undefined;
  #stopReason: string | undefined;
  #errorMessage: string | undefined;
  /** The unfinished line the last chunk ended with. */
  #partial = "";
  /** Whether the unfinished line passed `MAX_EVENT_CHARS` and is being skipped to its end. */
  #skipping = false;

  write(chunk: string): void {
    let start = 0;
    for (let end = chunk.indexOf("\n"); end !== -1; end = chunk.indexOf("\n", start)) {
      if (!this.#skipping) this.#read(this.#partial + chunk.slice(start, end));
      this.#partial = "";
      this.#skipping = false;
      start = end + 1;
    }
    if (this.#skipping) return;
    this.#partial += chunk.slice(start);
    if (this.#partial.length > MAX_EVENT_CHARS) {
      this.#partial = "";
      this.#skipping = true;
    }
  }

  /** The run the child's stdout described, once it exited with `exitCode`; `model` stands in when it named none. */
  finish(exitCode: number, model?: string): PiFixRun {
    if (!this.#skipping) this.#read(this.#partial);
    this.#partial = "";
    const resolvedModel = this.#model ?? model;
    return {
      text: this.#text,
      failed: exitCode !== 0 || this.#stopReason === "error" || this.#stopReason === "aborted",
      ...(this.#errorMessage === undefined ? {} : { errorMessage: this.#errorMessage }),
      details: { exitCode, ...(resolvedModel === undefined ? {} : { model: resolvedModel }), usage: { ...this.#usage } },
    };
  }

  #read(line: string): void {
    if (!line.includes("\"message_end\"")) return;
    let event: unknown;
    try {
      event = JSON.parse(line);
    } catch {
      return;
    }
    const parsed = assistantEnd.safeParse(event);
    if (!parsed.success) return;
    const { message } = parsed.data;
    this.#usage.turns += 1;
    if (message.usage !== undefined) {
      this.#usage.input += message.usage.input;
      this.#usage.output += message.usage.output;
      this.#usage.cacheRead += message.usage.cacheRead;
      this.#usage.cacheWrite += message.usage.cacheWrite;
      this.#usage.cost += message.usage.cost.total;
    }
    this.#model ??= message.model;
    this.#stopReason = message.stopReason;
    this.#errorMessage = message.errorMessage;
    const texts = typeof message.content === "string"
      ? [message.content]
      : message.content.flatMap(part => part.type === "text" && typeof part.text === "string" ? [part.text] : []);
    if (texts.length > 0) this.#text = texts.join("\n");
  }
}

/**
 * Runs one child `pi` to completion and returns its answer and details. An abort, or `timeoutMs` passing, stops it with
 * SIGTERM, then SIGKILL, and fails the fix, as does a failed run; only a tail of its stderr is kept.
 */
export async function runPiFix(
  invocation: { command: string; args: string[] },
  cwd: string,
  signal: AbortSignal | undefined,
  options: { model?: string; timeoutMs?: number } = {},
): Promise<{ text: string; details: PiFixDetails }> {
  const timeoutMs = options.timeoutMs ?? PI_FIX_TIMEOUT_MS;
  const { promise, resolve } = Promise.withResolvers<number>();
  const child = spawn(invocation.command, invocation.args, { cwd, shell: false, stdio: ["ignore", "pipe", "pipe"] });
  const stdout = new PiFixReader();
  let stderr = "";
  let stopped: "aborted" | "timeout" | undefined;
  let killer: NodeJS.Timeout | undefined;
  const stop = (reason: "aborted" | "timeout"): void => {
    if (stopped !== undefined) return;
    stopped = reason;
    child.kill("SIGTERM");
    killer = setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    }, KILL_GRACE_MS);
    killer.unref();
  };
  const abort = (): void => stop("aborted");
  const timer = setTimeout(() => stop("timeout"), timeoutMs);
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => stdout.write(chunk));
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr = (stderr + chunk).slice(-STDERR_TAIL_CHARS); });
  child.on("close", code => resolve(code ?? 1));
  child.on("error", error => {
    stderr = (stderr + error.message).slice(-STDERR_TAIL_CHARS);
    resolve(1);
  });
  if (signal?.aborted === true) abort();
  else signal?.addEventListener("abort", abort, { once: true });
  const exitCode = await promise;
  clearTimeout(timer);
  clearTimeout(killer);
  signal?.removeEventListener("abort", abort);
  if (stopped === "aborted") throw new Error(`${PI_FIX_TOOL} was aborted`);
  if (stopped === "timeout") throw new Error(`${PI_FIX_TOOL} timed out after ${timeoutMs / 1000}s and was stopped`);
  const run = stdout.finish(exitCode, options.model);
  if (run.failed) {
    const reason = run.errorMessage ?? (stderr.trim() === "" ? run.text : stderr.trim());
    throw new Error(`${PI_FIX_TOOL} failed (exit ${run.details.exitCode}): ${reason === "" ? "(no output)" : reason}`);
  }
  return { text: run.text === "" ? "(no output)" : run.text, details: run.details };
}

/**
 * Internal testing: the experimental `subagent` steer strategy's `rerail_fix`, which hands one Rerail rule violation to
 * a child `pi` on the parent's model and thinking level, in the session's checkout, and returns the child's
 * one-sentence account of its change.
 */
export function registerPiFixTool(pi: Pick<ExtensionAPI, "registerTool">): void {
  pi.registerTool({
    name: PI_FIX_TOOL,
    label: "Rerail Fix",
    description: internalTesting(`Delegate the fix of one Rerail rule violation to a subagent, which makes the smallest change that satisfies the rule and replies with one sentence. Call ${PI_FIX_TOOL} when a Rerail steer asks you to delegate its fix, then carry on with your own task.`),
    promptSnippet: `${PI_FIX_TOOL}: delegate the fix of a Rerail rule violation to a subagent`,
    promptGuidelines: [
      `When a Rerail steer asks you to delegate its fix, call ${PI_FIX_TOOL} with the steer text, the file, and the location, then continue your own task.`,
    ],
    parameters,
    async execute(_toolCallId, input, signal, _onUpdate, ctx) {
      const parent: PiFixParent = {
        ...(ctx.model === undefined ? {} : { provider: ctx.model.provider, model: ctx.model.id }),
        ...(ctx.thinkingLevel === undefined ? {} : { thinkingLevel: ctx.thinkingLevel }),
      };
      const { text, details } = await runPiFix(
        piFixInvocation(input.task, parent),
        ctx.cwd,
        signal,
        ctx.model === undefined ? {} : { model: ctx.model.id },
      );
      return { content: [{ type: "text", text }], details };
    },
  });
}
