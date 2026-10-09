/**
 * The resident daemon: the HTTP/MCP server, the page, and the native session server.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { DEFAULT_DAEMON_PORT, defaultEngineSocketPath, RerailDaemon } from "./daemon.js";
