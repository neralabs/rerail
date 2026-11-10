/**
 * Experimental features: off unless their environment variable is `1` in the process that runs them (the daemon for
 * OMP, Claude Code, Codex and Cursor sessions, plus Cursor's hook for its `stop` follow-up; the coding agent itself for
 * a Pi session running Rerail in process). Their behaviour may change or go away.
 */

/**
 * Experimental: a code-rule steer whose check finishes after its step has moved on is held, and the agent
 * gets one digest of those findings, per rule with files and spans, when it finishes its run. Off, such a steer reaches
 * the agent at its next event or poll.
 */
export function missedSteerDigestEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST === "1";
}

/**
 * Experimental: a checkout's saved `steerStrategy` other than `message` (`rewind`, `subagent`) takes effect.
 * Off, every code-rule steer goes out as `message`, whatever the checkout saved.
 */
export function steerStrategiesEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.RERAIL_EXPERIMENTAL_STEER_STRATEGIES === "1";
}

/**
 * Experimental: Merge Assist's decision ledger. A person's verdict on a conflict also writes a ledger entry
 * (`.rerail/decisions/<id>.json`) that travels with the branch, and every detection reads the ledger view computed from
 * Git: statuses and the merge check, precedent across checkouts, `verdicts` conflicts, reopening by fingerprint, and
 * holding other branches' unmerged verdicts. Off, a verdict settles the conflict locally and writes only the decision
 * this branch follows (`.rerail/candidates/`), and nothing reads the ledger.
 */
export function decisionLedgerEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.RERAIL_EXPERIMENTAL_DECISION_LEDGER === "1";
}
