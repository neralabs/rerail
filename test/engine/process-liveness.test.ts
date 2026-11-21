import * as childProcess from "node:child_process";
import { readFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import * as native from "../../src/engine/native/native.js";
import { exitedEveryThread, processStartedAt, processStartedAtThroughEngine, processState, type ProcessSignal } from "../../src/engine/process-liveness.js";
import { exitedEveryThread as removalExitedEveryThread } from "../support/platform/remove-folder.js";

// Spies that call through, so a test can stand a program in for Windows PowerShell, and see whether the engine loads.
vi.mock("node:child_process", { spy: true });
vi.mock("../../src/engine/native/native.js", { spy: true });

/** A pid no live process can hold on the platforms Rerail supports. */
const DEAD_PID = 2 ** 22 - 1;

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

  it("ends a PowerShell that does not answer at its bound and reads nothing", async () => {
    const seen: childProcess.SpawnSyncOptions[] = [];
    await standIn(process.execPath, ["-e", "setTimeout(() => {}, 60_000)"], seen);
    const asked = Date.now();
    expect(processStartedAt(process.pid, 500, "win32")).toBeUndefined();
    expect(Date.now() - asked).toBeLessThan(10_000);
    expect(seen.map(options => options.timeout)).toEqual([500]);
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
    expect(processStartedAtThroughEngine(7, 2_000, "win32", engine)).toBe(1_700_000_000_000);
    expect(processStartedAtThroughEngine(8, 2_000, "win32", engine)).toBeUndefined();
    expect(asked).toEqual([7, 8]);
    expect(vi.mocked(childProcess.spawnSync).mock.calls.length).toBe(spawned);
  });

  // As the PowerShell read is: a lookup whose limit an engine silent to the proof used up keeps the record unread.
  it("reads nothing on Windows with no time left, and asks the engine nothing", () => {
    const asked: number[] = [];
    const engine = () => ({
      processStartedAt: (pid: number) => {
        asked.push(pid);
        return 1_700_000_000_000;
      },
    });
    expect(processStartedAtThroughEngine(7, 0, "win32", engine)).toBeUndefined();
    expect(processStartedAtThroughEngine(7, -5, "win32", engine)).toBeUndefined();
    expect(asked).toEqual([]);
  });

  it("reads nothing where the engine does not load or was built before the read", () => {
    expect(processStartedAtThroughEngine(process.pid, 2_000, "win32", () => {
      throw new Error("Unsupported platform or missing native addon");
    })).toBeUndefined();
    expect(processStartedAtThroughEngine(process.pid, 2_000, "win32", () => ({}))).toBeUndefined();
  });

  it("reads no start on a system it does not read, and asks the engine nothing", () => {
    const asked: number[] = [];
    expect(processStartedAtThroughEngine(process.pid, 2_000, "darwin", answeringEngine(asked))).toBeUndefined();
    expect(asked).toEqual([]);
  });

  it.runIf(process.platform === "linux")("reads Linux's start from /proc whatever time is left, and asks the engine nothing", () => {
    const asked: number[] = [];
    // As processStartedAt does, with no time left and with time left, when a lookup would load the engine to ask it.
    // Each read counts the start back from the clock at its own moment, so the reads can differ by the time they took
    // and by /proc's 10 ms resolution, and no more.
    const before = Date.now();
    const withNone = processStartedAtThroughEngine(process.pid, 0, "linux", answeringEngine(asked));
    const withSome = processStartedAtThroughEngine(process.pid, 2_000, "linux", answeringEngine(asked));
    const again = processStartedAt(process.pid, 0, "linux");
    const took = Date.now() - before;
    for (const started of [withNone, withSome]) {
      expect(started).toBeDefined();
      expect(Math.abs(started! - again!)).toBeLessThanOrEqual(took + 20);
    }
    expect(asked).toEqual([]);
  });

  it.runIf(process.platform === "win32")("reads when this process and a later one started through the engine built here", async () => {
    const spawned = vi.mocked(childProcess.spawnSync).mock.calls.length;
    // The order of facts decides, not how long anything took: this process was created before the case ran, and the
    // child after the case asked for it. Windows stamps a creation with a clock that can read a timer tick (15.6 ms by
    // default) apart from Node's, so the child's lower bound allows two ticks.
    const asked = Date.now();
    const started = processStartedAtThroughEngine(process.pid, 2_000);
    expect(started).toBeDefined();
    expect(started!).toBeLessThanOrEqual(asked);
    const spawning = Date.now();
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
    try {
      const childStarted = processStartedAtThroughEngine(child.pid!, 2_000);
      expect(childStarted).toBeGreaterThan(started!);
      expect(childStarted!).toBeGreaterThanOrEqual(spawning - 32);
    } finally {
      const closed = once(child, "close");
      child.kill();
      await closed;
    }
    expect(processStartedAtThroughEngine(DEAD_PID, 2_000)).toBeUndefined();
    expect(vi.mocked(childProcess.spawnSync).mock.calls.length).toBe(spawned);
  });
});
