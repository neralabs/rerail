/** What the daemon process's way out does around `RerailDaemon.close`; the daemon passes its own, tests theirs. */
export interface DaemonExitSteps {
  close: () => Promise<void>;
  /** After a clean close: gives up the main-instance record. */
  release: () => void;
  /** Starts the next daemon, with this one's arguments. */
  relaunch: () => void;
  exit: (code: number) => void;
  log: (line: string) => void;
}

const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/**
 * The daemon process's one way out. `stop` (a signal, or a shutdown request on Windows) closes the daemon and exits;
 * `restart` (the heap ceiling) closes it, starts the next daemon and exits. The first to ask wins and the other does
 * nothing: a stop during a restart closes nothing twice, and a restart during a stop never starts a daemon that
 * `rerail daemon stop` was ending. A failed release or relaunch is said and exits 1 like a failed close: it used to
 * escape as an unhandled rejection, a crash that wrote neither why nor the exit code.
 */
export function daemonExit(steps: DaemonExitSteps): { stop: () => void; restart: () => void } {
  let closing = false;
  const closeThen = (next: (() => void) | undefined, what: string): void => {
    if (closing) return;
    closing = true;
    const fail = (error: unknown): void => {
      steps.log(`rerail daemon: ${what} failed: ${errorText(error)}\n`);
      steps.exit(1);
    };
    steps.close().then(() => {
      try {
        steps.release();
        next?.();
      } catch (error) {
        fail(error);
        return;
      }
      steps.exit(0);
    }, fail);
  };
  return { stop: () => closeThen(undefined, "shutdown"), restart: () => closeThen(steps.relaunch, "restart") };
}
