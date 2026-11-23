import * as childProcess from "node:child_process";
import { readFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { exitedEveryThread, processStartedAt, processState, type ProcessSignal } from "../../src/engine/process-liveness.js";
import { cmdletsWithoutModule, commandScript, MODULE_QUALIFIED_ONLY, withCommandPrefix } from "../support/platform/powershell-commands.js";
import { exitedEveryThread as removalExitedEveryThread } from "../support/platform/remove-folder.js";

// Spies that call through, so a test can stand a program in for Windows PowerShell.
vi.mock("node:child_process", { spy: true });

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
  it("reads the creation time PowerShell prints as a FILETIME, in epoch milliseconds", async () => {
    await standIn(process.execPath, ["-e", "process.stdout.write('133444736000001234\\r\\n')"]);
    expect(processStartedAt(process.pid, 30_000, "win32")).toBe(1_700_000_000_000);
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
