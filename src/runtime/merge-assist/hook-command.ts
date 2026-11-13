/**
 * The command handler (docs/merge-assist-v1-build.md): the person's conflict hook command, run on every conflict
 * event. It runs from the checkout's root as an argv array, never through a shell, with the person's environment plus
 * `RERAIL_CONFLICT_EVENT` and `RERAIL_CONFLICT_ID`; the conflict reaches it only on stdin, as one JSON object, never in
 * its arguments. It may answer on stdout with `{ "note"?: string, "status"?: string }` (at most 4 KiB): `note` joins
 * the agent's note for the conflict, `status` replaces the conflict's summary wherever its line shows. A non-zero exit,
 * a timeout or output that is not that JSON is recorded on the conflict (`hook: { ok: false, error }`) and changes
 * nothing else: delivery goes on without it.
 */
import { spawn, type ChildProcess } from "node:child_process";
import type { ConflictEvent, ConflictHookCommand, ConflictHookOutcome } from "../../engine/merge-assist/model.js";
import type { ConflictHandler, ConflictHandlerContext } from "./conflict-hook.js";
import type { StoredConflict } from "./conflict-store.js";
import { isoSeconds } from "./state.js";

/** The most the command may write to stdout. */
export const MAX_HOOK_OUTPUT_BYTES = 4 * 1024;
/** The longest note and status line kept from its answer. */
export const MAX_HOOK_NOTE = 600;
export const MAX_HOOK_STATUS = 90;
/** Grace a timed-out command's process group gets after SIGTERM before SIGKILL. */
const KILL_GRACE_MS = 2_000;
/** How much of the command's stderr or rejected output a failure reports. */
const DETAIL_TAIL_CHARS = 300;
const ANSI_SEQUENCES = /\u001B\][^\u0007\u001B]*(?:\u0007|\u001B\\)|\u001B\[[0-?]*[ -/]*[@-~]|\u001B[@-Z\\-_]/gu;

/** What the command gets on stdin. */
export interface HookPayload {
  readonly version: 1;
  readonly event: ConflictEvent;
  readonly conflict: StoredConflict["conflict"];
  readonly checkout: { readonly path: string; readonly branch: string | null; readonly base: string | null };
  readonly session: { readonly id: string; readonly host: string | null } | null;
}

type Answer = Pick<ConflictHookOutcome, "ok" | "error" | "note" | "status">;

const failed = (error: string): Answer => ({ ok: false, error, note: null, status: null });

/** Output fit for a person: no escape sequences or control characters, bearer and API tokens masked, the last 300 characters. */
function sanitizeTail(text: string): string {
  const clean = text
    .replace(ANSI_SEQUENCES, "")
    .replace(/[\p{Cc}\p{Cf}]+/gu, " ")
    .replace(/\b(Bearer\s+)[\w.~+/=-]+/giu, "$1[REDACTED]")
    .replace(/\b(?:sk|ghp|gho|ghs|github_pat|xox[abp])[-_][\w-]{8,}/gu, "[REDACTED]")
    .replace(/\s+/gu, " ")
    .trim();
  return clean.length <= DETAIL_TAIL_CHARS ? clean : `…${clean.slice(-(DETAIL_TAIL_CHARS - 1))}`;
}

/** Text kept from the command: no control character but new lines in a note, one line for a status, then cut. */
function kept(text: string | undefined, max: number, oneLine: boolean): string | null {
  const clean = (oneLine ? text?.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]+/gu, " ").replace(/\s+/gu, " ") : text?.replace(/[^\P{Cc}\n]+/gu, " "))?.trim() ?? "";
  if (clean === "") return null;
  return clean.length <= max ? clean : `${clean.slice(0, max - 1).trimEnd()}…`;
}

/** The command's stdout as a JSON object, or why it is not one. */
function outputObject(stdout: string): Record<string, unknown> | string {
  try {
    const value: unknown = JSON.parse(stdout);
    return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : "its output is not a JSON object";
  } catch {
    return `its output is not JSON: ${sanitizeTail(stdout)}`;
  }
}

/** The command's stdout read as its answer: empty, or one JSON object with optional string `note` and `status`. */
function answerOf(stdout: string): Answer {
  if (stdout.trim() === "") return { ok: true, error: null, note: null, status: null };
  const output = outputObject(stdout);
  if (typeof output === "string") return failed(output);
  const { note, status } = output;
  if ([note, status].some(field => field !== undefined && typeof field !== "string")) return failed("its output's note and status must be strings");
  return { ok: true, error: null, note: kept(note as string | undefined, MAX_HOOK_NOTE, false), status: kept(status as string | undefined, MAX_HOOK_STATUS, true) };
}

/** Why a command that ran to its end failed, from its exit and stderr; null when it exited 0. */
function exitProblem(code: number | null, signal: NodeJS.Signals | null, stderr: string): string | null {
  if (code === 0) return null;
  const tail = stderr.trim() === "" ? "" : `: ${sanitizeTail(stderr)}`;
  return `${code === null ? `stopped by ${signal ?? "a signal"}` : `exited with code ${code}`}${tail}`;
}

