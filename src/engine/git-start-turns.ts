/**
 * Git processes start one per turn of the event loop. Starting a process holds the thread that starts it: libuv forks
 * on Linux and macOS, and a fork copies the page tables of the whole process, about 30 ms for each GB it holds; on
 * Windows it calls CreateProcess, 15 to 40 ms. Reads asked for together (a checkout's Git view, the verdicts of a walk,
 * each session walking the other worktrees) would start all their processes in one turn, and every socket of the
 * process would wait for all of them: in the daemon, a hook's health probe and event and the dashboard's reads, which a
 * hook gives up on when they take too long, losing its event. Each start waits for a turn of its own instead, after the
 * loop has polled for I/O, so the loop answers its sockets between any two starts.
 *
 * Starts go in the order they were asked for, except that work nothing waits for (the walk of the other worktrees,
 * `inGitBackground`) lets the starts of everything else go first, a hook's own reads among them. It still gets one
 * start in every `BACKGROUND_SHARE` while others wait, so it never stops.
 */
import { AsyncLocalStorage } from "node:async_hooks";

/** One start in this many goes to background work while other starts wait too. */
export const BACKGROUND_SHARE = 4;

const background = new AsyncLocalStorage<true>();
const waiting = { foreground: [] as Array<() => void>, background: [] as Array<() => void> };
let scheduled = false;
/** Foreground starts let go since the last background one. */
let sinceBackground = 0;

/**
 * Runs `work` as work nothing waits for: the Git processes it starts, and those its continuations start, wait behind
 * everyone else's (see the module comment).
 */
export function inGitBackground<T>(work: () => T): T {
  return background.run(true, work);
}

/** Resolves in a turn of the event loop in which no other Git process has started; start the process then. */
export async function gitStartTurn(): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  (background.getStore() === true ? waiting.background : waiting.foreground).push(resolve);
  if (!scheduled) {
    scheduled = true;
    setImmediate(startNext);
  }
  await promise;
}

/**
 * Lets one waiting start go in the loop's check phase, which follows its poll for I/O. The start runs as this
 * callback's continuation, before any other check-phase callback; the next one waits for the next check phase.
 */
function startNext(): void {
  const backgroundTurn = waiting.background.length > 0
    && (waiting.foreground.length === 0 || sinceBackground >= BACKGROUND_SHARE - 1);
  if (backgroundTurn) {
    sinceBackground = 0;
    waiting.background.shift()?.();
  } else {
    sinceBackground += 1;
    waiting.foreground.shift()?.();
  }
  if (waiting.foreground.length + waiting.background.length > 0) setImmediate(startNext);
  else scheduled = false;
}
