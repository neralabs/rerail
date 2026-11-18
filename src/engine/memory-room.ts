import { totalmem } from "node:os";
import { getHeapStatistics } from "node:v8";

/**
 * What the daemon's memory reads as at one moment, in bytes. `available` is what the system says other programs can
 * still have; undefined where its figure does not count what the system would free: on macOS libuv reads only the free
 * pages, and the system keeps most of its memory as cache it gives back on demand, so the figure is low on an idle Mac.
 */
export interface MemoryReading {
  readonly heapUsed: number;
  readonly heapLimit: number;
  readonly resident: number;
  readonly total: number;
  readonly available: number | undefined;
}

/**
 * Past this share of its heap limit, garbage counted, the daemon lets go of what no session uses: under the 0.8 at which
 * the heap watch restarts the daemon (`HEAP_CEILING_RATIO`), so what it keeps warm never brings that restart about.
 */
const HEAP_ROOM_RATIO = 0.6;
/**
 * Past this share of the machine's memory held by the daemon itself, the daemon lets go of what no session uses. The
 * resident size counts what the heap does not: a compiled rule's program is native memory, 512 MB for the Node pack's
 * builtins rule.
 */
const RESIDENT_SHARE = 0.25;
/** Below this share of the machine's memory left to other programs, the daemon lets go of what no session uses. */
const AVAILABLE_FLOOR = 0.1;

/** The daemon's memory now: its heap, its resident size, and the machine's memory (a container's limit where it has one). */
export function readMemory(platform: NodeJS.Platform = process.platform): MemoryReading {
  const { used_heap_size: heapUsed, heap_size_limit: heapLimit } = getHeapStatistics();
  const constrained = process.constrainedMemory();
  const total = constrained > 0 && constrained < totalmem() ? constrained : totalmem();
  const available = platform === "darwin" ? undefined : process.availableMemory();
  return { heapUsed, heapLimit, resident: process.memoryUsage.rss(), total, available };
}

/**
 * Whether the daemon needs the room that what no session uses takes (a checkout's runtime with its compiled rules and
 * source walk): its heap is past `HEAP_ROOM_RATIO` of its limit, it holds more than `RESIDENT_SHARE` of the machine's
 * memory, or the machine has less than `AVAILABLE_FLOOR` of it left. Otherwise keeping it costs nobody memory they need,
 * and a session that comes back to its checkout finds its rules compiled.
 */
export function needsRoom(reading: MemoryReading = readMemory()): boolean {
  if (reading.heapUsed > reading.heapLimit * HEAP_ROOM_RATIO) return true;
  if (reading.resident > reading.total * RESIDENT_SHARE) return true;
  return reading.available !== undefined && reading.available < reading.total * AVAILABLE_FLOOR;
}
