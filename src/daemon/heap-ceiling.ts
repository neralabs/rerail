import { getHeapStatistics } from "node:v8";
import { collectGarbage } from "../runtime/engine-service.js";

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
  /** Runs a full collection; `gc()`, exposed for the call, by default. */
  collect?: () => void;
  ratio?: number;
  intervalMs?: number;
  /**
   * How soon after the start a heap past the ceiling means the limit is too low for the daemon's work rather than that
   * it is running out: a restart would only pass the ceiling again, so the watch ends and the daemon keeps running.
   */
  graceMs?: number;
  /** Called once, the first time the heap is still past `ratio` of the limit after a full collection. */
  onCeiling: (use: HeapUse) => void;
}

function v8Heap(): HeapUse {
  const { used_heap_size: used, heap_size_limit: limit } = getHeapStatistics();
  return { used, limit };
}

/**
 * Watches the daemon's heap. Near its limit, V8 spends most of its time in mark-compact pauses of a second or two, in
 * which the daemon answers no health probe and records no event, and then the process dies without cleaning up; a
 * daemon that restarts at `ratio` loses a few seconds instead. The used heap counts garbage V8 has not collected yet,
 * which can pass the ceiling on a live heap far below it, so a reading past it is taken again after a full collection
 * and only that one decides. Past the ceiling within `graceMs` of the start, the watch ends without a restart: the
 * daemon used to restart anyway once the grace was over, and so again every few minutes for as long as its live heap
 * stayed high. Returns the function that stops the watch. The timer keeps no process alive.
 */
export function watchHeap({ read = v8Heap, collect = collectGarbage, ratio = HEAP_CEILING_RATIO, intervalMs = HEAP_CHECK_INTERVAL_MS, graceMs = HEAP_CEILING_GRACE_MS, onCeiling }: HeapWatch): () => void {
  const started = Date.now();
  const timer = setInterval(() => {
    const first = read();
    if (first.used < first.limit * ratio) return;
    collect();
    const use = read();
    if (use.used < use.limit * ratio) return;
    clearInterval(timer);
    if (Date.now() - started >= graceMs) onCeiling(use);
  }, intervalMs);
  timer.unref();
  return () => clearInterval(timer);
}
