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
});
