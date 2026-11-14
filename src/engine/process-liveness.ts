import { readFileSync } from "node:fs";

/**
 * What `kill(pid, 0)`, which sends no signal, says of the process holding `pid`:
 *
 * - `running`: it runs, and this process may signal it.
 * - `running-not-signalable`: it runs, and `kill` answers `EPERM`: another user's process or a system service. On Windows
 *   this user's own elevated process answers the same way, so there `EPERM` alone never says whose process it is.
 * - `gone`: no process holds the pid (`ESRCH`), or on Linux one that exited and was never reaped (a zombie).
 */
export type ProcessState = "running" | "running-not-signalable" | "gone";

/** `process.kill`'s shape, which tests replace. */
export type ProcessSignal = (pid: number, signal?: string | number) => boolean;

/**
 * The state of the process holding `pid` ({@link ProcessState}). On Linux a zombie still answers signal 0, so its
 * `/proc/<pid>/stat` is read too. A zombie reaped between the signal and the read takes its `/proc` entry with it:
 * a read that fails is answered by signal 0 again, which then says `ESRCH`, so a process that has just exited never
 * reads as running.
 */
export function processState(pid: number, kill: ProcessSignal = (target, signal) => process.kill(target, signal)): ProcessState {
  const state = signalZero(pid, kill);
  if (state === "gone" || process.platform !== "linux") return state;
  const stat = procStat(pid);
  if (stat === undefined) return signalZero(pid, kill);
  return isZombie(stat) ? "gone" : state;
}

/**
 * What signal 0 says of `pid`. It only ever fails with `EPERM` or `ESRCH`, on POSIX `kill(2)` and in libuv's Windows
 * `uv_kill` alike; any other failure is read as gone.
 */
function signalZero(pid: number, kill: ProcessSignal): ProcessState {
  try {
    kill(pid, 0);
    return "running";
  } catch (error) {
    return error instanceof Error && "code" in error && error.code === "EPERM" ? "running-not-signalable" : "gone";
  }
}

/** `/proc/<pid>/stat`, or undefined when it cannot be read: the process is gone, or `/proc` is hidden or absent. */
function procStat(pid: number): string | undefined {
  try {
    return readFileSync(`/proc/${pid}/stat`, "utf8");
  } catch {
    return undefined;
  }
}

/**
 * A process that has exited but was never reaped still answers signal 0. In a container without an init process
 * nothing reaps a detached daemon, so it stays a zombie for as long as the container runs: it holds no port, no socket
 * and no lock, and counts as gone.
 */
function isZombie(stat: string): boolean {
  // `pid (comm) state …`: the command name may itself hold spaces and parentheses, so the state follows the last `)`.
  return /^\s*[ZX]/u.test(stat.slice(stat.lastIndexOf(")") + 1));
}
