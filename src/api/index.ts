/**
 * The daemon's HTTP and MCP API: operations, request and response contracts, and the stdio MCP bridge.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { createRerailApi, publicHostSet } from "./http.js";
export { createRerailMcpHandler } from "./mcp.js";
export { auditControlsResponseSchema, operations, type RerailApiStores } from "./operations.js";
export { rulePackListSchema, rulePackRecommendationsSchema } from "./rule-pack-contracts.js";
export { presentationResponseSchema, type sessionResponseSchema } from "./session-contracts.js";
