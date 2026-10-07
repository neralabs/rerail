import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { duplicateJsonKeyError } from "../json-duplicate-keys.js";
import { allowedHost, type PublicHosts } from "./http.js";
import { ApiError, operationInputShape, type Operation, type RerailApiStores } from "./operations.js";

/** Every operation becomes one MCP tool with the same id, description, and schemas as its HTTP route. */
export function createRerailMcpServer(
  stores: RerailApiStores,
  operations: readonly Operation[],
  info: { version: string },
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
          const result = operation.handler(stores, input as Record<string, unknown>) as Record<string, unknown>;
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

/**
 * The SDK parses the body with JSON.parse, which keeps the last of repeated keys, so a tool's arguments could
 * silently lose a field. A body that repeats a key anywhere is refused as invalid params before the SDK sees it.
 */
async function rejectDuplicateKeys(request: Request): Promise<Response | undefined> {
  const text = await request.clone().text();
  const message = duplicateJsonKeyError(text, "request body");
  if (message === undefined) return undefined;
  const parsed: unknown = JSON.parse(text);
  const id = typeof parsed === "object" && parsed !== null && !Array.isArray(parsed) && "id" in parsed ? parsed.id : null;
  return Response.json({ jsonrpc: "2.0", id, error: { code: -32602, message } }, { status: 400 });
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
    const duplicate = await rejectDuplicateKeys(request);
    if (duplicate !== undefined) return duplicate;
    const server = createRerailMcpServer(stores, operations, info);
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
