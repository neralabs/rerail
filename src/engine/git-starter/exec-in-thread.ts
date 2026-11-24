/**
 * A program run to its end as Node's `execFile` runs it, started in a starter thread where this process starts Git in
 * them (`../git-thread.ts`, the daemon on Windows): Windows' own programs the daemon runs while it answers hooks, such as
 * `icacls` on a session's records folder, start there as slowly as Git does on a machine whose process starts stall,
 * and Node makes each start on the thread that asks. Elsewhere it is Node's `execFile` on this thread, as before.
 */
import { promisify } from "node:util";
import { execFile } from "../child-process.js";
import type { GitProcess } from "../git-start.js";
import { threadStart } from "../git-thread.js";

/** What a run that ended with code 0 printed, as UTF-8. */
export interface Printed {
  readonly stdout: string;
  readonly stderr: string;
}

/**
 * `file` with `args` and this process's environment, run to its end with no input, as `promisify(execFile)` runs it
 * with UTF-8 output and `timeoutMs` (none when undefined): its output once it ends with code 0, else a rejection with
 * `execFile`'s error ("Command failed: <command>" and its stderr, with its `code`, `killed`, `signal` and output).
 * Started in a thread, it differs from `execFile` in two ways no caller here meets: a start that fails rejects with
 * Node's spawn error alone, without the `cmd`, `stdout` and `stderr` `execFile` adds, and its output has no
 * `maxBuffer` cut (`execFile`'s is 1 MiB). Every `icacls` here prints a few lines, and its callers read only an
 * error's message.
 */
export async function execFileInThread(file: string, args: readonly string[], timeoutMs?: number): Promise<Printed> {
  const child = threadStart(file, args, process.env, "pipe");
  if (child !== undefined) return await ranToEnd(child, [file, ...args].join(" "), timeoutMs);
  const { stdout, stderr } = await promisify(execFile)(file, [...args], { encoding: "utf8", ...(timeoutMs === undefined ? {} : { timeout: timeoutMs }) });
  return { stdout, stderr };
}

/**
 * `child`, a start {@link threadStart} made, run to its end as `execFile` runs it. Its time limit counts from its
 * start, not from its wait for a thread to make it, as a Git read's does; past it the child is killed as `execFile`
 * kills it.
 */
async function ranToEnd(child: GitProcess, command: string, timeoutMs: number | undefined): Promise<Printed> {
  const printed = { stdout: "", stderr: "" };
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => { printed.stdout += chunk; });
  child.stderr?.setEncoding("utf8").on("data", (chunk: string) => { printed.stderr += chunk; });
  child.stdin.on("error", () => undefined);
  child.stdin.end();
  let killed = false;
  let limit: NodeJS.Timeout | undefined;
  if (timeoutMs !== undefined) {
    child.once("spawn", () => {
      limit = setTimeout(() => {
        killed = child.kill();
      }, timeoutMs);
    });
  }
  try {
    const [code, signal] = await ended(child);
    if (code === 0 && signal === null) return printed;
    throw Object.assign(new Error(`Command failed: ${command}\n${printed.stderr}`), { code, killed, signal, cmd: command, ...printed });
  } finally {
    clearTimeout(limit);
  }
}

/** `child`'s code and signal once it closes; a rejection with its error when it could not start. */
async function ended(child: GitProcess): Promise<[number | null, NodeJS.Signals | null]> {
  return await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.once("close", (code: number | null, signal: NodeJS.Signals | null) => resolve([code, signal]));
  });
}
