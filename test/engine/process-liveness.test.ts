import * as childProcess from "node:child_process";
import { readFileSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import * as native from "../../src/engine/native/native.js";
import {
  endProcessStartedBy,
  endProcessStartedByInChild,
  endProcessStartedByPowerShell,
  ENGINE_READER_ANSWER_MS,
  type EngineReader,
  engineReaderAnswer,
  exitedEveryThread,
  type ProcessEnd,
  processStartedAt,
  processStartedAtInChild,
  processStartedAtThroughEngine,
  processState,
  type ProcessSignal,
  START_UNANSWERED,
  type StartRead,
} from "../../src/engine/process-liveness.js";
import { cmdletsWithoutModule, commandScript, MODULE_QUALIFIED_ONLY, withCommandPrefix } from "../support/platform/powershell-commands.js";
import { exitedEveryThread as removalExitedEveryThread, removeFolder } from "../support/platform/remove-folder.js";

// Spies that call through, so a test can stand a program in for Windows PowerShell, and see whether the engine loads.
vi.mock("node:child_process", { spy: true });
vi.mock("../../src/engine/native/native.js", { spy: true });

/** A pid no live process can hold on the platforms Rerail supports. */
const DEAD_PID = 2 ** 22 - 1;

/**
 * A case's own guard against a real Windows PowerShell that never answers, as one held by a prompt or a locked module
 * cache: `spawnSync` blocks the worker, so the case's 60 s limit cannot end the run. It is not the product's bound,
 * which the case asserts apart, and below it how fast this machine starts PowerShell decides nothing.
 */
const POWERSHELL_GUARD_MS = 50_000;

const refusing = (code: string): ProcessSignal => () => {
  throw Object.assign(new Error(`kill ${code}`), { code });
};

describe("processState", () => {
  it.each([
    ["answers signal 0", "running", () => true],
    // Another user's process or a service; on Windows also this user's elevated process.
    ["refuses it with EPERM", "running-not-signalable", refusing("EPERM")],
    ["refuses it with ESRCH", "gone", refusing("ESRCH")],
  ] as const)("reads a pid whose kill %s as %s", (_case, state, kill) => {
    expect(processState(DEAD_PID, kill)).toBe(state);
  });

  it("finds this process running and a pid no process holds gone", () => {
    expect(processState(process.pid)).toBe("running");
    expect(processState(DEAD_PID)).toBe("gone");
  });

  it.runIf(process.platform === "linux")("reads a process reaped between its signal and the read of its /proc entry as gone", () => {
    // Signal 0 finds it, a zombie not yet reaped; its parent reaps it, and the /proc entry and the pid go with it.
    let signals = 0;
    const reapedAfterFirst: ProcessSignal = () => {
      signals += 1;
      if (signals === 1) return true;
      throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
    };
    expect(processState(DEAD_PID, reapedAfterFirst)).toBe("gone");
  });

  it.runIf(process.platform === "linux")("reads a process whose main thread exited as running while another thread runs on, and gone once that one has", async () => {
    // Python's main thread ends itself and leaves a thread that sleeps: /proc reads the process `Z`, while the thread that
    // runs on still holds every file, socket and lock the process opened.
    const script = "import ctypes, threading, time\nthreading.Thread(target=time.sleep, args=(60,)).start()\nctypes.CDLL(None).pthread_exit(None)";
    const child = spawn("python3", ["-c", script], { stdio: "ignore" });
    const exited = once(child, "exit");
    const pid = child.pid!;
    const status = (): string => readFileSync(`/proc/${pid}/status`, "utf8");
    try {
      // Within the test's 5 s.
      await vi.waitFor(() => expect(status()).toMatch(/^State:\s*Z/mu), { timeout: 3_000, interval: 10 });
      expect(status()).toMatch(/^Threads:\s*2$/mu);
      expect(processState(pid)).toBe("running");
      child.kill("SIGKILL");
      // Node reaps the child only on a later turn of the event loop, so this loop sees the zombie it became once the
      // thread that ran on has exited too.
      const deadline = Date.now() + 1_000;
      while (/^Threads:\s*2$/mu.test(status()) && Date.now() < deadline) {
        // The killed thread exits on the system's schedule.
      }
      expect(processState(pid)).toBe("gone");
    } finally {
      child.kill("SIGKILL");
      await exited;
    }
  });

  it("reads a Linux status as exited once every thread has, as the test folders' removal reads it", () => {
    const status = (state: string, threads?: number): string =>
      `Name:\tnode\nState:\t${state}\nTgid:\t7\n${threads === undefined ? "" : `Threads:\t${threads}\n`}`;
    const cases = [
      [status("Z (zombie)", 1), true],
      [status("X (dead)", 1), true],
      [status("Z (zombie)"), true],
      [status("Z (zombie)", 2), false],
      [status("S (sleeping)", 1), false],
      [status("R (running)", 4), false],
    ] as const;
    expect(cases.map(([text]) => exitedEveryThread(text))).toEqual(cases.map(([, exited]) => exited));
    expect(cases.map(([text]) => removalExitedEveryThread(text))).toEqual(cases.map(([, exited]) => exited));
  });
});

describe("processStartedAt", () => {
  // Linux reads /proc, Windows the creation time; other systems read none.
  it.runIf(process.platform === "linux" || process.platform === "win32")("reads when this process started, and a process started after it as later", async () => {
    const started = processStartedAt(process.pid, 30_000);
    expect(started).toBeDefined();
    expect(Math.abs(started! - (Date.now() - process.uptime() * 1_000))).toBeLessThan(1_000);
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
    try {
      expect(processStartedAt(child.pid!, 30_000)).toBeGreaterThan(started!);
    } finally {
      const closed = once(child, "close");
      child.kill();
      await closed;
    }
  });

  it("reads no start for a pid no process holds, nor on a system it does not read", () => {
    expect(processStartedAt(DEAD_PID, 30_000)).toBeUndefined();
    expect(processStartedAt(process.pid, 30_000, "darwin")).toBeUndefined();
  });
});

describe("processStartedAt on Windows, with a program standing in for Windows PowerShell", () => {
  /** Runs `program` with `args` in place of Windows PowerShell, with the options the reader gave; the options go to `seen`. */
  async function standIn(program: string, args: readonly string[], seen: childProcess.SpawnSyncOptions[] = []): Promise<void> {
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.spawnSync).mockImplementationOnce(((_program: string, _args: readonly string[], options: childProcess.SpawnSyncOptions) => {
      seen.push(options);
      return runSync(program, args, options);
    }) as unknown as typeof childProcess.spawnSync);
  }

  // A FILETIME counts 100 ns ticks from 1601-01-01 UTC; this one is 1,234 ticks past 1_700_000_000_000 ms since 1970,
  // more than a double holds exactly.
  // The uninstall reads this way before it deletes the folder the engine loads from, which a loaded DLL would keep.
  it("reads the creation time PowerShell prints as a FILETIME, in epoch milliseconds, and loads no engine", async () => {
    const loads = vi.mocked(native.loadNativeBinding).mock.calls.length;
    await standIn(process.execPath, ["-e", "process.stdout.write('133444736000001234\\r\\n')"]);
    expect(processStartedAt(process.pid, 30_000, "win32")).toBe(1_700_000_000_000);
    expect(vi.mocked(native.loadNativeBinding).mock.calls.length).toBe(loads);
  });

  // AppLocker and WDAC script policies run PowerShell in Constrained Language Mode, which allows cmdlets and the methods
  // of core types only. The real PowerShell runs the reader's command with that mode set first.
  it.runIf(process.platform === "win32")("reads when this process started under PowerShell's Constrained Language Mode", async () => {
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.spawnSync).mockImplementationOnce(((program: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => {
      const constrained = args.map((arg, at) => (args[at - 1] === "-Command" ? `$ExecutionContext.SessionState.LanguageMode = 'ConstrainedLanguage'; ${arg}` : arg));
      return runSync(program, constrained, options);
    }) as unknown as typeof childProcess.spawnSync);
    const started = processStartedAt(process.pid, 30_000);
    expect(started).toBeDefined();
    expect(Math.abs(started! - (Date.now() - process.uptime() * 1_000))).toBeLessThan(1_000);
  });

  // A cmdlet named alone makes Windows PowerShell read every installed module it has not read yet before it runs: 46 s
  // with 40 large ones, past every limit a caller gives the read.
  it("names the module of the cmdlet it runs, so PowerShell reads no other installed module first", async () => {
    const scripts: Array<string | undefined> = [];
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.spawnSync).mockImplementationOnce(((_program: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => {
      scripts.push(commandScript(args));
      return runSync(process.execPath, ["-e", "process.stdout.write('133444736000001234')"], options);
    }) as unknown as typeof childProcess.spawnSync);
    expect(processStartedAt(process.pid, 30_000, "win32")).toBe(1_700_000_000_000);
    expect(scripts).toHaveLength(1);
    expect(scripts[0]).toMatch(/\\Get-Process /u);
    expect(cmdletsWithoutModule(scripts[0] ?? "")).toEqual([]);
  });

  it.runIf(process.platform === "win32")("reads when this process started while PowerShell loads only the modules a command names", async () => {
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.spawnSync).mockImplementationOnce(((program: string, args: readonly string[], options: childProcess.SpawnSyncOptions) =>
      runSync(program, withCommandPrefix(args, MODULE_QUALIFIED_ONLY), options)) as unknown as typeof childProcess.spawnSync);
    const started = processStartedAt(process.pid, 30_000);
    expect(started).toBeDefined();
    expect(Math.abs(started! - (Date.now() - process.uptime() * 1_000))).toBeLessThan(1_000);
  });

  // The setting stands in for a machine with many modules only while a cmdlet named alone is then not found. Were
  // Microsoft.PowerShell.Management loaded before the command starts, the case above, and the stop's case in
  // daemon-control.test.ts, would pass with the module's name taken out of the read.
  it.runIf(process.platform === "win32")("reads nothing under that setting when the cmdlet is named alone, as before its module was named", async () => {
    const scripts: Array<string | undefined> = [];
    const results: Array<childProcess.SpawnSyncReturns<string | Buffer>> = [];
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.spawnSync).mockImplementationOnce(((program: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => {
      const alone = args.map(arg => arg.replace("Microsoft.PowerShell.Management\\Get-Process", "Get-Process"));
      scripts.push(commandScript(alone));
      const result = runSync(program, withCommandPrefix(alone, MODULE_QUALIFIED_ONLY), options);
      results.push(result);
      return result;
    }) as unknown as typeof childProcess.spawnSync);
    expect(processStartedAt(process.pid, 30_000)).toBeUndefined();
    expect(cmdletsWithoutModule(scripts[0] ?? "")).toEqual(["Get-Process"]);
    // PowerShell ran and did not find the cmdlet: the read failed for that, not for a start that failed or ran out of time.
    expect(results).toHaveLength(1);
    expect(String(results[0]?.stderr)).toMatch(/CommandNotFoundException/u);
  });

  it("ends a PowerShell that does not answer at its bound and reads nothing", async () => {
    const seen: childProcess.SpawnSyncOptions[] = [];
    await standIn(process.execPath, ["-e", "setTimeout(() => {}, 60_000)"], seen);
    const asked = Date.now();
    expect(processStartedAt(process.pid, 500, "win32")).toBeUndefined();
    expect(Date.now() - asked).toBeLessThan(10_000);
    expect(seen.map(options => options.timeout)).toEqual([500]);
  });

  // The uninstall ends a daemon of Rerail 0.1.4 or earlier this way, under the policy's mode too: the process object
  // reads the start, ends the process and waits through the one handle it opened, so a pid that changed hands is spared.
  it.runIf(process.platform === "win32")("ends a process through PowerShell only when it started no later than the time given", async () => {
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    const constrained = ((program: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => runSync(
      program, args.map((arg, at) => (args[at - 1] === "-Command" ? `$ExecutionContext.SessionState.LanguageMode = 'ConstrainedLanguage'; ${arg}` : arg)), options,
    )) as unknown as typeof childProcess.spawnSync;
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
    const closed = once(child, "close");
    try {
      const started = processStartedAtThroughEngine(child.pid!)!;
      vi.mocked(childProcess.spawnSync).mockImplementationOnce(constrained);
      expect(endProcessStartedByPowerShell(child.pid!, started - 1, 5_000)).toBe("later");
      expect(child.exitCode).toBeNull();
      vi.mocked(childProcess.spawnSync).mockImplementationOnce(constrained);
      expect(endProcessStartedByPowerShell(child.pid!, started, 5_000)).toBe("ended");
      await closed;
    } finally {
      child.kill();
    }
  });

  // A cmdlet named alone makes Windows PowerShell read every installed module it has not read yet before it runs: 46 s
  // with 40 large ones, past the uninstall's 15 s bound on the end, which would then end nothing.
  it("names the module of each cmdlet the end runs, so PowerShell reads no other installed module first", async () => {
    const scripts: Array<string | undefined> = [];
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.spawnSync).mockImplementationOnce(((_program: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => {
      scripts.push(commandScript(args));
      return runSync(process.execPath, ["-e", "process.stdout.write('asked\\r\\nended\\r\\n')"], options);
    }) as unknown as typeof childProcess.spawnSync);
    expect(endProcessStartedByPowerShell(DEAD_PID, Date.now(), 5_000)).toBe("ended");
    expect(scripts).toHaveLength(1);
    for (const cmdlet of ["Get-Process", "Stop-Process", "Wait-Process"]) expect(scripts[0]).toContain(`Microsoft.PowerShell.Management\\${cmdlet} `);
    expect(cmdletsWithoutModule(scripts[0] ?? "")).toEqual([]);
  });

  // PowerShell's own setting that loads only the modules a command names stands in for a computer with many modules: a
  // cmdlet named alone is not found under it. PowerShell runs to its answer, so how fast this machine starts it decides
  // nothing; the bound it was given, the uninstall's 15 s (its stop's wait and its kill wait), goes to `bounds`. The
  // case that ends a PowerShell that does not answer covers that bound; {@link POWERSHELL_GUARD_MS} ends one that never
  // answers here. The case's time gives the engine's first load in its worker room: on a loaded machine that load took
  // 27 s.
  it.runIf(process.platform === "win32")("ends a process through PowerShell, given the uninstall's bound, while PowerShell loads only the modules a command names", async () => {
    const bounds: unknown[] = [];
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.spawnSync).mockImplementationOnce(((program: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => {
      bounds.push(options.timeout);
      const result = runSync(program, withCommandPrefix(args, MODULE_QUALIFIED_ONLY), { ...options, timeout: POWERSHELL_GUARD_MS });
      const failed: NodeJS.ErrnoException | undefined = result.error;
      if (failed?.code === "ETIMEDOUT") throw new Error(`Windows PowerShell did not answer within the case's own ${POWERSHELL_GUARD_MS / 1_000} s guard`);
      return result;
    }) as unknown as typeof childProcess.spawnSync);
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
    const closed = once(child, "close");
    try {
      const started = processStartedAtThroughEngine(child.pid!)!;
      expect(endProcessStartedByPowerShell(child.pid!, started, 15_000)).toBe("ended");
      expect(bounds).toEqual([15_000]);
    } finally {
      child.kill();
      await closed;
    }
  }, 60_000);

  // The end waits on the process's handle with no limit of its own; only a PowerShell or an end that hangs meets the
  // call's one bound, the stop's wait and its kill wait, and is ended there.
  it("ends a PowerShell whose end of a process does not answer at the stop's wait and its kill wait, and reports nothing ended", async () => {
    const seen: childProcess.SpawnSyncOptions[] = [];
    await standIn(process.execPath, ["-e", "setTimeout(() => {}, 60_000)"], seen);
    const asked = Date.now();
    expect(endProcessStartedByPowerShell(process.pid, Date.now(), 500)).toBe("unread");
    expect(Date.now() - asked).toBeLessThan(10_000);
    expect(seen.map(options => options.timeout)).toEqual([500]);
  });

  // One ended at the bound once it said it ended the process, as while it waits for the exit, has ended it.
  it("reads a PowerShell ended at its bound after it ended the process as an end still running", { timeout: 30_000 }, async () => {
    await standIn(process.execPath, ["-e", "process.stdout.write('asked\\r\\n'); setTimeout(() => {}, 60_000)"]);
    expect(endProcessStartedByPowerShell(process.pid, Date.now(), 3_000)).toBe("running");
  });

  // The real PowerShell prints that it ended the process before it waits: a wait that never ends stands in for the exit's.
  it.runIf(process.platform === "win32")("says through PowerShell that it ended a process before it waits for the exit", async () => {
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
    const closed = once(child, "close");
    try {
      // Its start as Windows records it: this process's clock can read a moment before it.
      const started = processStartedAtThroughEngine(child.pid!)!;
      vi.mocked(childProcess.spawnSync).mockImplementationOnce(((program: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => runSync(
        program, args.map(arg => arg.replace(String.raw`Microsoft.PowerShell.Management\Wait-Process -InputObject $p`, String.raw`Microsoft.PowerShell.Utility\Start-Sleep -Seconds 60`)), options,
      )) as unknown as typeof childProcess.spawnSync);
      expect(endProcessStartedByPowerShell(child.pid!, started, 8_000)).toBe("running");
      await closed;
    } finally {
      child.kill();
    }
  }, 30_000);

  // A start that is no finite number names no time a process started: the uninstall's end answers as the engine's does,
  // ends nothing and starts no PowerShell.
  it.each([Infinity, -Infinity, Number.NaN])("ends nothing through PowerShell for a start of %s, and starts no PowerShell", startedAt => {
    const calls = vi.mocked(childProcess.spawnSync).mock.calls.length;
    expect(endProcessStartedByPowerShell(process.pid, startedAt, 5_000)).toBe("unread");
    expect(vi.mocked(childProcess.spawnSync).mock.calls.length).toBe(calls);
  });

  // A start an hour past this machine's clock comes after every live process's start, so any process at the pid would
  // pass for the one it names: the end answers as the engine's does, and starts no PowerShell. One 5 s past it, as a
  // clock set back since the daemon stamped it leaves it, still names a start.
  it("ends nothing through PowerShell for a start an hour past this machine's clock, and starts no PowerShell", async () => {
    const calls = vi.mocked(childProcess.spawnSync).mock.calls.length;
    expect(endProcessStartedByPowerShell(DEAD_PID, Date.now() + 3_600_000, 5_000)).toBe("unread");
    expect(vi.mocked(childProcess.spawnSync).mock.calls.length).toBe(calls);
    await standIn(process.execPath, ["-e", "process.stdout.write('asked\\r\\nended\\r\\n')"]);
    expect(endProcessStartedByPowerShell(DEAD_PID, Date.now() + 5_000, 5_000)).toBe("ended");
  });

  it("reads nothing from a PowerShell the machine cannot run, and starts none with no time left", async () => {
    await standIn(join(tmpdir(), "rerail-no-such-powershell", "powershell.exe"), []);
    expect(processStartedAt(process.pid, 30_000, "win32")).toBeUndefined();
    const calls = vi.mocked(childProcess.spawnSync).mock.calls.length;
    expect(processStartedAt(process.pid, 0, "win32")).toBeUndefined();
    expect(vi.mocked(childProcess.spawnSync).mock.calls.length).toBe(calls);
  });
});

describe("processStartedAtThroughEngine", () => {
  /**
   * An engine that answers every pid with a start and records each pid it is asked for. A read on a system the engine
   * does not serve shows by both that it never asked: an engine that throws could not show it, since the read takes any
   * throw for no answer.
   */
  const answeringEngine = (asked: number[]) => () => ({
    processStartedAt: (pid: number) => {
      asked.push(pid);
      return 1_700_000_000_000;
    },
  });

  it("reads a Windows process's start through the engine, which starts no process", () => {
    const spawned = vi.mocked(childProcess.spawnSync).mock.calls.length;
    const asked: number[] = [];
    const engine = () => ({
      processStartedAt: (pid: number) => {
        asked.push(pid);
        return pid === 7 ? 1_700_000_000_000 : null;
      },
    });
    expect(processStartedAtThroughEngine(7, "win32", engine)).toBe(1_700_000_000_000);
    expect(processStartedAtThroughEngine(8, "win32", engine)).toBeUndefined();
    expect(asked).toEqual([7, 8]);
    expect(vi.mocked(childProcess.spawnSync).mock.calls.length).toBe(spawned);
  });

  // A system call needs no limit: a lookup whose 2 s an engine slow to the proof used up still reads the start, so a
  // daemon of Rerail 0.1.4 or earlier whose 404 came late is found, not dropped.
  it("reads a Windows process's start through the engine however little of a lookup's limit is left", () => {
    const asked: number[] = [];
    expect(processStartedAtThroughEngine(7, "win32", answeringEngine(asked))).toBe(1_700_000_000_000);
    expect(asked).toEqual([7]);
  });

  // A daemon keeps its stderr in its log, and the first failure says why, once per process. Every Windows start read
  // goes through the engine, a busy daemon's of any release as well as one of 0.1.4 or earlier, so the line names no
  // release.
  it("reports the first failure of the engine's read once, and reads nothing", async () => {
    vi.resetModules();
    const fresh = await import("../../src/engine/process-liveness.js");
    const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      const failing = () => {
        throw new Error("Unsupported platform or missing native addon");
      };
      const line = "rerail: the engine could not read or end a process, so Rerail could not tell whether the recorded daemon is still running: Unsupported platform or missing native addon\n";
      expect(fresh.processStartedAtThroughEngine(7, "win32", failing)).toBeUndefined();
      expect(written.mock.calls.map(([text]) => String(text))).toEqual([line]);
      expect(fresh.endProcessStartedBy(7, 1_700_000_000_000, 5_000, failing)).toBe("unread");
      expect(written.mock.calls.map(([text]) => String(text))).toEqual([line]);
    } finally {
      written.mockRestore();
    }
  });

  it("ends a process through the engine's one call, and takes any other answer, or none, for unread", () => {
    const calls: number[][] = [];
    const engine = (answer: string) => () => ({
      endProcessStartedBy: (pid: number, startedAt: number, waitMs: number) => {
        calls.push([pid, startedAt, waitMs]);
        return answer;
      },
    });
    expect(endProcessStartedBy(7, 1_700_000_000_000, 5_000, engine("ended"))).toBe("ended");
    expect(endProcessStartedBy(7, 1_700_000_000_000, 5_000, engine("later"))).toBe("later");
    expect(endProcessStartedBy(7, 1_700_000_000_000, 5_000, engine("gone"))).toBe("unread");
    expect(endProcessStartedBy(7, 1_700_000_000_000, 5_000, () => ({}))).toBe("unread");
    expect(calls).toEqual([[7, 1_700_000_000_000, 5_000], [7, 1_700_000_000_000, 5_000], [7, 1_700_000_000_000, 5_000]]);
  });

  it("reads nothing where the engine does not load or was built before the read", () => {
    expect(processStartedAtThroughEngine(process.pid, "win32", () => {
      throw new Error("Unsupported platform or missing native addon");
    })).toBeUndefined();
    expect(processStartedAtThroughEngine(process.pid, "win32", () => ({}))).toBeUndefined();
  });

  it("reads no start on a system it does not read, and asks the engine nothing", () => {
    const asked: number[] = [];
    expect(processStartedAtThroughEngine(process.pid, "darwin", answeringEngine(asked))).toBeUndefined();
    expect(asked).toEqual([]);
  });

  it.runIf(process.platform === "linux")("reads Linux's start from /proc, and asks the engine nothing", () => {
    const asked: number[] = [];
    // As processStartedAt does, when a lookup would load the engine to ask it.
    // Each read counts the start back from the clock at its own moment, so the reads can differ by the time they took
    // and by /proc's 10 ms resolution, and no more.
    const before = Date.now();
    const started = processStartedAtThroughEngine(process.pid, "linux", answeringEngine(asked));
    const again = processStartedAt(process.pid, 0, "linux");
    const took = Date.now() - before;
    expect(started).toBeDefined();
    expect(Math.abs(started! - again!)).toBeLessThanOrEqual(took + 20);
    expect(asked).toEqual([]);
  });

  it.runIf(process.platform === "win32")("reads when this process and a later one started through the engine built here", async () => {
    const spawned = vi.mocked(childProcess.spawnSync).mock.calls.length;
    // The order of facts decides, not how long anything took: this process was created before the case ran, and the
    // child after the case asked for it. Windows stamps a creation with a clock that can read a timer tick (15.6 ms by
    // default) apart from Node's, so the child's lower bound allows two ticks.
    const asked = Date.now();
    const started = processStartedAtThroughEngine(process.pid);
    expect(started).toBeDefined();
    expect(started!).toBeLessThanOrEqual(asked);
    const spawning = Date.now();
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
    try {
      const childStarted = processStartedAtThroughEngine(child.pid!);
      expect(childStarted).toBeGreaterThan(started!);
      expect(childStarted!).toBeGreaterThanOrEqual(spawning - 32);
    } finally {
      const closed = once(child, "close");
      child.kill();
      await closed;
    }
    expect(processStartedAtThroughEngine(DEAD_PID)).toBeUndefined();
    expect(vi.mocked(childProcess.spawnSync).mock.calls.length).toBe(spawned);
  });
});

