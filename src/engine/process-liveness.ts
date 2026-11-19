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
 * The state of the process holding `pid` ({@link ProcessState}). Signal 0 only ever fails with `EPERM` or `ESRCH`, on
 * POSIX `kill(2)` and in libuv's Windows `uv_kill` alike; any other failure is read as gone.
 */
export function processState(pid: number, kill: ProcessSignal = (target, signal) => process.kill(target, signal)): ProcessState {
  const signalled = signalState(pid, kill);
  if (signalled === "gone") return "gone";
  const zombie = isZombie(pid);
  // A zombie answers signal 0 until it is reaped, and its /proc entry goes with the reaping: one reaped between the
  // signal and the read answers the signal again with `ESRCH`, so it reads as gone, never as running.
  if (zombie === undefined) return signalState(pid, kill);
  return zombie ? "gone" : signalled;
}

/** What signal 0 alone says of `pid`. */
function signalState(pid: number, kill: ProcessSignal): ProcessState {
  try {
    kill(pid, 0);
  } catch (error) {
    return error instanceof Error && "code" in error && error.code === "EPERM" ? "running-not-signalable" : "gone";
  }
  return "running";
}

/**
 * A process that has exited but was never reaped still answers signal 0. In a container without an init process
 * nothing reaps a detached daemon, so it stays a zombie for as long as the container runs: it holds no port, no socket
 * and no lock, and counts as gone. Undefined on Linux when `/proc` has no entry to read for `pid`: it was reaped since
 * it answered, or `/proc` hides it or is not mounted.
 */
function isZombie(pid: number): boolean | undefined {
  if (process.platform !== "linux") return false;
  let stat: string;
  try {
    stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  } catch {
    return undefined;
  }
  // `pid (comm) state …`: the command name may itself hold spaces and parentheses, so the state follows the last `)`.
  return /^\s*[ZX]/u.test(stat.slice(stat.lastIndexOf(")") + 1));
}
