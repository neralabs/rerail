/**
 * The daemon's upkeep of the recorded checkouts: when it starts and every hour after, it forgets the checkouts whose
 * folder is gone or that Git no longer lists ({@link pruneCheckouts}), so the reads that go over a repository's
 * checkouts on every request, and its checkout count, follow the worktrees and clones on disk.
 */
import { existsSync } from "node:fs";
import { pruneCheckouts } from "../engine/catalog/repository-registry.js";
import { openRerailDatabase } from "../engine/storage/database.js";

/** How often a running daemon forgets gone checkouts, so a removed worktree stays recorded at most this long. */
export const CHECKOUT_PRUNING_INTERVAL_MS = 60 * 60 * 1000;

export interface CheckoutPruning {
  /**
   * Stops the passes. A pass under way forgets nothing and ends after the folder it is looking at; its connection is
   * closed once this settles.
   */
  stop(): Promise<void>;
}

export interface CheckoutPruningOptions {
  readonly intervalMs?: number;
  readonly log?: (line: string) => void;
}

/**
 * Forget gone checkouts now and every `intervalMs`, one pass at a time, each over its own connection so the API's
 * reads see the change. A daemon with no database yet does not create one. It never throws and never holds the
 * daemon open; a failed pass is logged and the next one tries again.
 */
export function startCheckoutPruning(databasePath: string, options: CheckoutPruningOptions = {}): CheckoutPruning {
  const log = options.log ?? (line => process.stderr.write(line));
  let running: Promise<void> | undefined;
  const abort = new AbortController();
  const pass = (): void => {
    if (abort.signal.aborted || running !== undefined || !existsSync(databasePath)) return;
    running = prune(databasePath, abort.signal)
      .catch((error: unknown) => log(`rerail daemon: forgetting removed checkouts: ${error instanceof Error ? error.message : String(error)}\n`))
      .finally(() => { running = undefined; });
  };
  const timer = setInterval(pass, options.intervalMs ?? CHECKOUT_PRUNING_INTERVAL_MS);
  timer.unref();
  pass();
  return {
    async stop() {
      clearInterval(timer);
      abort.abort();
      await running;
    },
  };
}

async function prune(databasePath: string, signal: AbortSignal): Promise<void> {
  const database = openRerailDatabase(databasePath);
  try {
    await pruneCheckouts(database, { signal });
  } finally {
    database.$client.close();
  }
}
