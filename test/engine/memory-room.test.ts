import { PerformanceObserver } from "node:perf_hooks";
import { getHeapStatistics, setFlagsFromString } from "node:v8";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";
import { isFullCollection, memoryRoom, needsRoom, readMemory, type MemoryReading } from "../../src/engine/memory-room.js";

const GB = 1024 ** 3;
/** A daemon of a 16 GB machine with room to spare: a fifth of its heap limit, 300 MB resident, half the machine free. */
const roomy: MemoryReading = { heapUsed: 0.8 * GB, heapLimit: 4 * GB, resident: 0.3 * GB, total: 16 * GB, available: 8 * GB };

describe("whether the daemon needs the room what no session uses takes", () => {
  it("keeps it while the heap, the daemon's own size and the machine all have room", () => {
    expect(needsRoom(roomy)).toBe(false);
  });

  // The heap watch restarts the daemon at 0.8 of its limit: what it keeps warm must go well before that.
  it("needs the room once the heap is past six tenths of its limit", () => {
    expect(needsRoom({ ...roomy, heapUsed: 2.4 * GB })).toBe(false);
    expect(needsRoom({ ...roomy, heapUsed: 2.41 * GB })).toBe(true);
  });

  // A compiled rule's program is native memory the heap does not count: 512 MB for the Node pack's builtins rule.
  it("needs the room once the daemon holds more than a quarter of the machine's memory", () => {
    expect(needsRoom({ ...roomy, resident: 4 * GB })).toBe(false);
    expect(needsRoom({ ...roomy, resident: 4.01 * GB })).toBe(true);
  });

  it("needs the room once the machine has less than a tenth of its memory left for other programs", () => {
    expect(needsRoom({ ...roomy, available: 1.6 * GB })).toBe(false);
    expect(needsRoom({ ...roomy, available: 1.59 * GB })).toBe(true);
  });

  it("judges a machine whose free memory does not count its cache by the daemon's own figures alone", () => {
    expect(needsRoom({ ...roomy, available: undefined })).toBe(false);
    expect(needsRoom({ ...roomy, available: undefined, resident: 4.01 * GB })).toBe(true);
  });

  // macOS reports only its free pages, which an idle Mac keeps low by caching: read there, every unused runtime would go.
  it("reads the machine's free memory everywhere but macOS", () => {
    expect(readMemory("darwin").available).toBeUndefined();
    for (const platform of ["linux", "win32"] as const) {
      const reading = readMemory(platform);
      expect(reading.available).toBeGreaterThan(0);
      expect(reading.available).toBeLessThanOrEqual(reading.total);
      expect(reading.resident).toBeGreaterThan(0);
      expect(reading.heapUsed).toBeLessThan(reading.heapLimit);
    }
  });

  // The heap's own count holds the garbage made since the last collection, most of a busy daemon's heap: read as it is,
  // a burst of work would close every unused runtime though the daemon could free that garbage.
  it("reads the heap as the last full collection left it, not the garbage made since", async () => {
    memoryRoom();
    const seen = Promise.withResolvers<void>();
    const observer = new PerformanceObserver((list) => {
      if (list.getEntries().some(isFullCollection)) seen.resolve();
    });
    observer.observe({ type: "gc" });
    try {
      // A full collection, exposing `gc` for the call, as the daemon makes one once a runtime has closed.
      setFlagsFromString("--expose-gc");
      (runInNewContext("gc") as () => void)();
      await seen.promise;
    } finally {
      observer.disconnect();
    }
    const collected = readMemory("linux").heapUsed;
    const made = Array.from({ length: 50_000 }, (_, index) => `${"x".repeat(64)}${index}`);
    expect(getHeapStatistics().used_heap_size).toBeGreaterThan(collected + 2 * 1024 ** 2);
    expect(readMemory("linux").heapUsed).toBe(collected);
    expect(made).toHaveLength(50_000);
  });
});
