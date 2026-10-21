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
/** How much of a failed child's stderr the tool result carries. */
const STDERR_TAIL_CHARS = 2_000;

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

/** The child `pi` command line for `task`: Pi's own entry point when there is one, the `pi` on PATH otherwise. */
export function piFixInvocation(
  task: string,
  parent: PiFixParent,
  execPath: string = process.execPath,
  script: string | undefined = process.argv[1],
): { command: string; args: string[] } {
  const args = ["--mode", "json", "-p", "--no-session", "--no-extensions"];
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

/** What a child's `--mode json` stdout says: its last assistant text, and usage summed over its assistant messages. */
export function readPiFixRun(stdout: string, exitCode: number, model?: string): PiFixRun {
  const usage: PiFixUsage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 };
  let text = "";
  let reportedModel: string | undefined;
  let stopReason: string | undefined;
  let errorMessage: string | undefined;
  for (const line of stdout.split("\n")) {
    if (line.trim() === "") continue;
    let event: unknown;
    try {
      event = JSON.parse(line);
    } catch {
      continue;
    }
    const parsed = assistantEnd.safeParse(event);
    if (!parsed.success) continue;
    const { message } = parsed.data;
    usage.turns += 1;
    if (message.usage !== undefined) {
      usage.input += message.usage.input;
      usage.output += message.usage.output;
      usage.cacheRead += message.usage.cacheRead;
      usage.cacheWrite += message.usage.cacheWrite;
      usage.cost += message.usage.cost.total;
    }
    reportedModel ??= message.model;
    stopReason = message.stopReason;
    errorMessage = message.errorMessage;
    const texts = typeof message.content === "string"
      ? [message.content]
      : message.content.flatMap(part => part.type === "text" && typeof part.text === "string" ? [part.text] : []);
    if (texts.length > 0) text = texts.join("\n");
  }
  const resolvedModel = reportedModel ?? model;
  return {
    text,
    failed: exitCode !== 0 || stopReason === "error" || stopReason === "aborted",
    ...(errorMessage === undefined ? {} : { errorMessage }),
    details: { exitCode, ...(resolvedModel === undefined ? {} : { model: resolvedModel }), usage },
  };
}

/** Runs one child `pi` to completion; an abort stops it with SIGTERM, then SIGKILL. */
function runChild(
  invocation: { command: string; args: string[] },
  cwd: string,
  signal: AbortSignal | undefined,
): Promise<{ stdout: string; stderr: string; exitCode: number; aborted: boolean }> {
  const { promise, resolve } = Promise.withResolvers<{ stdout: string; stderr: string; exitCode: number; aborted: boolean }>();
  const child = spawn(invocation.command, invocation.args, { cwd, shell: false, stdio: ["ignore", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  let aborted = false;
  let killer: NodeJS.Timeout | undefined;
  const stop = (): void => {
    aborted = true;
    child.kill("SIGTERM");
    killer = setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    }, KILL_GRACE_MS);
    killer.unref();
  };
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => { stdout += chunk; });
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
  const settle = (exitCode: number): void => {
    clearTimeout(killer);
    signal?.removeEventListener("abort", stop);
    resolve({ stdout, stderr, exitCode, aborted });
  };
  child.on("close", code => settle(code ?? 1));
  child.on("error", error => {
    stderr += error.message;
    settle(1);
  });
  if (signal?.aborted === true) stop();
  else signal?.addEventListener("abort", stop, { once: true });
  return promise;
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
      const child = await runChild(piFixInvocation(input.task, parent), ctx.cwd, signal);
      if (child.aborted) throw new Error(`${PI_FIX_TOOL} was aborted`);
      const run = readPiFixRun(child.stdout, child.exitCode, ctx.model?.id);
      if (run.failed) {
        const reason = run.errorMessage ?? (child.stderr.trim() === "" ? run.text : child.stderr.trim().slice(-STDERR_TAIL_CHARS));
        throw new Error(`${PI_FIX_TOOL} failed (exit ${run.details.exitCode}): ${reason === "" ? "(no output)" : reason}`);
      }
      return { content: [{ type: "text", text: run.text === "" ? "(no output)" : run.text }], details: run.details };
    },
  });
}
