/**
 * Install and update: harness registration (the hooks and commands written into each coding agent's config),
 * the managed updater, and rule packs from npm.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { CLAUDE_STATUS_DATABASE_FILE, type ClaudeStatusLine } from "./claude-status-store.js";
export {
  CURRENT_LINK,
  LAUNCHER_DIR,
  ManagedUpdater,
  UPDATE_LOCK_DIR,
  UPDATE_STATE_FILE,
  type UpdateResult,
  type UpdateStatus,
  VERSIONS_DIR,
} from "./updater.js";
export { codingAgentHome, codingAgentStates, registrationRoot, SEEN_AGENTS_FILE } from "./coding-agents.js";
export { CLAUDE_HOOK_EVENTS, registerHarnesses } from "./harness-registration.js";
export { LAUNCHER_PATH_FILE, launcherPathMarker } from "./shell-path.js";
export { type Harness } from "./harness-registration.js";
export {
  addPackSpecs,
  discoverPacks,
  installedPackagesOf,
  installedPackVersion,
  installPackVersions,
  newerPackVersions,
  npmPackStore,
  packsLeftOutByCore,
  parsePackVersions,
  type PackDiscovery,
  type PackStore,
  type PackVersion,
} from "./rule-pack-store.js";
