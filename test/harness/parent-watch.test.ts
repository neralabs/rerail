import { afterEach, describe, expect, it, vi } from "vitest";
import { exitWhenParentGone } from "../../src/harness/parent-watch.js";

describe("exitWhenParentGone", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("exits a Windows hook once the cmd.exe or PowerShell that started it is gone", () => {
    vi.useFakeTimers();
    let parentRuns = true;
    const exit = vi.fn();
    const alive = vi.fn(() => parentRuns);
    exitWhenParentGone({ platform: "win32", parentPid: 4242, alive, exit, intervalMs: 1_000 });

    vi.advanceTimersByTime(3_000);
    expect(alive).toHaveBeenCalledWith(4242);
    expect(exit).not.toHaveBeenCalled();

    // The host killed the shell on its hook timeout; killing a parent on Windows leaves the child running.
    parentRuns = false;
    vi.advanceTimersByTime(1_000);
    expect(exit).toHaveBeenCalledExactlyOnceWith(0);
    vi.advanceTimersByTime(5_000);
    expect(exit).toHaveBeenCalledTimes(1);
  });

  it("does nothing on macOS and Linux, where the launcher execs node in its own place", () => {
    vi.useFakeTimers();
    const exit = vi.fn();
    const alive = vi.fn(() => false);
    exitWhenParentGone({ platform: "linux", parentPid: 4242, alive, exit, intervalMs: 1_000 });
    vi.advanceTimersByTime(5_000);
    expect(alive).not.toHaveBeenCalled();
    expect(exit).not.toHaveBeenCalled();
  });

  it("stops watching when asked", () => {
    vi.useFakeTimers();
    const exit = vi.fn();
    const stop = exitWhenParentGone({ platform: "win32", parentPid: 4242, alive: () => false, exit, intervalMs: 1_000 });
    stop();
    vi.advanceTimersByTime(5_000);
    expect(exit).not.toHaveBeenCalled();
  });
});
