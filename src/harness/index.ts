/**
 * Harness-neutral host plumbing the hosts share: hook adapters and launcher, the event stream, the native session
 * client, semantic tools and the `/rerail` commands.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { type HookInput, type HookOptions, type HookOutput, hookRerailStarted, serveHook } from "./hook-host.js";
export { NativeSessionClient } from "./native-session-client.js";
export { MANAGED_SKILL_MARKER } from "./rerail-command-files.js";
export { registerRerailCommands, type RerailCommandContext, type RerailCommandHost } from "./rerail-command.js";
export { RERAIL_COMMANDS } from "./rerail-commands.js";
