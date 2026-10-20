/**
 * Experimental features: off unless their environment variable is `1` in the process that runs them (the daemon for
 * OMP, Claude Code, Codex and Cursor sessions, plus Cursor's hook for its `stop` follow-up; the coding agent itself for
 * a Pi session running Rerail in process). Their behaviour may change or go away.
 */

/**
 * Experimental (NER-1955): a code-rule steer whose check finishes after its step has moved on is held, and the agent
 * gets one digest of those findings, per rule with files and spans, when it finishes its run. Off, such a steer reaches
 * the agent at its next event or poll.
 */
export function missedSteerDigestEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST === "1";
}
