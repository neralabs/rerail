import * as childProcess from "node:child_process";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { processStartedAt, processState, type ProcessSignal } from "../../src/engine/process-liveness.js";

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

  it("reads the creation time PowerShell prints", async () => {
    await standIn(process.execPath, ["-e", "process.stdout.write('1700000000000')"]);
    expect(processStartedAt(process.pid, 30_000, "win32")).toBe(1_700_000_000_000);
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
