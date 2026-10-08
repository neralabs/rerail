import { realpathSync, statSync } from "node:fs";
import { isAbsolute, sep } from "node:path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { physicalPath } from "../checkout-control.js";
import { duplicateJsonKeyError } from "../json-duplicate-keys.js";
import { CALLER_CWD_HEADER, normalizeFolder, type ApiCaller } from "../repository-model.js";
import { repositoryIdentityLocation } from "../repository-identity.js";
import { allowedHost, type PublicHosts } from "./http.js";
import { ApiError, invokeOperation, type Operation, type RerailApiStores } from "./operation.js";
import { operationInputShape } from "./operations.js";
import { within } from "./repository-operations.js";

/** Every operation becomes one MCP tool with the same id, description, and schemas as its HTTP route. */
export function createRerailMcpServer(
  stores: RerailApiStores,
  operations: readonly Operation[],
  info: { version: string },
  caller: ApiCaller = { kind: "agent", checkout: undefined, folder: "" },
): McpServer {
  const server = new McpServer({ name: "rerail", version: info.version });
  for (const operation of operations) {
    server.registerTool(
      operation.id,
      {
        description: operation.summary,
        inputSchema: operationInputShape(operation),
        outputSchema: operation.output,
        annotations: {
          readOnlyHint: operation.method === "GET",
          destructiveHint: operation.method === "DELETE",
          idempotentHint: operation.method !== "POST",
        },
      },
      input => {
        try {
          const result = invokeOperation(operation, stores, input as Record<string, unknown>, caller) as Record<string, unknown>;
          return { content: [{ type: "text", text: JSON.stringify(result) }], structuredContent: result };
        } catch (error) {
          const message = error instanceof ApiError ? `${error.code}: ${error.message}` : (error as Error).message;
          return { isError: true, content: [{ type: "text", text: message }] };
        }
      },
    );
  }
  return server;
}

/** The JSON-RPC id of a request body, for an error answered before the SDK sees it. */
function requestId(text: string): unknown {
  try {
    const parsed: unknown = JSON.parse(text);
    return typeof parsed === "object" && parsed !== null && !Array.isArray(parsed) && "id" in parsed ? parsed.id : null;
  } catch {
    return null;
  }
}

const invalidParams = (id: unknown, message: string): Response =>
  Response.json({ jsonrpc: "2.0", id, error: { code: -32602, message } }, { status: 400 });

/** One answer for every unusable caller path, so the endpoint doesn't reveal which paths exist. */
const INVALID_CALLER = `${CALLER_CWD_HEADER} must be the URI-encoded absolute path of an existing folder`;

/** Checkout root of each caller folder already resolved; a bridge sends the same folder on every request. */
const callerCheckouts = new Map<string, string>();
const CALLER_CACHE_LIMIT = 1_024;

/**
 * The calling agent from {@link CALLER_CWD_HEADER}: the URI-encoded real path of the folder its bridge started in,
 * resolved to the checkout that folder is in and the folder inside it. No header is an agent with no checkout, which
 * must name its target. Returns the reason when the header is unusable.
 */
export function callerFromHeader(header: string | null): ApiCaller | string {
  if (header === null) return { kind: "agent", checkout: undefined, folder: "" };
  let real: string;
  try {
    const cwd = decodeURIComponent(header);
    if (cwd.includes("\0") || !isAbsolute(cwd)) return INVALID_CALLER;
    real = realpathSync(cwd);
    if (!statSync(real).isDirectory()) return INVALID_CALLER;
  } catch {
    return INVALID_CALLER;
  }
  try {
    let checkout = callerCheckouts.get(real);
    if (checkout === undefined) {
      checkout = physicalPath(repositoryIdentityLocation(real).workspaceRoot);
      if (callerCheckouts.size >= CALLER_CACHE_LIMIT) callerCheckouts.clear();
      callerCheckouts.set(real, checkout);
    }
    const inside = within(checkout, real);
    return { kind: "agent", checkout, folder: inside === undefined ? "" : normalizeFolder(inside.split(sep).join("/")) };
  } catch {
    return INVALID_CALLER;
  }
}

/**
 * Stateless Streamable HTTP endpoint: POST-only JSON responses, no sessions or
 * standalone SSE listener. Advertising an SSE stream and then closing its
 * per-request transport makes clients treat the connection as lost.
 * The SDK forbids reusing a stateless transport, so each POST gets a fresh pair.
 *
 * The daemon's host allowlist runs before this handler; it is checked again here so the endpoint
 * defends itself wherever it is mounted. The SDK's own `allowedHosts` is deprecated and matches
 * the whole `host:port` string, which cannot express "any port" for a port-mapped daemon.
 *
 * The SDK parses the body with JSON.parse, which keeps the last of repeated keys, so a tool's arguments could
 * silently lose a field: a body that repeats a key anywhere is refused as invalid params before the SDK sees it,
 * as is an unusable caller header.
 */
export function createRerailMcpHandler(
  stores: RerailApiStores,
  operations: readonly Operation[],
  info: { version: string; publicHosts?: PublicHosts },
): (request: Request) => Promise<Response> {
  const publicHosts = info.publicHosts ?? new Set<string>();
  return async request => {
    if (!allowedHost(request.headers.get("host") ?? undefined, publicHosts)) {
      return Response.json({ error: "host-not-allowed" }, { status: 421 });
    }
    if (request.method !== "POST") {
      return new Response(null, { status: 405, headers: { Allow: "POST" } });
    }
    const text = await request.clone().text();
    const duplicate = duplicateJsonKeyError(text, "request body");
    if (duplicate !== undefined) return invalidParams(requestId(text), duplicate);
    const caller = callerFromHeader(request.headers.get(CALLER_CWD_HEADER));
    if (typeof caller === "string") return invalidParams(requestId(text), caller);
    const server = createRerailMcpServer(stores, operations, info, caller);
    const transport = new WebStandardStreamableHTTPServerTransport({ enableJsonResponse: true });
    await server.connect(transport);
    try {
      return await transport.handleRequest(request);
    } finally {
      await transport.close();
      await server.close();
    }
  };
}
