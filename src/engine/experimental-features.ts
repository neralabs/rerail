/**
 * Experimental features: off unless turned on in the process that runs them (the daemon for OMP, Claude Code, Codex
 * and Cursor sessions, its API and dashboard, plus Cursor's hook for its `stop` follow-up; the CLI process for
 * `rerail merge-assist`; the coding agent itself for a Pi session running Rerail in process). Their behaviour may
 * change or go away.
 *
 * Each feature has its own variable, `RERAIL_EXPERIMENTAL_<FEATURE>`: `1` turns it on and `0` off, whatever the
 * blanket {@link EXPERIMENTAL_ALL} says. With the feature's variable unset (or any other value), `RERAIL_EXPERIMENTAL=1`
 * turns it on; otherwise it is off.
 */

/** The blanket switch: `1` turns on every experimental feature whose own variable is neither `1` nor `0`. */
export const EXPERIMENTAL_ALL = "RERAIL_EXPERIMENTAL";

/** Whether `feature` (`RERAIL_EXPERIMENTAL_<feature>`) is on in `env`: its own `1` or `0`, else the blanket switch. */
function experimentOn(feature: string, env: NodeJS.ProcessEnv): boolean {
  const own = env[`${EXPERIMENTAL_ALL}_${feature}`];
  if (own === "1" || own === "0") return own === "1";
  return env[EXPERIMENTAL_ALL] === "1";
}

/**
 * Experimental: a code-rule steer whose check finishes after its step has moved on is held, and the agent
 * gets one digest of those findings, per rule with files and spans, when it finishes its run. Off, such a steer reaches
 * the agent at its next event or poll. `RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST`.
 */
export function missedSteerDigestEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentOn("MISSED_STEER_DIGEST", env);
}

/**
 * Experimental: a checkout's saved `steerStrategy` other than `message` (`rewind`, `subagent`) takes effect.
 * Off, every code-rule steer goes out as `message`, whatever the checkout saved. `RERAIL_EXPERIMENTAL_STEER_STRATEGIES`.
 */
export function steerStrategiesEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentOn("STEER_STRATEGIES", env);
}

/**
 * Experimental: the first session in a checkout starts one full-repository audit in log mode. Off, a session audits
 * only what it changes; `rerail scan` still audits the repository when asked. `RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT`.
 */
export function firstRepositoryAuditEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentOn("FIRST_REPOSITORY_AUDIT", env);
}

/**
 * Experimental: Merge Assist. Off, nothing of it runs (no detection, scan, `gh`, conflict hook command, agent note,
 * person's line or footer segment), even for a repository a person turned it on for; its API operations answer
 * `404 experimental-off`, and `rerail merge-assist` refuses. On, a repository still has it only once a person turns it
 * on there. `RERAIL_EXPERIMENTAL_MERGE_ASSIST`.
 */
export function mergeAssistExperimentEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentOn("MERGE_ASSIST", env);
}
