/**
 * Install and update: harness registration (the hooks and commands written into each coding agent's config)
 * and the managed updater.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { type ClaudeStatusLine } from "./claude-status-store.js";
export { ManagedUpdater } from "./updater.js";
