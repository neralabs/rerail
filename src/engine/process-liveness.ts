import { readFileSync } from "node:fs";
import { spawnSync } from "./child-process.js";
import { windowsPowerShell } from "./system-programs.js";

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
  let state: ProcessState = "running";
  try {
    kill(pid, 0);
  } catch (error) {
    if (!(error instanceof Error && "code" in error && error.code === "EPERM")) return "gone";
    state = "running-not-signalable";
  }
  return isZombie(pid) ? "gone" : state;
}

/**
 * A process that has exited but was never reaped still answers signal 0. In a container without an init process
 * nothing reaps a detached daemon, so it stays a zombie for as long as the container runs: it holds no port, no socket
 * and no lock, and counts as gone.
 */
function isZombie(pid: number): boolean {
  if (process.platform !== "linux") return false;
  let stat: string;
  try {
    stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  } catch {
    return false;
  }
  // `pid (comm) state …`: the command name may itself hold spaces and parentheses, so the state follows the last `)`.
  return /^\s*[ZX]/u.test(stat.slice(stat.lastIndexOf(")") + 1));
}

/**
 * When the process holding `pid` started, in epoch milliseconds, or undefined where it cannot be read. A process keeps
 * its pid for life, so its start time tells the process that holds a pid now from one that held it before. Linux reads
 * `/proc` ({@link linuxStartedAt}), Windows the process's creation time ({@link windowsStartedAt}); other systems are
 * not read.
 */
export function processStartedAt(pid: number, platform: NodeJS.Platform = process.platform): number | undefined {
  if (platform === "linux") return linuxStartedAt(pid);
  return platform === "win32" ? windowsStartedAt(pid) : undefined;
}

/**
 * Linux: the start in `/proc/<pid>/stat`, in clock ticks since boot (USER_HZ, 100 on every architecture Node runs on
 * there), set against the time since boot in `/proc/uptime`, both counted on the clock that goes on through a suspend.
 * The result is on the wall clock as it reads now, to within 20 ms.
 */
function linuxStartedAt(pid: number): number | undefined {
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
    // The fields after the command name's last `)` begin with the third, the state, so the start, the 22nd, is the 20th.
    const ticks = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[19]);
    const uptimeMs = Number(readFileSync("/proc/uptime", "utf8").split(" ")[0]) * 1_000;
    return Number.isFinite(ticks) && Number.isFinite(uptimeMs) ? Math.round(Date.now() - uptimeMs + ticks * 10) : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Windows: the process's creation time, as .NET's `Process.StartTime` reads it, through Windows PowerShell, which loads
 * no Rerail engine: the uninstall reads it before it removes `versions/`, and Windows keeps a loaded DLL from being
 * deleted. A process this user may not query is not read.
 */
function windowsStartedAt(pid: number): number | undefined {
  const read = spawnSync(windowsPowerShell(), ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
    `([DateTimeOffset][Diagnostics.Process]::GetProcessById(${pid}).StartTime).ToUnixTimeMilliseconds()`], { encoding: "utf8" });
  const text = String(read.stdout ?? "").trim();
  return read.status === 0 && /^\d+$/u.test(text) ? Number(text) : undefined;
}
