import { availableParallelism, totalmem } from "node:os";

/**
 * What a `.rerail` run's child process may take before the runner kills it, and whether it keeps a store. Each
 * default has its reason beside it and a `RERAIL_RR_*` override, read at each run.
 *
 * The figures below are the Django pack's (27 rules) with no store, on 16 threads, a full run each time: 15 s and
 * 0.8 GB on a 229-file project, 123 s and 4.7 GB on a 1,300-file one, 251 s and 7.7 GB on a 4,500-file one.
 *
 * The budgets are one child's, and one child runs at a time in a process (`rr-run-slots.ts`): every checkout's runs
 * after edits and every audit's run take turns. So a process's `.rerail` runs take at most half the memory and half
 * the cores together. A headless `rerail scan` is a process of its own, beside the daemon's.
 */
export interface RrBudgets {
  /** Wall time, from the child's start. */
  deadlineMs: number;
  /** The engine's worker threads in the child (`RERAIL_THREADS`). */
  threads: number;
  /** The child's resident memory. */
  memoryBytes: number;
  /** Whether the child keeps the engine's store of the checkout it runs in (`RR_STORE`). */
  store: boolean;
}

/**
 * 10 minutes. The JSON path stops a repository audit at 5 minutes (`AUDIT_MAX_DURATION_MS`), but a `.rerail` pack
 * reads the whole corpus in one run, and its cross-file rules are the costly part: the 4,500-file project's run took
 * 251 s on 16 threads and 274 s on 4. Ten minutes leaves room for that size on half a laptop's cores; a run past it
 * is a run no session waits for.
 */
const DEADLINE_MS = 10 * 60_000;

/**
 * Half the cores, at least one. The run shares the machine with the person's editor, build and agent. With half the
 * cores the laptop stays usable and the engine still runs in parallel: the 4,500-file run took 274 s on 4 threads
 * against 251 s on 16.
 */
function defaultThreads(): number {
  return Math.max(1, Math.floor(availableParallelism() / 2));
}

/**
 * Half the physical memory. The engine's own chunk budget does not bound a run, and a run larger than half the machine
 * would push the person's own work into swap long before it finished, so it is stopped there. On a 16 GB laptop the
 * limit is 8 GB: the 1,300-file project's run (4.7 GB) fits, and the 4,500-file one (7.2 GB on 4 threads) just fits.
 */
function defaultMemoryBytes(): number {
  return Math.floor(totalmem() / 2);
}

/**
 * No store, by default. With the engine's store, a repeat run on an unchanged checkout takes under half a second, and
 * a run after an edit of a 229-file project 3.4 s against 13 s. But on larger projects the store's run after an edit
 * costs more than a full run without it (184 s and 6.0 GB against 123 s and 4.7 GB on the 1,300-file project, 493 s
 * and 8.5 GB against 251 s and 7.7 GB on the 4,500-file one), and its first run peaks about 1.7 times higher. Memory
 * is the limit on a laptop, and sessions run after edits, so the child runs with no store. To revisit when the store's
 * cost after an edit is fixed. `RERAIL_RR_STORE=checkout` keeps the engine's store of the checkout the child runs in.
 */
const RR_STORE = "RERAIL_RR_STORE";

/**
 * The budgets of a run: each `RERAIL_RR_*` variable that is set, else the caller's deadline (`deadlineMs`), else the
 * default. Throws for a variable that holds no value it takes.
 */
export function rrBudgets(deadlineMs?: number, environment: NodeJS.ProcessEnv = process.env): RrBudgets {
  const memoryMb = positive(environment, "RERAIL_RR_MEMORY_MB");
  return {
    deadlineMs: positive(environment, "RERAIL_RR_DEADLINE_MS") ?? deadlineMs ?? DEADLINE_MS,
    threads: positive(environment, "RERAIL_RR_THREADS") ?? defaultThreads(),
    memoryBytes: memoryMb === undefined ? defaultMemoryBytes() : memoryMb * 1024 ** 2,
    store: keepsStore(environment),
  };
}

/** Whether `RR_STORE` asks for the checkout's store: `checkout`; `off` or unset, no store. */
function keepsStore(environment: NodeJS.ProcessEnv): boolean {
  const text = environment[RR_STORE];
  if (text === undefined || text === "" || text === "off") return false;
  if (text === "checkout") return true;
  throw new Error(`${RR_STORE} is ${JSON.stringify(text)}: it takes "off" or "checkout"`);
}

/** The positive integer `name` holds, or undefined when it is unset. */
function positive(environment: NodeJS.ProcessEnv, name: string): number | undefined {
  const text = environment[name];
  if (text === undefined || text === "") return undefined;
  const value = Number(text);
  if (!Number.isSafeInteger(value) || value <= 0) throw new Error(`${name} is ${JSON.stringify(text)}: it takes a positive whole number`);
  return value;
}
