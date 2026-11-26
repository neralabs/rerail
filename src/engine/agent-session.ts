/**
 * Whether a CLI command runs in a coding agent's session, read from the variable each coding agent sets for the commands
 * its shell tool runs. The commands that approve rules (`rules approve`, `rules import`) are a person's, as the API's
 * are for an MCP caller: from an agent session they are refused. `rules add` approves the rule it stores where the
 * person approved the command itself: in their own terminal, or in a session whose agent asks them before it runs it
 * ({@link AgentSession.asksFirst}). In any other agent's session the rule waits for a person's approval, and an edit of
 * an approved rule of yours is stored as a copy that waits.
 *
 * It holds a coding agent to the path it is given, as the API's check does. It is no boundary: the agent runs as the
 * person, and can unset a variable before it runs a command; the agent then asks for a command its command files do
 * not pre-allow. A host in a mode that approves every command asks for none. OMP sets none of these for its shell, so
 * it is not seen.
 */

/**
 * Each variable a coding agent sets in its shell, the agent it names, and whether that agent asks the person before it
 * runs `rerail rules add`; a non-empty value counts. Claude Code asks for a command its command files do not pre-allow
 * (`rerail-command-files.ts` leaves `rules add` out), and Codex runs the launcher only through its own approval. Cursor
 * asks nothing here, since Rerail's Cursor hook allows every tool call, and Pi has no approval prompt.
 */
const MARKERS: ReadonlyArray<readonly [variable: string, agent: string, asksFirst: boolean]> = [
  ["CLAUDECODE", "Claude Code", true],
  ["CODEX_THREAD_ID", "Codex", true],
  ["CURSOR_AGENT", "Cursor", false],
  ["PI_CODING_AGENT", "Pi", false],
  // The variable several coding agents set as well, Claude Code and Pi among them.
  ["AI_AGENT", "a coding agent", false],
];

/** Every variable that marks a coding agent's session, for a test to clear. */
export const AGENT_SESSION_VARIABLES: readonly string[] = MARKERS.map(([variable]) => variable);

/** The coding agent a command runs for, and the variable that says so. */
export interface AgentSession {
  readonly agent: string;
  readonly variable: string;
  /** Whether the agent asks the person before it runs `rerail rules add`, so the command running is their approval. */
  readonly asksFirst: boolean;
}

/** The coding agent whose session runs this command, from `env`; undefined in a person's own terminal. */
export function agentSession(env: NodeJS.ProcessEnv = process.env): AgentSession | undefined {
  const found = MARKERS.find(([variable]) => (env[variable] ?? "") !== "");
  return found === undefined ? undefined : { agent: found[1], variable: found[0], asksFirst: found[2] };
}

/**
 * Whether `rules add` run in `session` ran with the person's approval of the command, so the rule it stores is approved:
 * in the person's own terminal (no session), or in a session whose agent asked them first.
 */
export function addIsApproved(session: AgentSession | undefined): boolean {
  return session === undefined || session.asksFirst;
}

/** Why a person's command is refused in `session`: who runs it, and what tells. */
export function agentRunsText(command: string, session: AgentSession): string {
  return `${command} is a person's, and ${session.agent} runs this command (${session.variable} is set)`;
}
