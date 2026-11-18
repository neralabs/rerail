/**
 * The daemon's upkeep of the recorded checkouts: when it starts and every hour after, it forgets the checkouts Git no
 * longer lists as checkouts of their clone ({@link pruneCheckouts}), so the reads that go over a repository's
 * checkouts on every request, and its checkout count, follow the worktrees Git knows. Where a checkout may be off
 * disk, every checkout of its clone stays: its `.git` can't be read, or its folder is gone and no folder above it is
 * there, or the nearest one still there is one drives mount in, or is empty and inside no checkout, as an unplugged
 * drive leaves its mount point. Such a checkout may be a copy of the clone, whose worktrees no registry on disk lists.
 */
import { existsSync } from "node:fs";
import { pruneCheckouts, type UnansweredLooks } from "../engine/catalog/repository-registry.js";
import { openRerailDatabase } from "../engine/storage/database.js";

/** How often a running daemon forgets gone checkouts, so a worktree Git removed stays recorded at most this long. */
const CHECKOUT_PRUNING_INTERVAL_MS = 60 * 60 * 1000;
/**
 * How long a pass may wait on the filesystem before it gives up. A stat on a hard-mounted share whose server is down
 * never answers, and a pass waiting on it would keep every later one from starting for the daemon's life. A pass asks
 * the filesystem and Git one thing at a time, and while something it gave up on is unanswered, later passes look at
 * nothing at all: not another checkout on that share, nor a worktree on it that Git lists. So a dead share holds one
 * pool thread or one Git, never one per pass, and pruning starts again once the share answers.
 */
const CHECKOUT_PRUNING_PASS_LIMIT_MS = 15 * 60 * 1000;

export interface CheckoutPruning {
  /**
   * Stops the passes. A pass under way forgets nothing and ends at once, even while a share has not answered it; its
   * connection is closed once this settles.
   */
  stop(): Promise<void>;
}

export interface CheckoutPruningOptions {
  readonly intervalMs?: number;
  /** How long one pass may take before it gives up, forgetting nothing, so the next one can start. */
  readonly passLimitMs?: number;
  readonly log?: (line: string) => void;
}

/**
 * Forget gone checkouts now and every `intervalMs`, one pass at a time, each over its own connection so the API's
 * reads see the change. A daemon with no database yet does not create one. It never throws and never holds the
 * daemon open; a failed pass is logged and the next one tries again, as is a pass still waiting on the filesystem after
 * `passLimitMs`: it gives up, forgetting nothing, and the call it waits on keeps its pool thread until answered.
 */
export function startCheckoutPruning(databasePath: string, options: CheckoutPruningOptions = {}): CheckoutPruning {
  const log = options.log ?? (line => process.stderr.write(line));
  const passLimitMs = options.passLimitMs ?? CHECKOUT_PRUNING_PASS_LIMIT_MS;
  let stopped = false;
  const unanswered: UnansweredLooks = { looking: undefined };
  let running: { readonly done: Promise<void>; readonly abort: AbortController } | undefined;
  const pass = (): void => {
    if (stopped || running !== undefined || !existsSync(databasePath)) return;
    const abort = new AbortController();
    const limit = setTimeout(() => {
      log(`rerail daemon: forgetting removed checkouts: gave up on a pass the filesystem had not answered after ${Math.round(passLimitMs / 1000)} s; later passes look at nothing until it answers\n`);
      abort.abort();
    }, passLimitMs);
    limit.unref();
    const done = prune(databasePath, abort.signal, unanswered)
      .catch((error: unknown) => log(`rerail daemon: forgetting removed checkouts: ${error instanceof Error ? error.message : String(error)}\n`))
      .finally(() => {
        clearTimeout(limit);
        running = undefined;
      });
    running = { done, abort };
  };
  const timer = setInterval(pass, options.intervalMs ?? CHECKOUT_PRUNING_INTERVAL_MS);
  timer.unref();
  pass();
  return {
    async stop() {
      stopped = true;
      clearInterval(timer);
      running?.abort.abort();
      await running?.done;
    },
  };
}

async function prune(databasePath: string, signal: AbortSignal, unanswered: UnansweredLooks): Promise<void> {
  const database = openRerailDatabase(databasePath);
  try {
    await pruneCheckouts(database, { signal, unanswered });
  } finally {
    database.$client.close();
  }
}
