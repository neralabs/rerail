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
 * reads the whole corpus in one run, and its cross-file rules are the costly part. The Django pack's first run over
 * a 229-file project took 45 to 50 s on 16 threads, and an edit costs a run of the same size. With half a laptop's
 * cores ({@link defaultThreads}), a project a few times larger takes minutes. A run past 10 minutes is a run no
 * session waits for.
 */
const DEADLINE_MS = 10 * 60_000;

/**
 * Half the cores, at least one. The run shares the machine with the person's editor, build and agent. With half the
 * cores the laptop stays usable and the engine still runs in parallel; on one thread it is impractically slow.
 */
function defaultThreads(): number {
  return Math.max(1, Math.floor(availableParallelism() / 2));
}

/**
 * Half the physical memory, at most 8 GiB. The Django pack's first run over a 229-file project peaked at 5.7 GiB.
 * Over larger projects the engine's chunk budget does not hold the run (one of 1,300 files passed 55 GiB). Without
 * this limit, a run would push the person's own work into swap long before it finished.
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
