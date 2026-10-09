/**
 * The Claude Code hooks and status line.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { claudeHookEvents, createClaudeEventHandler } from "./claude-event-adapter.js";
export { runClaudeHook } from "./claude-hook.js";
