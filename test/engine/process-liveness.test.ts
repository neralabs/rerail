import { spawn } from "node:child_process";
import { once } from "node:events";
import { describe, expect, it } from "vitest";
import { processStartedAt, processState, type ProcessSignal } from "../../src/engine/process-liveness.js";

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
    const started = processStartedAt(process.pid);
    expect(started).toBeDefined();
    expect(Math.abs(started! - (Date.now() - process.uptime() * 1_000))).toBeLessThan(1_000);
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
    try {
      expect(processStartedAt(child.pid!)).toBeGreaterThan(started!);
    } finally {
      const closed = once(child, "close");
      child.kill();
      await closed;
    }
  });

  it("reads no start for a pid no process holds, nor on a system it does not read", () => {
    expect(processStartedAt(DEAD_PID)).toBeUndefined();
    expect(processStartedAt(process.pid, "darwin")).toBeUndefined();
  });
});
