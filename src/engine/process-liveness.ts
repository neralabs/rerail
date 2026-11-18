import { readFileSync } from "node:fs";
import { spawnSync } from "./child-process.js";
import { loadNativeBinding, type NativeBinding } from "./native/native.js";
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

/**
 * When the process holding `pid` started, in epoch milliseconds, or undefined where it cannot be read. A process keeps
 * its pid for life, so its start time tells the process that holds a pid now from one that held it before. Linux reads
 * `/proc` ({@link linuxStartedAt}), Windows the process's creation time through Windows PowerShell within `timeoutMs`
 * ({@link windowsStartedAt}), which loads no engine: for a process that deletes the engine's folder after it reads, as
 * the uninstall does. Every other reader asks {@link processStartedAtThroughEngine}. Other systems are not read.
 */
export function processStartedAt(pid: number, timeoutMs: number, platform: NodeJS.Platform = process.platform): number | undefined {
  return startedAt(pid, platform, () => windowsStartedAt(pid, timeoutMs));
}

/**
 * {@link processStartedAt}, with Windows reading the creation time through the engine (`GetProcessTimes`), which starts
 * no process: a hook or a status pays a system call where a PowerShell start costs about a second, and a policy that
 * blocks Windows PowerShell does not keep it from the answer. As there, Windows reads only while `timeoutMs` is left: a
 * lookup whose limit went to an engine silent to the proof reads nothing, and keeps the record. An engine that does not
 * load, or was built before the read, reads nothing.
 */
export function processStartedAtThroughEngine(
  pid: number,
  timeoutMs: number,
  platform: NodeJS.Platform = process.platform,
  engine: () => Pick<NativeBinding, "processStartedAt"> = loadNativeBinding,
): number | undefined {
  return startedAt(pid, platform, () => (timeoutMs > 0 ? engineStartedAt(pid, engine) : undefined));
}

/** Linux's start of `pid`, `windows` on Windows, and none elsewhere. */
function startedAt(pid: number, platform: NodeJS.Platform, windows: () => number | undefined): number | undefined {
  if (platform === "linux") return linuxStartedAt(pid);
  return platform === "win32" ? windows() : undefined;
}

/** What the engine reads of when `pid` started; nothing when it does not load or has no such read. */
function engineStartedAt(pid: number, engine: () => Pick<NativeBinding, "processStartedAt">): number | undefined {
  try {
    return engine().processStartedAt?.(pid) ?? undefined;
  } catch {
    return undefined;
  }
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

/** Milliseconds from 1601-01-01 UTC, where a Windows FILETIME counts from, to the Unix epoch. */
const FILETIME_EPOCH_MS = 11_644_473_600_000;

/**
 * Windows: the process's creation time, as `Get-Process` reads it, through Windows PowerShell, which loads no Rerail
 * engine: the uninstall reads it before it removes `versions/`, and Windows keeps a loaded DLL from being deleted. The
 * command is a cmdlet and a method of a core type, which PowerShell's Constrained Language Mode allows: AppLocker and WDAC
 * script policies impose that mode, and it refuses .NET's static methods. It prints a FILETIME, 100 ns ticks since 1601
 * in UTC, past a double's exact integers, so it is divided as a BigInt. The read runs synchronously, so PowerShell is
 * ended at `timeoutMs` and not started with no time left (Node reads a timeout of 0 as none). A process this user may not
 * query, a PowerShell that cannot run and one ended at the bound read nothing.
 */
function windowsStartedAt(pid: number, timeoutMs: number): number | undefined {
  if (!(timeoutMs > 0)) return undefined;
  const read = spawnSync(windowsPowerShell(), ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
    `(Get-Process -Id ${pid} -ErrorAction Stop).StartTime.ToFileTimeUtc()`], { encoding: "utf8", timeout: timeoutMs });
  const text = String(read.stdout ?? "").trim();
  return read.status === 0 && /^\d+$/u.test(text) ? Number(BigInt(text) / 10_000n) - FILETIME_EPOCH_MS : undefined;
}
