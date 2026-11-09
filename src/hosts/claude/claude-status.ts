import type { ChildProcess } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { isAbsolute, resolve, win32 } from "node:path";
import type { Readable, Writable } from "node:stream";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { spawn } from "../../engine/child-process.js";
import { gitExecutable } from "../../engine/git.js";
import { windowsSystemProgram } from "../../engine/system-programs.js";
import { readClaudeStatusCommand, type ClaudeStatusLine } from "../../setup/claude-status-store.js";
import { fallbackPresentation, formatTerminalStatus, readPresentation } from "../../presentation/session-presentation.js";
import type { RepositoryStartState } from "../../engine/catalog/repository-model.js";
import { REQUIRED_NODE_MAJOR } from "../../engine/package-root.js";

/**
 * The checkout's start state from the database, read-only: the status line refreshes often and must never
 * take the write lock, so where only a writable open could read the database (no schema yet) it cannot say
 * (undefined, "not running").
 * Loaded only when no daemon answers: the reader links Drizzle and SQLite, about 170 ms per status-line run.
 */
async function localStartState(root: string): Promise<RepositoryStartState | undefined> {
  // Static import cannot work here: it would load Drizzle and SQLite on every status-line run, daemon or not.
  const reader = await import("../../runtime/repository-start.js");
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
  const sessionUnavailable = (): string => formatTerminalStatus(fallbackPresentation("session unavailable"));
  if (!validInput) return sessionUnavailable();
  try {
    const payload = statusPayloadSchema.safeParse(JSON.parse(Buffer.concat(chunks).toString("utf8")));
    if (!payload.success) return sessionUnavailable();
    const { session_id: sessionId, cwd, workspace } = payload.data;
    const root = [workspace?.project_dir, cwd].find(path => path !== undefined && isAbsolute(path));
    if (root === undefined) return sessionUnavailable();
    // Nothing autostarts where Rerail is off, so with no daemon answering the checkout's own state decides.
    const result = await readPresentation({ sessionId, host: "claude-code", cwd: root, startState: () => localStartState(root) });
    return result.presence === "silent" ? "" : formatTerminalStatus(result);
  } catch {
    return formatTerminalStatus(fallbackPresentation("unavailable"));
  }
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
  const bash = platform === "win32" ? gitBash(env) : undefined;
  return bash === undefined ? { file: command, args: [], shell: true } : { file: bash, args: ["-c", command], shell: false };
}

/** Git Bash's bash.exe: `CLAUDE_CODE_GIT_BASH_PATH`, which Claude Code itself honours, else the one beside Rerail's Git. */
function gitBash(env: NodeJS.ProcessEnv): string | undefined {
  const named = env.CLAUDE_CODE_GIT_BASH_PATH;
  if (named !== undefined && named !== "") return named;
  try {
    // Git for Windows' `<Git>\bin\bash.exe` sits a folder or two above its git.exe: `cmd\`, `bin\` or `mingw64\bin\`.
    let folder = win32.dirname(gitExecutable());
    for (let level = 0; level < 3; level += 1) {
      const bash = win32.join(folder, "bin", "bash.exe");
      if (existsSync(bash)) return bash;
      folder = win32.dirname(folder);
    }
  } catch {
    // No Git: no Git Bash either.
  }
  return undefined;
}

/** Ends the process tree under `pid`, calling `failed` when that could not be done. */
export type ProcessTreeKill = (pid: number, failed: () => void) => void;

/** `taskkill /T /F`, which walks the tree from `pid`: the process must still be running for it to find the rest. */
function taskkillTree(pid: number, failed: () => void): void {
  const taskkill = spawn(windowsSystemProgram("taskkill.exe"), ["/T", "/F", "/PID", String(pid)], { stdio: "ignore" });
  taskkill.on("error", failed);
  taskkill.on("exit", code => {
    if (code !== 0) failed();
  });
}

function killShell(child: Pick<ChildProcess, "kill">): void {
  try {
    child.kill("SIGKILL");
  } catch { /* The command may already have exited. */ }
}

/**
 * Ends the previous status line and what it started. A POSIX command runs in its own process group, which goes whole.
 * On Windows killing a process leaves its children running, and the shell is Git Bash's launcher, which runs the real
 * bash as its child: `taskkill /T` ends the tree while the launcher still runs, and the launcher alone is killed only
 * when taskkill fails.
 */
export function stopPreviousStatus(
  child: Pick<ChildProcess, "pid" | "kill">,
  platform: NodeJS.Platform = process.platform,
  killTree: ProcessTreeKill = taskkillTree,
): void {
  if (platform === "win32") {
    if (child.pid === undefined) killShell(child);
    else killTree(child.pid, () => killShell(child));
    return;
  }
  try {
    if (child.pid !== undefined) process.kill(-child.pid, "SIGKILL");
    else child.kill("SIGKILL");
  } catch { /* The command may already have exited. */ }
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
    const plan = previousStatusSpawn(previous.command);
    const child = spawn(plan.file, plan.args, {
      shell: plan.shell,
      detached: process.platform !== "win32",
      env: { ...process.env, [PARENT_PROCESS_KEY]: String(process.pid) },
      stdio: ["pipe", "pipe", "ignore"],
    });
    const timer = setTimeout(() => {
      stopPreviousStatus(child);
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
  if (rerailText !== "") output.write(`${lastByte !== undefined && lastByte !== 10 ? "\n" : ""}${rerailText}\n`);
}

const launchedPath = process.argv[1];
if (launchedPath !== undefined && realpathSync(fileURLToPath(import.meta.url)) === realpathSync(resolve(launchedPath))) {
  await runClaudeStatus(undefined, undefined, process.argv[2]);
}
