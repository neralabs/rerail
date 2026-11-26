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
 * - `gone`: no process holds the pid (`ESRCH`), or on Linux one that exited, every thread of it, and was never reaped (a
 *   zombie).
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
 * and no lock, and counts as gone ({@link exitedEveryThread}). Undefined on Linux when `/proc` has no entry to read for
 * `pid`: it was reaped since it answered, or `/proc` hides it or is not mounted.
 */
function isZombie(pid: number): boolean | undefined {
  if (process.platform !== "linux") return false;
  let status: string;
  try {
    status = readFileSync(`/proc/${pid}/status`, "utf8");
  } catch {
    return undefined;
  }
  return exitedEveryThread(status);
}

/**
 * Whether `status`, the text of a Linux `/proc/<pid>/status`, shows a process that has exited, every thread of it. Its
 * main thread alone exiting is not enough: `State` reads `Z` from then on while the process's other threads still exit,
 * holding its files, sockets and locks until the last one has. The process has exited once `Threads` reads 1, the main
 * thread that exited.
 */
export function exitedEveryThread(status: string): boolean {
  return /^State:\s*[ZX]/mu.test(status) && Number(/^Threads:\s*(\d+)/mu.exec(status)?.[1] ?? 1) <= 1;
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
 * blocks Windows PowerShell does not keep it from the answer. A system call needs no limit, so Windows reads it however
 * little of a lookup's limit is left. An engine that does not load, or was built before the read, reads nothing, and
 * the first such failure is reported ({@link reportEngineReadFailure}).
 */
export function processStartedAtThroughEngine(
  pid: number,
  platform: NodeJS.Platform = process.platform,
  engine: () => Pick<NativeBinding, "processStartedAt"> = loadNativeBinding,
): number | undefined {
  return startedAt(pid, platform, () => engineStartedAt(pid, engine));
}

/**
 * How far the reader's clock may have moved since a daemon stamped its start, either way. A clock set forward since
 * then puts a later read of the daemon's own start up to this much after the stamp; one set back puts the stamp up to
 * this much past the reader's now.
 */
export const START_CLOCK_SLACK_MS = 10_000;

/**
 * Whether a recorded start (epoch ms) can name the time a process started: a finite number no more than
 * {@link START_CLOCK_SLACK_MS} past the reader's clock. Every process's start compares no later than Infinity, or than a
 * start further ahead, so any process at the pid would pass for the one that wrote it, and none compares to NaN.
 */
export function startNamesProcess(startedAt: number): boolean {
  return Number.isFinite(startedAt) && startedAt <= Date.now() + START_CLOCK_SLACK_MS;
}

/** Linux's start of `pid`, `windows` on Windows, and none elsewhere. */
function startedAt<Read>(pid: number, platform: NodeJS.Platform, windows: () => Read): number | undefined | Read {
  if (platform === "linux") return linuxStartedAt(pid);
  return platform === "win32" ? windows() : undefined;
}

/** What the engine reads of when `pid` started; nothing when it does not load or has no such read. */
function engineStartedAt(pid: number, engine: () => Pick<NativeBinding, "processStartedAt">): number | undefined {
  try {
    return engineStartRead(engine(), pid) ?? undefined;
  } catch (error) {
    reportEngineReadFailure(error);
    return undefined;
  }
}

/** What the loaded engine `binding` reads of when `pid` started; it throws when the engine was built before that read. */
function engineStartRead(binding: Pick<NativeBinding, "processStartedAt">, pid: number): number | null {
  const read = binding.processStartedAt;
  if (read === undefined) throw new Error("the engine was built before it could read a process's start");
  return read(pid);
}

/** Whether a failure of the engine's process reads was reported in this process. */
let engineReadFailureReported = false;

/**
 * Writes the first failure of the engine's process reads to stderr, which a daemon keeps in its log, once per process:
 * without them neither a busy daemon nor one of Rerail 0.1.4 or earlier is told from a stale record by its start, and
 * the latter is not ended. The line names neither, since every Windows start read goes through the engine.
 */
function reportEngineReadFailure(error: unknown): void {
  if (engineReadFailureReported) return;
  engineReadFailureReported = true;
  const reason = error instanceof Error ? error.message : String(error);
  process.stderr.write(`rerail: the engine could not read or end a process, so Rerail could not tell whether the recorded daemon is still running: ${reason}\n`);
}

/**
 * What ending a daemon of a release before the engine's proof through its process came to ({@link endProcessStartedBy}):
 * it exited (`ended`), it was ended and still runs after the wait (`running`), the process holding the pid started
 * after the record's `startedAt` and was let alone (`later`), or the process could not be opened, its start read or
 * waited on, as when no process holds the pid, or `startedAt` names no time a process started ({@link startNamesProcess})
 * (`unread`), or the child process that asked the engine was ended at its bound before it answered, so whether the end
 * was asked is not known (`unanswered`, {@link endProcessStartedByInChild}). A wait that failed came after the end was
 * asked, so an `unread` process may have been ended.
 */
export type ProcessEnd = "ended" | "running" | "later" | "unread" | "unanswered";

const PROCESS_ENDS: ReadonlySet<string> = new Set(["ended", "running", "later"]);

/**
 * Windows: ends the process holding `pid` when it started no later than `startedAt` (epoch ms), and waits up to
 * `waitMs` for it to exit, through the engine's one call (`endProcessStartedBy`), which opens the process, reads its
 * start and ends it through that one handle: the process ended is the one whose start was read, whatever process
 * Windows gives the pid between two calls. An engine that does not load, or was built before it, ends nothing.
 */
export function endProcessStartedBy(
  pid: number,
  startedAt: number,
  waitMs: number,
  engine: () => Pick<NativeBinding, "endProcessStartedBy"> = loadNativeBinding,
): ProcessEnd {
  try {
    return engineEnd(engine(), { pid, startedAt, waitMs });
  } catch (error) {
    reportEngineReadFailure(error);
    return "unread";
  }
}

/** A process to end when it started no later than `startedAt` (epoch ms), with how long to wait for its exit. */
export interface EndTarget {
  readonly pid: number;
  readonly startedAt: number;
  readonly waitMs: number;
}

/** What the loaded engine `binding` answers to its end of `target`; it throws when the engine was built before that end. */
function engineEnd(binding: Pick<NativeBinding, "endProcessStartedBy">, { pid, startedAt, waitMs }: EndTarget): ProcessEnd {
  const end = binding.endProcessStartedBy;
  if (end === undefined) throw new Error("the engine was built before it could end a process by its start");
  return processEnd(end(pid, startedAt, waitMs));
}

/**
 * {@link endProcessStartedBy} through Windows PowerShell, which loads no Rerail engine, for the uninstall: the process
 * object `Get-Process` returns keeps the handle it opened for `.Handle` and reads its start, ends it and waits on that
 * one handle for the end, with cmdlets and properties alone, which Constrained Language Mode allows. Each cmdlet is named
 * with its module, as {@link windowsStartedAt}'s is, so PowerShell reads no other installed module first. The wait has
 * no limit of its own: the end is a fact the handle gives. `waitMs` bounds the whole call, PowerShell's own start
 * included, only so that a PowerShell or an end that hangs cannot hold the uninstall for good. It prints
 * {@link END_ASKED} once it has ended the process, so a PowerShell ended at the bound after that reads `running`; one
 * ended before it, or one that cannot run, ends nothing it reports: `unread`.
 */
export function endProcessStartedByPowerShell(pid: number, startedAt: number, waitMs: number): ProcessEnd {
  // A start that names no time a process started ends nothing, as the engine's end answers.
  if (!startNamesProcess(startedAt)) return "unread";
  const latest = (BigInt(Math.floor(startedAt)) + BigInt(FILETIME_EPOCH_MS)) * 10_000n + 9_999n;
  const script = `$p = ${PROCESS_CMDLETS}\\Get-Process -Id ${pid} -ErrorAction Stop; $null = $p.Handle; `
    + `if ($p.StartTime.ToFileTimeUtc() -gt ${latest}) { 'later' } else { ${PROCESS_CMDLETS}\\Stop-Process -InputObject $p -Force -ErrorAction Stop; `
    + `'${END_ASKED}'; ${PROCESS_CMDLETS}\\Wait-Process -InputObject $p; 'ended' }`;
  const run = spawnSync(windowsPowerShell(), ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8", timeout: waitMs });
  return powerShellEnd(run.status, String(run.stdout ?? ""));
}

/**
 * A read of a process's start that was ended at its bound before it answered: unlike a read that answered with no start,
 * it says nothing of the process, and a stop that meets it says so.
 */
export const START_UNANSWERED = "unanswered";

/** What a read of when a process started gives: the start in epoch ms, none, or {@link START_UNANSWERED}. */
export type StartRead = number | undefined | typeof START_UNANSWERED;

/** The Node binary and the entry file of the child process that reads through the engine ({@link engineReaderAnswer}). */
export interface EngineReader {
  readonly node: string;
  readonly entry: string;
}

/**
 * How long a child that reads through the engine has to answer, its Node's start and the engine's load included. The
 * read is a system call, so on a machine that runs the child this bound decides nothing: it only ends a child that never
 * answers, as one a debugger holds, so that it cannot hold the uninstall for good.
 */
export const ENGINE_READER_ANSWER_MS = 30_000;

/**
 * {@link processStartedAt} for a process that must not load the engine, as the uninstall must not before it deletes
 * `versions/`: Windows reads the start through the engine in a child Node process (`reader`), which has exited when
 * this returns, so no process of the command holds the engine's file afterwards. A child that gives no answer, as when
 * the engine does not load or the entry file is missing, leaves the read to `fallback`; one still running at
 * {@link ENGINE_READER_ANSWER_MS} is ended there and reads {@link START_UNANSWERED}.
 */
export function processStartedAtInChild(pid: number, reader: EngineReader, fallback: () => StartRead, platform: NodeJS.Platform = process.platform): StartRead {
  return startedAt(pid, platform, () => {
    const answer = askEngineReader(reader, ["start", String(pid)], ENGINE_READER_ANSWER_MS);
    if (answer === START_UNANSWERED) return START_UNANSWERED;
    if (answer === undefined || !("startedAt" in answer)) return fallback();
    return typeof answer.startedAt === "number" ? answer.startedAt : undefined;
  });
}

/**
 * {@link endProcessStartedBy} for a process that must not load the engine, through the engine in a child Node process
 * (`reader`), as {@link processStartedAtInChild} reads. The child has the engine's wait for the exit on top of
 * {@link ENGINE_READER_ANSWER_MS}; one still running then is ended and its end reads `unanswered`. A child that gives no
 * answer leaves the end to `fallback`.
 */
export function endProcessStartedByInChild(target: EndTarget, reader: EngineReader, fallback: () => ProcessEnd): ProcessEnd {
  // A start that names no time a process started ends nothing, as the engine's end answers.
  if (!startNamesProcess(target.startedAt)) return "unread";
  const args = ["end", String(target.pid), String(target.startedAt), String(target.waitMs)];
  const answer = askEngineReader(reader, args, ENGINE_READER_ANSWER_MS + target.waitMs);
  if (answer === START_UNANSWERED) return "unanswered";
  return answer === undefined || !("end" in answer) ? fallback() : processEnd(String(answer.end));
}

/**
 * What the child `reader` printed for `args` as its last line, parsed: undefined when it exited otherwise than with 0
 * and an object, and {@link START_UNANSWERED} when it was still running at `timeoutMs` and ended there. Its stderr is
 * this process's, so a person sees why an engine did not load.
 */
function askEngineReader(reader: EngineReader, args: readonly string[], timeoutMs: number): Record<string, unknown> | typeof START_UNANSWERED | undefined {
  const run = spawnSync(reader.node, [reader.entry, ...args], { encoding: "utf8", timeout: timeoutMs, stdio: ["ignore", "pipe", "inherit"] });
  if ((run.error as NodeJS.ErrnoException | undefined)?.code === "ETIMEDOUT") return START_UNANSWERED;
  if (run.status !== 0) return undefined;
  try {
    const answer: unknown = JSON.parse(String(run.stdout ?? "").trim().split(/\r?\n/u).at(-1) ?? "");
    return typeof answer === "object" && answer !== null ? answer as Record<string, unknown> : undefined;
  } catch {
    return undefined;
  }
}

/** What the process reader prints and its exit code ({@link engineReaderAnswer}). */
export interface EngineReaderAnswer {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode: number;
}

/**
 * What the process reader (`src/engine/process-reader.ts`) answers to `args` through `engine`: `start <pid>` prints
 * `{"startedAt":<epoch ms or null>}`, `end <pid> <startedAt> <waitMs>` prints `{"end":"<ProcessEnd>"}`, each with exit
 * code 0. An engine that does not load, or was built before the call, answers nothing and exits 3, its reason on
 * stderr, so the reader's caller reads another way.
 */
export function engineReaderAnswer(args: readonly string[], engine: () => Pick<NativeBinding, "processStartedAt" | "endProcessStartedBy">): EngineReaderAnswer {
  const [kind, ...numbers] = args;
  const [pid = Number.NaN, startedAt = Number.NaN, waitMs = Number.NaN] = numbers.map(Number);
  if (kind !== "start" && kind !== "end") return { stdout: "", stderr: `rerail: the process reader takes start or end, not ${kind}\n`, exitCode: 2 };
  try {
    const answer = kind === "start" ? { startedAt: engineStartRead(engine(), pid) } : { end: engineEnd(engine(), { pid, startedAt, waitMs }) };
    return { stdout: `${JSON.stringify(answer)}\n`, stderr: "", exitCode: 0 };
  } catch (error) {
    return { stdout: "", stderr: `rerail: the engine could not read or end a process: ${error instanceof Error ? error.message : String(error)}\n`, exitCode: 3 };
  }
}

/** What {@link endProcessStartedByPowerShell} prints once it has ended the process, before it waits for the exit. */
const END_ASKED = "asked";

/** The module of the process cmdlets Rerail runs through Windows PowerShell, which names each one with it. */
const PROCESS_CMDLETS = "Microsoft.PowerShell.Management";

/**
 * The {@link ProcessEnd} a run of {@link endProcessStartedByPowerShell} that exited with `status` came to by what it
 * printed (`stdout`): its last line once it exited, `running` when it was ended after it ended the process and before
 * that process exited, and `unread` otherwise.
 */
function powerShellEnd(status: number | null, stdout: string): ProcessEnd {
  const lines = stdout.trim().split(/\r?\n/u);
  if (status === 0) return processEnd(lines.at(-1) ?? "");
  return lines.includes(END_ASKED) ? "running" : "unread";
}

/** The {@link ProcessEnd} an engine or PowerShell answered; anything else is `unread`. */
function processEnd(answer: string): ProcessEnd {
  return PROCESS_ENDS.has(answer) ? answer as ProcessEnd : "unread";
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
 * script policies impose that mode, and it refuses .NET's static methods. The cmdlet is named with its module, so
 * PowerShell loads that module by its name: a cmdlet named alone makes it first read every installed module its cache
 * has not read yet, which took 46 s with 40 large ones. It prints a FILETIME, 100 ns ticks since 1601 in UTC, past a
 * double's exact integers, so it is divided as a BigInt. The read runs synchronously, so PowerShell is ended at
 * `timeoutMs` and not started with no time left (Node reads a timeout of 0 as none). A process this user may not query,
 * a PowerShell that cannot run and one ended at the bound read nothing.
 */
function windowsStartedAt(pid: number, timeoutMs: number): number | undefined {
  if (!(timeoutMs > 0)) return undefined;
  const read = spawnSync(windowsPowerShell(), ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
    `(${PROCESS_CMDLETS}\\Get-Process -Id ${pid} -ErrorAction Stop).StartTime.ToFileTimeUtc()`],
  { encoding: "utf8", timeout: timeoutMs });
  const text = String(read.stdout ?? "").trim();
  return read.status === 0 && /^\d+$/u.test(text) ? Number(BigInt(text) / 10_000n) - FILETIME_EPOCH_MS : undefined;
}