/** Signal the command's process group (its own, `detached`), or the command alone where there is none (Windows). */
function signalGroup(child: ChildProcess, signal: NodeJS.Signals): void {
  try {
    if (child.pid !== undefined) process.kill(-child.pid, signal);
  } catch {
    child.kill(signal);
  }
}

/**
 * The hook commands one Merge Assist has running, so stopping it stops them: a command that never exits would
 * otherwise outlive the daemon, reparented away from it.
 */
export class RunningCommands {
  readonly #children = new Map<ChildProcess, Promise<void>>();

  add(child: ChildProcess): void {
    const { promise: closed, resolve } = Promise.withResolvers<void>();
    child.once("close", () => resolve());
    child.once("error", () => resolve());
    this.#children.set(child, closed);
    void closed.then(() => this.#children.delete(child));
  }

  /** SIGTERM to every running command's process group, then SIGKILL after `graceMs` to each still running. */
  async stop(graceMs: number): Promise<void> {
    const running = [...this.#children];
    if (running.length === 0) return;
    for (const [child] of running) signalGroup(child, "SIGTERM");
    const grace = Promise.withResolvers<void>();
    const timer = setTimeout(grace.resolve, graceMs);
    try {
      await Promise.race([Promise.all(running.map(([, closed]) => closed)), grace.promise]);
    } finally {
      clearTimeout(timer);
    }
    for (const [child] of running) if (this.#children.has(child)) signalGroup(child, "SIGKILL");
  }
}

/**
 * Run the command once with `payload` on stdin; never throws. It runs in a process group of its own: at its timeout
 * the group gets SIGTERM, and SIGKILL after a 2-second grace, when its pipes are closed too, so a process it left
 * behind holding them (one it detached) cannot keep the answer waiting. The answer comes at most its timeout plus the
 * grace after it started.
 */
export function runHookCommand(command: ConflictHookCommand, payload: HookPayload, options: { cwd: string; env: NodeJS.ProcessEnv; running?: RunningCommands }): Promise<Answer> {
  const [program, ...args] = command.command;
  const { promise, resolve } = Promise.withResolvers<Answer>();
  if (program === undefined) return Promise.resolve(failed("no command is set"));
  const child = spawn(program, args, {
    cwd: options.cwd,
    env: { ...options.env, RERAIL_CONFLICT_EVENT: payload.event, RERAIL_CONFLICT_ID: payload.conflict.id },
    stdio: ["pipe", "pipe", "pipe"],
    shell: false,
    detached: true,
  });
  options.running?.add(child);
  const stdout: Buffer[] = [];
  let size = 0;
  let overflow = false;
  let stderr = "";
  let timedOut = false;
  let kill: NodeJS.Timeout | undefined;
  const timer = setTimeout(() => {
    timedOut = true;
    signalGroup(child, "SIGTERM");
    kill = setTimeout(() => {
      signalGroup(child, "SIGKILL");
      child.stdout.destroy();
      child.stderr.destroy();
      resolve(failed(`timed out after ${command.timeoutSeconds} s`));
    }, KILL_GRACE_MS);
  }, command.timeoutSeconds * 1_000);
  child.stdout.on("data", (chunk: Buffer) => {
    size += chunk.byteLength;
    if (size > MAX_HOOK_OUTPUT_BYTES) overflow = true;
    else stdout.push(chunk);
  });
  child.stderr.on("data", (chunk: Buffer) => {
    stderr = `${stderr}${chunk.toString("utf8")}`.slice(-4_000);
  });
  // A command that does not read its stdin closes it early; that is no failure of its own.
  child.stdin.on("error", () => undefined);
  child.on("error", error => {
    clearTimeout(timer);
    resolve(failed(`could not start ${program}: ${error.message}`));
  });
  child.on("close", (code, signal) => {
    clearTimeout(timer);
    clearTimeout(kill);
    const problem = timedOut ? `timed out after ${command.timeoutSeconds} s` : exitProblem(code, signal, stderr);
    if (problem !== null) resolve(failed(problem));
    else if (overflow) resolve(failed(`its output is over ${MAX_HOOK_OUTPUT_BYTES} bytes`));
    else resolve(answerOf(Buffer.concat(stdout).toString("utf8")));
  });
  child.stdin.end(`${JSON.stringify(payload)}\n`);
  return promise;
}

/** The payload for one event, as the command reads it. */
function payloadOf(event: ConflictEvent, stored: StoredConflict, context: ConflictHandlerContext): HookPayload {
  const { path, branch, base } = context.checkout;
  return { version: 1, event, conflict: stored.conflict, checkout: { path, branch, base }, session: context.session };
}

/**
 * The person's command, when one is set, run as one of `running`. A conflict's agent note waits for its answer to
 * `detected`, so its `note` joins the note; never longer than the hook's bound on such a wait (`NOTE_WAIT_MS`): an
 * answer after that follows as a note of its own.
 */
export function commandHandler(running: RunningCommands): ConflictHandler {
  return {
    name: "command",
    awaits: (event, context) => context.settings.hook !== null && event === "detected",
    on: async (event, stored, context) => {
      const command = context.settings.hook;
      if (command === null) return;
      const answer = await runHookCommand(command, payloadOf(event, stored, context), { cwd: context.checkout.path, env: context.env, running });
      return { hook: { event, ...answer, at: isoSeconds(context.now()) } };
    },
  };
}
