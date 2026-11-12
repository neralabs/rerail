/**
 * Git processes start one per turn of the event loop. Starting a process holds the thread that starts it: libuv forks
 * on Linux and macOS, and a fork copies the page tables of the whole process, about 30 ms for each GB it holds; on
 * Windows it calls CreateProcess, 15 to 40 ms. Reads asked for together (a checkout's Git view, the verdicts of a walk,
 * each session taking the same walk) would start all their processes in one turn, and every socket of the process
 * would wait for all of them: in the daemon, a hook's health probe and event and the dashboard's reads, which a hook
 * gives up on when they take too long, losing its event. Each start waits for a turn of its own instead, after the
 * loop has polled for I/O, in the order the starts were asked for, so the loop answers its sockets between any two
 * starts. A start waits no longer than the rest of the turn it was asked in plus one turn per start ahead of it.
 */

const waiting: Array<() => void> = [];
let scheduled = false;

/** Resolves in a turn of the event loop in which no other Git process has started; start the process then. */
export async function gitStartTurn(): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  waiting.push(resolve);
  if (!scheduled) {
    scheduled = true;
    setImmediate(startNext);
  }
  await promise;
}

/**
 * Lets the oldest waiting start go in the loop's check phase, which follows its poll for I/O. The start runs as this
 * callback's continuation, before any other check-phase callback; the next one waits for the next check phase.
 */
function startNext(): void {
  waiting.shift()?.();
  if (waiting.length > 0) setImmediate(startNext);
  else scheduled = false;
}