describe("the process reader a process that must not load the engine reads through", () => {
  const engine = (startedAt: number | null, end = "ended") => () => ({ processStartedAt: () => startedAt, endProcessStartedBy: () => end });

  it("prints the engine's start, or its end, as one line, and exits 0", () => {
    expect(engineReaderAnswer(["start", "7"], engine(1_700_000_000_000))).toEqual({ stdout: "{\"startedAt\":1700000000000}\n", stderr: "", exitCode: 0 });
    expect(engineReaderAnswer(["start", "7"], engine(null))).toEqual({ stdout: "{\"startedAt\":null}\n", stderr: "", exitCode: 0 });
    expect(engineReaderAnswer(["end", "7", "1700000000000", "5000"], engine(null, "later"))).toEqual({ stdout: "{\"end\":\"later\"}\n", stderr: "", exitCode: 0 });
    expect(engineReaderAnswer(["end", "7", "1700000000000", "5000"], engine(null, "gone"))).toEqual({ stdout: "{\"end\":\"unread\"}\n", stderr: "", exitCode: 0 });
  });

  // Its caller then reads another way: an install whose engine is missing or broken must still be removable.
  it("answers nothing and exits 3, saying why, where the engine does not load or was built before the call", () => {
    const missing = engineReaderAnswer(["start", "7"], () => {
      throw new Error("Unsupported platform or missing native addon");
    });
    expect(missing).toEqual({ stdout: "", stderr: "rerail: the engine could not read or end a process: Unsupported platform or missing native addon\n", exitCode: 3 });
    expect(engineReaderAnswer(["end", "7", "1700000000000", "5000"], () => ({}))).toMatchObject({ stdout: "", exitCode: 3 });
    expect(engineReaderAnswer(["stop", "7"], engine(1))).toMatchObject({ stdout: "", exitCode: 2 });
  });

  /** A process reader that is a script printing `line`, in a folder of its own, and the folder. */
  async function readerPrinting(line: string): Promise<{ reader: EngineReader; folder: string }> {
    const folder = await mkdtemp(join(tmpdir(), "rerail-reader-"));
    const entry = join(folder, "reader.mjs");
    await writeFile(entry, `process.stdout.write(${JSON.stringify(`${line}\n`)});`);
    return { reader: { node: process.execPath, entry }, folder };
  }

  it("reads a Windows start from the line its child prints, and reads another way only when the child gives no answer", async () => {
    const { reader, folder } = await readerPrinting("{\"startedAt\":1700000000000}");
    try {
      const fallback = vi.fn((): StartRead => 5);
      expect(processStartedAtInChild(7, reader, fallback, "win32")).toBe(1_700_000_000_000);
      expect(processStartedAtInChild(7, { ...reader, entry: join(folder, "missing.mjs") }, fallback, "win32")).toBe(5);
      expect(fallback).toHaveBeenCalledTimes(1);
      expect(processStartedAtInChild(7, reader, fallback, "darwin")).toBeUndefined();
    } finally {
      await removeFolder(folder);
    }
  });

  it("reads no start, and starts no PowerShell, when its child is still running at the bound", () => {
    vi.mocked(childProcess.spawnSync).mockImplementationOnce((() => ({
      pid: 0, output: [null, "", ""], stdout: "", stderr: "", status: null, signal: "SIGTERM",
      error: Object.assign(new Error("spawnSync node ETIMEDOUT"), { code: "ETIMEDOUT" }),
    })) as unknown as typeof childProcess.spawnSync);
    const fallback = vi.fn((): StartRead => 5);
    expect(processStartedAtInChild(7, { node: process.execPath, entry: "reader.mjs" }, fallback, "win32")).toBe(START_UNANSWERED);
    expect(fallback).not.toHaveBeenCalled();
    expect(vi.mocked(childProcess.spawnSync).mock.lastCall?.[2]).toMatchObject({ timeout: ENGINE_READER_ANSWER_MS });
  });

  it("ends a process through its child, which has the engine's wait on top of its bound, and reads another way without an answer", async () => {
    const { reader, folder } = await readerPrinting("{\"end\":\"ended\"}");
    try {
      const fallback = vi.fn((): ProcessEnd => "running");
      const target = { pid: 7, startedAt: 1_700_000_000_000, waitMs: 5_000 };
      expect(endProcessStartedByInChild(target, reader, fallback)).toBe("ended");
      expect(vi.mocked(childProcess.spawnSync).mock.lastCall?.[2]).toMatchObject({ timeout: ENGINE_READER_ANSWER_MS + 5_000 });
      expect(endProcessStartedByInChild(target, { ...reader, entry: join(folder, "missing.mjs") }, fallback)).toBe("running");
      // A start that names no time a process started ends nothing, and starts no child.
      expect(endProcessStartedByInChild({ ...target, startedAt: Number.POSITIVE_INFINITY }, reader, fallback)).toBe("unread");
      expect(fallback).toHaveBeenCalledTimes(1);
    } finally {
      await removeFolder(folder);
    }
  });
});
