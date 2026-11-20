/**
 * How long Rerail waits for its daemon: for a start to bring it up, for one health probe, and for a busy daemon to answer
 * a hook. They live apart from `daemon-control.ts`, which reads each as it waits, so a test can scale them (`vi.mock`)
 * and still go through the same waits in the same order.
 */

/**
 * How long a start waits, each in turn: for the start lock, for a daemon that holds the home to record itself
 * (`lookupHomeDaemon`), and for the daemon it launched to answer (`readyDaemon`).
 */
export const START_TIMEOUT_MS = 15_000;
/** How long one health probe of the daemon, or its engine's proof on Windows, waits for an answer. */
export const PROBE_TIMEOUT_MS = 2_000;
/** How long, in all, a hook's second health probe waits for a busy daemon to answer as itself. */
export const BUSY_DAEMON_WAIT_MS = 10_000;
/** How long, in all, a hook waits for a busy daemon from the event's arrival: the first probe and the busy wait after it. */
export const HOOK_DAEMON_WAIT_MS = PROBE_TIMEOUT_MS + BUSY_DAEMON_WAIT_MS;
