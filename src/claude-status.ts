import { spawn } from "node:child_process";
import { realpathSync } from "node:fs";
import { isAbsolute, resolve } from "node:path";
import type { Readable, Writable } from "node:stream";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { readClaudeStatusCommand, type ClaudeStatusLine } from "./claude-status-store.js";
import { formatTerminalStatus, readPresentation } from "./session-presentation.js";
import type { RepositoryStartState } from "./repository-start.js";

/**
 * The checkout's start state from the database, read-only: the status line refreshes often and must never
 * take the write lock, so where only a writable open could read the database (no schema yet) it cannot say
 * (undefined, "not running").
 * Loaded only when no daemon answers: the reader links Drizzle and SQLite, about 170 ms per status-line run.
 */
async function localStartState(root: string): Promise<RepositoryStartState | undefined> {
  // Static import cannot work here: it would load Drizzle and SQLite on every status-line run, daemon or not.
  const reader = await import("./repository-start.js");
  return reader.readOnlyStartState(root);
}

const STATUS_TIMEOUT_MS = 2_000;
const MAX_INPUT_BYTES = 1_048_576;
// Process ancestry marker, not a presentation preference: stop a user's command recursively invoking this wrapper.
const PARENT_PROCESS_KEY = "_RERAIL_CLAUDE_STATUS_PARENT";
/** `cwd` follows `/cd`; `workspace.project_dir` stays the launch root that identifies the session. */
const statusPayloadSchema = z.object({
  session_id: z.string(),
  cwd: z.string().optional(),
  workspace: z.object({ project_dir: z.string().optional() }).optional(),
});

async function rerailStatus(chunks: Buffer[], validInput: boolean): Promise<string> {
  if (!validInput) return "Rerail | session unavailable";
  try {
    const payload = statusPayloadSchema.safeParse(JSON.parse(Buffer.concat(chunks).toString("utf8")));
    if (!payload.success) return "Rerail | session unavailable";
    const { session_id: sessionId, cwd, workspace } = payload.data;
    const root = [workspace?.project_dir, cwd].find(path => path !== undefined && isAbsolute(path));
    if (root === undefined) return "Rerail | session unavailable";
    // Nothing autostarts where Rerail is off, so with no daemon answering the checkout's own state decides.
    const result = await readPresentation({ sessionId, host: "claude-code", cwd: root, startState: () => localStartState(root) });
    return result.presence === "silent" ? "" : formatTerminalStatus(result);
  } catch {
    return "Rerail | unavailable";
  }
}

/** Preserve the user's status as a byte stream, including output before errors or timeouts. */
export async function runClaudeStatus(
  input: Readable = process.stdin,
  output: Writable = process.stdout,
  settingsPath?: string,
): Promise<void> {
  const ancestor = Number(process.env[PARENT_PROCESS_KEY]);
  if (Number.isSafeInteger(ancestor) && ancestor > 0) {
    try {
      process.kill(ancestor, 0);
      input.resume();
      return;
    } catch { /* A stale ancestry marker must not disable status. */ }
  }
  let previous: ClaudeStatusLine | undefined;
  try {
    previous = readClaudeStatusCommand(undefined, settingsPath);
  } catch {
    // A missing/corrupt registration cannot prevent the independent Rerail read.
  }
  let lastByte: number | undefined;
  const previousDone = new Promise<void>(resolvePrevious => {
    if (previous === undefined) {
      resolvePrevious();
      return;
    }
    const child = spawn(previous.command, {
      shell: true,
      detached: process.platform !== "win32",
      env: { ...process.env, [PARENT_PROCESS_KEY]: String(process.pid) },
      stdio: ["pipe", "pipe", "ignore"],
    });
    const timer = setTimeout(() => {
      // Kill the shell's entire group so a grandchild cannot retain stdout indefinitely.
      try {
        if (process.platform !== "win32" && child.pid !== undefined) process.kill(-child.pid, "SIGKILL");
        else child.kill("SIGKILL");
      } catch { /* The command may already have exited. */ }
    }, STATUS_TIMEOUT_MS);
    child.stdout.on("data", (chunk: Buffer) => {
      if (chunk.length > 0) lastByte = chunk[chunk.length - 1];
      if (!output.write(chunk)) child.stdout.pause();
    });
    const resumeOutput = (): void => { child.stdout.resume(); };
    output.on("drain", resumeOutput);
    const finish = (): void => {
      clearTimeout(timer);
      input.unpipe(child.stdin);
      input.resume();
      output.off("drain", resumeOutput);
      resolvePrevious();
    };
    child.on("error", finish);
    child.on("close", finish);
    child.stdin.on("error", () => { /* Early exit/EPIPE does not discard prior stdout. */ });
    input.pipe(child.stdin);
  });

  const chunks: Buffer[] = [];
  let bytes = 0;
  let validInput = true;
  await new Promise<void>(resolveInput => {
    const timer = setTimeout(() => {
      validInput = false;
      input.destroy();
      resolveInput();
    }, STATUS_TIMEOUT_MS);
    input.on("data", (value: Buffer | string) => {
      const chunk = Buffer.isBuffer(value) ? value : Buffer.from(value);
      bytes += chunk.length;
      // Continue forwarding every byte to the original command even if Rerail's JSON budget is exceeded.
      if (bytes <= MAX_INPUT_BYTES) chunks.push(chunk);
      else validInput = false;
    });
    const finish = (): void => {
      clearTimeout(timer);
      resolveInput();
    };
    input.on("end", finish);
    input.on("close", finish);
    input.on("error", () => { validInput = false; finish(); });
    input.resume();
  });

  const rerailText = await rerailStatus(chunks, validInput);
  await previousDone;
  if (rerailText !== "") output.write(`${lastByte !== undefined && lastByte !== 10 ? "\n" : ""}${rerailText}\n`);
}

const launchedPath = process.argv[1];
if (launchedPath !== undefined && realpathSync(fileURLToPath(import.meta.url)) === realpathSync(resolve(launchedPath))) {
  await runClaudeStatus(undefined, undefined, process.argv[2]);
}
