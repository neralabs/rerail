import { getHeapStatistics } from "node:v8";
import { collectGarbage } from "../runtime/engine-service.js";
// The ceiling lives beside the mark at which the engine service lets go of what no session uses, which it derives.
import { HEAP_CEILING_RATIO } from "../runtime/memory/memory-room.js";

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
  /**
   * Frees what the daemon keeps but no session uses (`EngineService.closeUnusedRuntimes`), settling once it is freed:
   * asked before a restart, since what it frees may bring the heap under the ceiling.
   */
  relieve?: () => Promise<void>;
  ratio?: number;
  intervalMs?: number;
  /**
   * How soon after the start a heap past the ceiling means the limit is too low for the daemon's work rather than that
   * it is running out: a restart would only pass the ceiling again, so the watch ends and the daemon keeps running.
   */
  graceMs?: number;
  /** Called once, the first time the heap is still past `ratio` of the limit after a full collection and the relief. */
  onCeiling: (use: HeapUse) => void;
}

function v8Heap(): HeapUse {
  const { used_heap_size: used, heap_size_limit: limit } = getHeapStatistics();
  return { used, limit };
}

/** The heap in use after a full collection, or undefined while it is under `ratio` of its limit. */
function pastCeiling(read: () => HeapUse, collect: () => void, ratio: number): HeapUse | undefined {
  const first = read();
  if (first.used < first.limit * ratio) return undefined;
  collect();
  const use = read();
  return use.used < use.limit * ratio ? undefined : use;
}

/**
 * Watches the daemon's heap. Near its limit, V8 spends most of its time in mark-compact pauses of a second or two, in
 * which the daemon answers no health probe and records no event, and then the process dies without cleaning up; a
 * daemon that restarts at `ratio` loses a few seconds instead. The used heap counts garbage V8 has not collected yet,
 * which can pass the ceiling on a live heap far below it, so a reading past it is taken again after a full collection;
 * then what the daemon keeps but no session uses is freed (`relieve`) and the heap read again after another collection,
 * and only that reading decides. Past the ceiling within `graceMs` of the start, the watch ends without a restart: the
 * daemon used to restart anyway once the grace was over, and so again every few minutes for as long as its live heap
 * stayed high. While a relief runs, the next looks skip; with no `relieve` the first reading after a collection decides.
 * Returns the function that stops the watch. The timer keeps no process alive.
 */
export function watchHeap({ read = v8Heap, collect = collectGarbage, relieve, ratio = HEAP_CEILING_RATIO, intervalMs = HEAP_CHECK_INTERVAL_MS, graceMs = HEAP_CEILING_GRACE_MS, onCeiling }: HeapWatch): () => void {
  const started = Date.now();
  let relieving = false;
  let stopped = false;
  const stop = (): void => {
    stopped = true;
    clearInterval(timer);
  };
  const atCeiling = (use: HeapUse | undefined): void => {
    if (use === undefined || stopped) return;
    stop();
    if (Date.now() - started >= graceMs) onCeiling(use);
  };
  const look = (): void => {
    const use = relieving ? undefined : pastCeiling(read, collect, ratio);
    if (use === undefined || relieve === undefined) return atCeiling(use);
    relieving = true;
    void relieve().catch(() => undefined).then(() => {
      relieving = false;
      atCeiling(pastCeiling(read, collect, ratio));
    });
  };
  const timer = setInterval(look, intervalMs);
  timer.unref();
  return stop;
}
