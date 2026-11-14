import { describe, expect, it } from "vitest";
import { processState, type ProcessSignal } from "../../src/engine/process-liveness.js";

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

  // Init reaps an exited daemon whenever it runs: signal 0 can still find the zombie, and by the read of its
  // /proc/<pid>/stat the entry is gone with it.
  it.runIf(process.platform === "linux")("reads a pid reaped between its signal and its /proc read as gone", () => {
    let asked = 0;
    const reapedAfterTheFirstSignal: ProcessSignal = () => {
      asked += 1;
      if (asked > 1) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
      return true;
    };
    expect(processState(DEAD_PID, reapedAfterTheFirstSignal)).toBe("gone");
  });

  it("finds this process running and a pid no process holds gone", () => {
    expect(processState(process.pid)).toBe("running");
    expect(processState(DEAD_PID)).toBe("gone");
  });
});
