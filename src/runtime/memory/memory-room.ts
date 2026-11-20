import { totalmem } from "node:os";
import { constants, PerformanceObserver, type PerformanceEntry, type PerformanceNodeEntry } from "node:perf_hooks";
import { getHeapStatistics } from "node:v8";

/**
 * What the daemon's memory reads as at one moment, in bytes. `heapUsed` is the heap in use just after the last full
 * collection, read as its entry is delivered, once `memoryRoom` watches full collections, else the heap's count now.
 * `available` is what the system says other programs can still have; undefined where its figure does not count what the
 * system would free: on macOS libuv reads only the free pages, and the system keeps most of its memory as cache it gives
 * back on demand, so the figure is low on an idle Mac.
 */
export interface MemoryReading {
  readonly heapUsed: number;
  readonly heapLimit: number;
  readonly resident: number;
  readonly total: number;
  readonly available: number | undefined;
}

/**
 * The share of its heap limit past which a daemon restarts itself rather than wait to run out (`watchHeap` in
 * daemon/heap-ceiling.ts), once it has closed what no session uses and its heap is still past it after a full collection.
 */
export const HEAP_CEILING_RATIO = 0.8;
/**
 * Past this share of its heap limit after a full collection, the daemon lets go of what no session uses: three quarters
 * of the ceiling, so what it keeps for its checkouts' next sessions is closed well before that restart comes near.
 */
const HEAP_ROOM_RATIO = HEAP_CEILING_RATIO * 0.75;
/**
 * Past this share of the machine's memory held by the daemon itself, the daemon lets go of what no session uses. The
 * resident size counts what the heap does not: a compiled rule's program is native memory, 512 MB for the Node pack's
 * builtins rule.
 */
const RESIDENT_SHARE = 0.25;
/** Below this share of the machine's memory left to other programs, the daemon lets go of what no session uses. */
const AVAILABLE_FLOOR = 0.1;

/** The heap in use as the last full collection ended, once `watchFullCollections` has seen one. */
let collected: number | undefined;
let watching = false;

/**
 * Notes the heap in use as each full collection ends, from now on (Node's `gc` performance entries), forcing none: what
 * the daemon holds and cannot free. The heap's own count also holds the garbage made since, which on a busy daemon is
 * most of it, so a reading past a mark could be garbage alone. Once per process.
 */
function watchFullCollections(): void {
  if (watching) return;
  watching = true;
  new PerformanceObserver((list) => {
    if (list.getEntries().some(isFullCollection)) collected = getHeapStatistics().used_heap_size;
  }).observe({ type: "gc" });
}

/** Whether the gc performance entry `entry` is a full collection's: its detail names the collection's kind. */
export function isFullCollection(entry: PerformanceEntry): boolean {
  // A gc entry is a node entry, whose detail the types leave as any.
  const detail: unknown = (entry as PerformanceNodeEntry).detail;
  return typeof detail === "object" && detail !== null && "kind" in detail && detail.kind === constants.NODE_PERFORMANCE_GC_MAJOR;
}

/** The daemon's memory now: its heap, its resident size, and the machine's memory (a container's limit where it has one). */
export function readMemory(platform: NodeJS.Platform = process.platform): MemoryReading {
  const constrained = process.constrainedMemory();
  const total = constrained > 0 && constrained < totalmem() ? constrained : totalmem();
  const available = platform === "darwin" ? undefined : process.availableMemory();
  const { used_heap_size: used, heap_size_limit: heapLimit } = getHeapStatistics();
  return { heapUsed: collected ?? used, heapLimit, resident: process.memoryUsage.rss(), total, available };
}

/**
 * Whether the daemon needs the room that what no session uses takes (a checkout's runtime with its compiled rules and
 * source walk): its heap is past `HEAP_ROOM_RATIO` of its limit, it holds more than `RESIDENT_SHARE` of the machine's
 * memory, or the machine has less than `AVAILABLE_FLOOR` of it left. Otherwise keeping it costs nobody memory they need,
 * and a session that comes back to its checkout finds its rules compiled.
 */
export function needsRoom(reading: MemoryReading): boolean {
  if (reading.heapUsed > reading.heapLimit * HEAP_ROOM_RATIO) return true;
  if (reading.resident > reading.total * RESIDENT_SHARE) return true;
  return reading.available !== undefined && reading.available < reading.total * AVAILABLE_FLOOR;
}

/**
 * Whether the daemon needs the room now (`needsRoom`), read from its memory each time it is asked (`readMemory`), with
 * the heap as the last full collection left it from the call on (`watchFullCollections`): the daemon's engine service
 * makes one as it starts.
 */
export function memoryRoom(platform: NodeJS.Platform = process.platform): () => boolean {
  watchFullCollections();
  return () => needsRoom(readMemory(platform));
}
