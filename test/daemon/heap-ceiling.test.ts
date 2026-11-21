import { afterEach, describe, expect, it, vi } from "vitest";
import { watchHeap, type HeapUse } from "../../src/daemon/heap-ceiling.js";

afterEach(() => {
  vi.useRealTimers();
});

/** A heap that reads `readings` in turn, then `rest`. */
function heap(readings: HeapUse[], rest: HeapUse) {
  return vi.fn(() => readings.shift() ?? rest);
}

describe("the daemon's heap ceiling", () => {
  it("calls for a restart once, the first time the heap is still past the ceiling after a full collection, and never below it", () => {
    vi.useFakeTimers();
    const read = heap([
      { used: 700, limit: 1_000 },
      { used: 799, limit: 1_000 },
      { used: 801, limit: 1_000 },
      { used: 850, limit: 1_000 },
    ], { used: 990, limit: 1_000 });
    const collect = vi.fn();
    const onCeiling = vi.fn();
    const stop = watchHeap({ read, collect, ratio: 0.8, intervalMs: 1_000, graceMs: 0, onCeiling });
    vi.advanceTimersByTime(2_000);
    expect(collect).not.toHaveBeenCalled();
    expect(onCeiling).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1_000);
    expect(collect).toHaveBeenCalledTimes(1);
    expect(onCeiling).toHaveBeenCalledExactlyOnceWith({ used: 850, limit: 1_000 });
    vi.advanceTimersByTime(5_000);
    expect(onCeiling).toHaveBeenCalledTimes(1);
    expect(read).toHaveBeenCalledTimes(4);
    stop();
  });

  // V8 collects a large heap only once it grew to a few times what was live, so the used heap passes the ceiling on
  // garbage alone; the daemon closed every session for it.
  it("restarts for no heap a full collection brings back under the ceiling", () => {
    vi.useFakeTimers();
    const read = heap([
      { used: 950, limit: 1_000 },
      { used: 300, limit: 1_000 },
      { used: 900, limit: 1_000 },
      { used: 400, limit: 1_000 },
    ], { used: 500, limit: 1_000 });
    const collect = vi.fn();
    const onCeiling = vi.fn();
    const stop = watchHeap({ read, collect, ratio: 0.8, intervalMs: 1_000, graceMs: 0, onCeiling });
    vi.advanceTimersByTime(5_000);
    expect(collect).toHaveBeenCalledTimes(2);
    expect(onCeiling).not.toHaveBeenCalled();
    stop();
  });

  // A daemon past the ceiling in its first minutes restarted once those minutes were over, and its successor again,
  // every few minutes for as long as its live heap stayed high.
  it("never restarts a daemon past the ceiling in its first minutes, so a heap limit set too low cannot make it restart again and again", () => {
    vi.useFakeTimers();
    const read = vi.fn(() => ({ used: 999, limit: 1_000 }));
    const onCeiling = vi.fn();
    const stop = watchHeap({ read, collect: () => {}, intervalMs: 1_000, graceMs: 10_000, onCeiling });
    vi.advanceTimersByTime(60_000);
    expect(onCeiling).not.toHaveBeenCalled();
    expect(read).toHaveBeenCalledTimes(2);
    stop();
  });

  it("restarts a daemon that passes the ceiling once its first minutes are over", () => {
    vi.useFakeTimers();
    const onCeiling = vi.fn();
    const read = vi.fn(() => (Date.now() < 10_000 ? { used: 100, limit: 1_000 } : { used: 999, limit: 1_000 }));
    vi.setSystemTime(0);
    const stop = watchHeap({ read, collect: () => {}, intervalMs: 1_000, graceMs: 10_000, onCeiling });
    vi.advanceTimersByTime(9_000);
    expect(onCeiling).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1_000);
    expect(onCeiling).toHaveBeenCalledExactlyOnceWith({ used: 999, limit: 1_000 });
    stop();
  });

  it("stops reading once stopped", () => {
    vi.useFakeTimers();
    const read = vi.fn(() => ({ used: 0, limit: 1_000 }));
    const stop = watchHeap({ read, collect: () => {}, intervalMs: 1_000, graceMs: 0, onCeiling: () => {} });
    vi.advanceTimersByTime(1_000);
    stop();
    vi.advanceTimersByTime(5_000);
    expect(read).toHaveBeenCalledTimes(1);
  });
});
