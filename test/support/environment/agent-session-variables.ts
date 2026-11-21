/**
 * The variables that mark a coding agent's session (src/engine/agent-session.ts), which the test setup clears so the suite
 * reads as a person's terminal whoever runs it. test/cli/project-cli.test.ts keeps this list equal to the CLI's.
 */
export const CLEARED_AGENT_SESSION_VARIABLES = ["CLAUDECODE", "CODEX_THREAD_ID", "CURSOR_AGENT", "PI_CODING_AGENT", "AI_AGENT"];
