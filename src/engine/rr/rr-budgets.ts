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
 * reads the whole corpus in one run, and its cross-file rules are the costly part. Measured with the Django pack's
 * 27 rules on 4 threads, on the checkout's store (`rr-child-process.ts`): a first run took 13 s on a 229-file project,
 * 151 s on a 1,300-file one and 283 s on a 4,500-file one, and a run after an edit 4 s, 183 s and 514 s. A run past
 * 10 minutes is a run no session waits for.
 */
const DEADLINE_MS = 10 * 60_000;

/**
 * Half the cores, at least one. The run shares the machine with the person's editor, build and agent. With half the
 * cores the laptop stays usable and the engine still runs in parallel: the runs above took no more than 1.5 times
 * as long on 4 threads as on 16.
 */
function defaultThreads(): number {
  return Math.max(1, Math.floor(availableParallelism() / 2));
}

/**
 * Half the physical memory. The engine's own chunk budget does not bound a run: the Django pack's runs above
 * peaked at 1.2 GB, 8.0 GB and 12.6 GB. A run larger than half the machine would push the person's own work into
 * swap long before it finished, so it is stopped there; a machine with less memory runs only smaller projects.
 */
function defaultMemoryBytes(): number {
  return Math.floor(totalmem() / 2);
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
