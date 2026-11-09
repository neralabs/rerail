import { getHeapStatistics } from "node:v8";

/** The share of its heap limit past which a daemon restarts itself rather than wait to run out. */
export const HEAP_CEILING_RATIO = 0.8;
const HEAP_CHECK_INTERVAL_MS = 30_000;
const HEAP_CEILING_GRACE_MS = 10 * 60_000;

export interface HeapUse {
  used: number;
  limit: number;
}

export interface HeapWatch {
  /** Reads the heap; `v8.getHeapStatistics` by default. */
  read?: () => HeapUse;
  ratio?: number;
  intervalMs?: number;
  /**
   * How long after the start no reading counts. A daemon whose heap passes the ceiling that soon would only pass it again
   * after each restart (a limit set too low), so it keeps running instead.
   */
  graceMs?: number;
  /** Called once, the first time the used heap passes `ratio` of the limit. */
  onCeiling: (use: HeapUse) => void;
}

function v8Heap(): HeapUse {
  const { used_heap_size: used, heap_size_limit: limit } = getHeapStatistics();
  return { used, limit };
}

/**
 * Watches the daemon's heap. Near its limit, V8 spends most of its time in mark-compact pauses of a second or two, in
 * which the daemon answers no health probe and records no event, and then the process dies without cleaning up; a
 * daemon that restarts at `ratio` loses a few seconds instead. Returns the function that stops the watch. The timer
 * keeps no process alive.
 */
export function watchHeap({ read = v8Heap, ratio = HEAP_CEILING_RATIO, intervalMs = HEAP_CHECK_INTERVAL_MS, graceMs = HEAP_CEILING_GRACE_MS, onCeiling }: HeapWatch): () => void {
  let reached = false;
  const counts = Date.now() + graceMs;
  const timer = setInterval(() => {
    if (reached || Date.now() < counts) return;
    const use = read();
    if (use.used < use.limit * ratio) return;
    reached = true;
    clearInterval(timer);
    onCeiling(use);
  }, intervalMs);
  timer.unref();
  return () => clearInterval(timer);
}
