import { afterEach, describe, expect, it, vi } from "vitest";
import { watchHeap, type HeapUse } from "../../src/daemon/heap-ceiling.js";

afterEach(() => {
  vi.useRealTimers();
});

describe("the daemon's heap ceiling", () => {
  it("calls for a restart once, the first time the heap passes the ceiling, and never below it", () => {
    vi.useFakeTimers();
    const readings: HeapUse[] = [
      { used: 700, limit: 1_000 },
      { used: 799, limit: 1_000 },
      { used: 801, limit: 1_000 },
      { used: 950, limit: 1_000 },
    ];
    const read = vi.fn(() => readings.shift() ?? { used: 990, limit: 1_000 });
    const onCeiling = vi.fn();
    const stop = watchHeap({ read, ratio: 0.8, intervalMs: 1_000, graceMs: 0, onCeiling });
    vi.advanceTimersByTime(2_000);
    expect(onCeiling).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1_000);
    expect(onCeiling).toHaveBeenCalledExactlyOnceWith({ used: 801, limit: 1_000 });
    vi.advanceTimersByTime(5_000);
    expect(onCeiling).toHaveBeenCalledTimes(1);
    expect(read).toHaveBeenCalledTimes(3);
    stop();
  });

  it("counts no reading in its first minutes, so a heap limit set too low cannot make it restart again and again", () => {
    vi.useFakeTimers();
    const onCeiling = vi.fn();
    const stop = watchHeap({ read: () => ({ used: 999, limit: 1_000 }), intervalMs: 1_000, graceMs: 10_000, onCeiling });
    vi.advanceTimersByTime(9_000);
    expect(onCeiling).not.toHaveBeenCalled();
    vi.advanceTimersByTime(2_000);
    expect(onCeiling).toHaveBeenCalledTimes(1);
    stop();
  });

  it("stops reading once stopped", () => {
    vi.useFakeTimers();
    const read = vi.fn(() => ({ used: 0, limit: 1_000 }));
    const stop = watchHeap({ read, intervalMs: 1_000, graceMs: 0, onCeiling: () => {} });
    vi.advanceTimersByTime(1_000);
    stop();
    vi.advanceTimersByTime(5_000);
    expect(read).toHaveBeenCalledTimes(1);
  });
});
