import { availableParallelism, totalmem } from "node:os";

/**
 * What a `.rerail` run's child process may take before the runner kills it. Each default has its reason beside it and
 * a `RERAIL_RR_*` override, read at each run.
 */
export interface RrBudgets {
  /** Wall time, from the child's start. */
  deadlineMs: number;
  /** The engine's worker threads in the child (`RERAIL_THREADS`). */
  threads: number;
  /** The child's resident memory. */
  memoryBytes: number;
}

/**
 * 10 minutes. The JSON path stops a repository audit at 5 minutes (`AUDIT_MAX_DURATION_MS`), but a `.rerail` pack
 * reads the whole corpus in one run and its cross-file rules are the costly part: the Django pack over netbox took
 * 164 s with every core of a 32-core machine, and runs here with half the cores ({@link defaultThreads}), so a
 * repository of that size needs about twice that. A run past 10 minutes is a run the session would not wait for.
 */
const DEADLINE_MS = 10 * 60_000;

/**
 * Half the cores, at least one. The run shares the machine with the person's editor, build and agent; with half the
 * cores the laptop stays usable, and the engine still parallelises (one thread is impractically slow: netbox's pack
 * takes over 10 minutes on one thread against under 3 on four).
 */
function defaultThreads(): number {
  return Math.max(1, Math.floor(availableParallelism() / 2));
}

/**
 * Half the physical memory, at most 8 GiB. The Django pack peaked at 4.5 GiB over netbox; a run larger than half the
 * machine would push the person's own work into swap, and past 8 GiB a run is one no session waits for.
 */
function defaultMemoryBytes(): number {
  return Math.min(8 * 1024 ** 3, Math.floor(totalmem() / 2));
}

/**
 * The budgets of a run: each `RERAIL_RR_*` variable that is set, else the caller's deadline (`deadlineMs`), else the
 * default. Throws for a variable that holds no positive integer.
 */
export function rrBudgets(deadlineMs?: number, environment: NodeJS.ProcessEnv = process.env): RrBudgets {
  const memoryMb = positive(environment, "RERAIL_RR_MEMORY_MB");
  return {
    deadlineMs: positive(environment, "RERAIL_RR_DEADLINE_MS") ?? deadlineMs ?? DEADLINE_MS,
    threads: positive(environment, "RERAIL_RR_THREADS") ?? defaultThreads(),
    memoryBytes: memoryMb === undefined ? defaultMemoryBytes() : memoryMb * 1024 ** 2,
  };
}

/** The positive integer `name` holds, or undefined when it is unset. */
function positive(environment: NodeJS.ProcessEnv, name: string): number | undefined {
  const text = environment[name];
  if (text === undefined || text === "") return undefined;
  const value = Number(text);
  if (!Number.isSafeInteger(value) || value <= 0) throw new Error(`${name} is ${JSON.stringify(text)}: it takes a positive whole number`);
  return value;
}
