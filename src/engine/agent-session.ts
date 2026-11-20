/**
 * Whether a CLI command runs in a coding agent's session, read from the variable each coding agent sets for the commands
 * its shell tool runs. The commands that approve rules or replace an approved one (`rules approve`, `rules import`, and
 * `rules add` over an approved rule of yours) are a person's, as the API's are for an MCP caller: from an agent session
 * they are refused or, for `rules add`, store the edit as a copy that waits for a person's approval.
 *
 * It holds a coding agent to the path it is given, as the API's check does. It is no boundary: the agent runs as the
 * person, and can unset a variable before it runs a command. OMP sets none of these for its shell, so it is not seen.
 */

/** Each variable a coding agent sets in its shell, and the agent it names; a non-empty value counts. */
const MARKERS: ReadonlyArray<readonly [variable: string, agent: string]> = [
  ["CLAUDECODE", "Claude Code"],
  ["CODEX_THREAD_ID", "Codex"],
  ["CURSOR_AGENT", "Cursor"],
  ["PI_CODING_AGENT", "Pi"],
  // The variable several coding agents set as well, Claude Code and Pi among them.
  ["AI_AGENT", "a coding agent"],
];

/** Every variable that marks a coding agent's session, for a test to clear. */
export const AGENT_SESSION_VARIABLES: readonly string[] = MARKERS.map(([variable]) => variable);

/** The coding agent a command runs for, and the variable that says so. */
export interface AgentSession {
  readonly agent: string;
  readonly variable: string;
}

/** The coding agent whose session runs this command, from `env`; undefined in a person's own terminal. */
export function agentSession(env: NodeJS.ProcessEnv = process.env): AgentSession | undefined {
  const found = MARKERS.find(([variable]) => (env[variable] ?? "") !== "");
  return found === undefined ? undefined : { agent: found[1], variable: found[0] };
}

/** Why a person's command is refused in `session`: who runs it, and what tells. */
export function agentRunsText(command: string, session: AgentSession): string {
  return `${command} is a person's, and ${session.agent} runs this command (${session.variable} is set)`;
}
