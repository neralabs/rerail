/**
 * Install and update: harness registration (the hooks and commands written into each coding agent's config),
 * the managed updater, and rule packs from npm.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { type ClaudeStatusLine } from "./claude-status-store.js";
export { ManagedUpdater, type UpdateResult, type UpdateStatus } from "./updater.js";
export { codingAgentHome, codingAgentStates, registrationRoot } from "./coding-agents.js";
export { registerHarnesses } from "./harness-registration.js";
export { type Harness } from "./harness-registration.js";
export {
  addPackSpecs,
  discoverPacks,
  installPackVersions,
  npmPackStore,
  packsLeftOutByCore,
  parsePackVersions,
  type PackDiscovery,
  type PackStore,
  type PackVersion,
} from "./rule-pack-store.js";
