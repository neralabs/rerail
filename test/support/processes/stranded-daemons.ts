import { execFileSync } from "node:child_process";
import { dirname } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

/** The program and arguments that print every process as `<pid> <parent pid> <command line>`, one to a line. */
const PROCESS_LIST: readonly [string, ...string[]] = ["ps", "-A", "-o", "pid=,ppid=,command="];

/**
 * Daemons this test file spawned that are still running. A spawned daemon is detached but keeps this worker as its
 * parent until the worker exits, after which nothing would ever stop it. Windows launches through the native binding
 * and has no `ps`; it is not checked. `listProcesses` prints the list: `ps`, unless a test gives a list of its own.
 */
export function strandedDaemons(listProcesses = PROCESS_LIST): Array<{ pid: number; home: string }> {
  if (process.platform === "win32") return [];
  const [program, ...args] = listProcesses;
  const stranded: Array<{ pid: number; home: string }> = [];
  for (const line of execFileSync(program, args, { encoding: "utf8" }).split("\n")) {
    const match = /^\s*(\d+)\s+(\d+)\s+.*[\\/]dist[\\/]daemon\.js --state-file (.+?)(?: --|$)/u.exec(line);
    if (match !== null && Number(match[2]) === process.pid) stranded.push({ pid: Number(match[1]), home: dirname(match[3]!) });
  }
  return stranded;
}

/** Sends `signal` to `pid`; false when the process is already gone. */
function signalProcess(pid: number, signal: NodeJS.Signals | 0): boolean {
  try {
    process.kill(pid, signal);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ESRCH") return false;
    throw error;
  }
}

/**
 * Stops the process `ps` found rather than going through `stopDaemon(home)`: that signals whatever pid the home's
 * state file records, which a test may have pointed at another process, and a test file's `vi.mock` can replace it.
 */
export async function stopProcess(pid: number): Promise<void> {
  if (!signalProcess(pid, "SIGTERM")) return;
  const deadline = Date.now() + 2_000;
  while (signalProcess(pid, 0) && Date.now() < deadline) await sleep(25);
  signalProcess(pid, "SIGKILL");
}
