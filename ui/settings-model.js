/** What Settings derives from the daemon's answers, apart from the page: coding agent states and the install date. */

/**
 * A coding agent's row (`GET /api/coding-agents`): its state, `tone` "on" when connected, and the command a person
 * runs in a terminal to change it (`null` when there is nothing to change). A problem wins: the check did not finish.
 */
export function agentStatus(agent) {
  if (agent.problem) return { state: "Couldn’t check", tone: "warn", command: null, problem: agent.problem };
  if (agent.connected) return { state: "Connected", tone: "on", command: { text: `rerail install --remove --agents ${agent.id}`, label: "Copy disconnect command" } };
  if (agent.found) return { state: "On this computer, not connected", tone: "", command: { text: `rerail install --agents ${agent.id}`, label: "Copy connect command" } };
  return { state: "Not found on this machine", tone: "off", command: null };
}

/** "Sep 3, 2026" for `/healthz.installedAt` (epoch ms, local calendar day); `null` while unknown. */
export const installedLabel = ms => (ms === null || ms === undefined
  ? null
  : new Date(ms).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" }));
