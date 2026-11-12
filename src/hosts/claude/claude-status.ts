import { existsSync, realpathSync } from "node:fs";
import { isAbsolute, join, resolve } from "node:path";
import type { Readable, Writable } from "node:stream";
import { fileURLToPath, pathToFileURL } from "node:url";
import { z } from "zod";
import { endProcessTree, spawn } from "../../engine/child-process.js";
import { gitBash } from "../../engine/git-access/checkout-git.js";
import { processState } from "../../engine/process-liveness.js";
import { readClaudeStatusCommand, type ClaudeStatusLine } from "../../setup/claude-status-store.js";
import { fallbackPresentation, formatTerminalStatus, readPresentation } from "../../presentation/session-presentation.js";
import type { RepositoryStartState } from "../../engine/catalog/repository-model.js";
import { ENTRY_POINTS, PACKAGE_ROOT, REQUIRED_NODE_MAJOR } from "../../engine/package-root.js";
import type * as StartStateReader from "../../runtime/start-state-reader.js";

/**
 * The checkout's start state from the database, read-only: the status line refreshes often and must never
 * take the write lock, so where only a writable open could read the database (no schema yet) it cannot say
 * (undefined, "not running").
 * Loaded only when no daemon answers, as one does not while it is busy: the reader links Drizzle and SQLite. It is the
 * bundled `start-state-reader.js`, so the run resolves no Drizzle module at run time (285 MB when it did).
 */
async function localStartState(root: string): Promise<RepositoryStartState | undefined> {
  const readerPath = join(PACKAGE_ROOT, ENTRY_POINTS.startStateReader);
  if (!existsSync(readerPath)) return undefined;
  // Runtime-selected on purpose: this file is bundled, and a static or literal import would put Drizzle and SQLite in
  // the bundle, loaded on every status-line run, daemon or not.
  const reader = await import(pathToFileURL(readerPath).href) as typeof StartStateReader;
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
  // The recorded-first fallback can start this on an older `node` from PATH once the node set-up recorded is gone;
  // the status line is where the person sees that, and Rerail reads nothing on it.
  const node = process.versions.node;
  if (Number.parseInt(node, 10) < REQUIRED_NODE_MAJOR) {
    return formatTerminalStatus(fallbackPresentation(`needs Node ${REQUIRED_NODE_MAJOR} or newer (this is ${node})`));
  }
  try {
    const session = validInput ? statusSession(chunks) : undefined;
    if (session === undefined) return formatTerminalStatus(fallbackPresentation("session unavailable"));
    const { sessionId, root } = session;
    // Nothing autostarts where Rerail is off, so with no daemon answering the checkout's own state decides.
    const result = await readPresentation({ sessionId, host: "claude-code", cwd: root, startState: () => localStartState(root) });
    return result.presence === "silent" ? "" : formatTerminalStatus(result);
  } catch {
    return formatTerminalStatus(fallbackPresentation("unavailable"));
  }
}

/**
 * The session a status line's input names and the checkout it runs in, its project folder or else its `cwd`;
 * undefined when the input names no session or no absolute folder. Input that is not JSON throws.
 */
function statusSession(chunks: Buffer[]): { sessionId: string; root: string } | undefined {
  const payload = statusPayloadSchema.safeParse(JSON.parse(Buffer.concat(chunks).toString("utf8")));
  if (!payload.success) return undefined;
  const { session_id: sessionId, cwd, workspace } = payload.data;
  const root = [workspace?.project_dir, cwd].find(path => path !== undefined && isAbsolute(path));
  return root === undefined ? undefined : { sessionId, root };
}

/** How the person's previous status line is started: a program, its arguments, and whether Node wraps it in a shell. */
export interface PreviousStatusSpawn {
  file: string;
  args: string[];
  shell: boolean;
}

/**
 * The previous status line runs as Claude Code ran it: through `/bin/sh` on macOS and Linux, which is Node's
 * `shell: true` there, and through Git Bash on Windows. `shell: true` on Windows is cmd.exe, which reads `~`, single
 * quotes and `$(...)` as plain text, so a bash status line broke once Rerail wrapped it. Without a Git Bash to find,
 * cmd.exe is what is left.
 */
export function previousStatusSpawn(command: string, platform: NodeJS.Platform = process.platform, env: NodeJS.ProcessEnv = process.env): PreviousStatusSpawn {
  const bash = platform === "win32" ? claudeGitBash(env) : undefined;
  return bash === undefined ? { file: command, args: [], shell: true } : { file: bash, args: ["-c", command], shell: false };
}

/** Git Bash's bash.exe: `CLAUDE_CODE_GIT_BASH_PATH`, which Claude Code itself honours, else the one beside Rerail's Git. */
function claudeGitBash(env: NodeJS.ProcessEnv): string | undefined {
  const named = env.CLAUDE_CODE_GIT_BASH_PATH;
  return named !== undefined && named !== "" ? named : gitBash();
}

/** Preserve the user's status as a byte stream, including output before errors or timeouts. */
export async function runClaudeStatus(
  input: Readable = process.stdin,
  output: Writable = process.stdout,
  settingsPath?: string,
): Promise<void> {
  const ancestor = Number(process.env[PARENT_PROCESS_KEY]);
  // A stale ancestry marker, whose pid is gone or another user's now, must not disable status.
  if (Number.isSafeInteger(ancestor) && ancestor > 0 && processState(ancestor) === "running") {
    input.resume();
    return;
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
    const plan = previousStatusSpawn(previous.command);
    const child = spawn(plan.file, plan.args, {
      shell: plan.shell,
      detached: process.platform !== "win32",
      env: { ...process.env, [PARENT_PROCESS_KEY]: String(process.pid) },
      stdio: ["pipe", "pipe", "ignore"],
    });
    const timer = setTimeout(() => {
      // The command and what it started: on macOS and Linux its process group (`detached`); on Windows the tree from
      // Git Bash's launcher, which runs the real bash as its child.
      endProcessTree(child, { group: true });
      // A process the command started can hold stdout open past the kill (one that left the group, or any child on
      // Windows); the budget holds without waiting for it to let go.
      child.stdout.destroy();
      finish();
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
  if (rerailText !== "") output.write(appendedLine(rerailText, lastByte));
}

/** `text` as a line after output whose last byte was `lastByte`: a newline first when that output ended mid-line. */
function appendedLine(text: string, lastByte: number | undefined): string {
  return `${lastByte !== undefined && lastByte !== 10 ? "\n" : ""}${text}\n`;
}

const launchedPath = process.argv[1];
if (launchedPath !== undefined && realpathSync(fileURLToPath(import.meta.url)) === realpathSync(resolve(launchedPath))) {
  await runClaudeStatus(undefined, undefined, process.argv[2]);
}
