/**
 * The Codex hooks.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { codexHookEvents, createCodexEventHandler } from "./codex-event-adapter.js";
export { runCodexHook } from "./codex-hook.js";
