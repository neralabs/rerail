import { statSync } from "node:fs";
import { isAbsolute, sep } from "node:path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { checkoutRealpath, physicalPath } from "../engine/catalog/checkout-control.js";
import { duplicateJsonKeyError } from "../engine/json-duplicate-keys.js";
import { CALLER_CWD_HEADER, normalizeFolder, type ApiCaller } from "../engine/catalog/repository-model.js";
import { locationNow, recordedCheckoutsVersion, stillPlaces, type KeptLocation } from "../engine/catalog/repository-registry.js";
import { allowedHost, type PublicHosts } from "./http.js";
import { ApiError, invokeOperation, operationAvailable, type Operation, type RerailApiStores } from "./operation.js";
import { operationInputShape } from "./operations.js";
import { within } from "./repository-operations.js";

/**
 * Every operation becomes one MCP tool with the same id, description, and schemas as its HTTP route, but for one of an
 * experimental feature this daemon runs without: a coding agent is never offered a tool that would only refuse.
 */
export function createRerailMcpServer(
  stores: RerailApiStores,
  operations: readonly Operation[],
  info: { version: string },
  caller: ApiCaller = { kind: "agent", checkout: undefined, folder: "" },
): McpServer {
  const server = new McpServer({ name: "rerail", version: info.version });
  for (const operation of operations) {
    if (!operationAvailable(operation)) continue;
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
      async input => {
        try {
          const result = await invokeOperation(operation, stores, input as Record<string, unknown>, caller) as Record<string, unknown>;
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

/**
 * Each caller folder in Git already resolved, with its location as the checkout resolver reads it ({@link locationNow})
 * and the checkout root that location names; a bridge sends the same folder on every request. One serves while its
 * location still places the folder ({@link stillPlaces}), so a folder made a repository of its own (`git init` in it
 * or in a folder between it and its top level) resolves to that repository at its next request, at one lstat per
 * folder between and with no Git process. All are forgotten whenever the recorded checkouts change, so a folder that
 * became a recorded checkout of its own resolves to it.
 */
const callerCheckouts = new Map<string, { readonly kept: KeptLocation; readonly checkout: string }>();
const CALLER_CACHE_LIMIT = 1_024;
let callerCacheVersion: string | undefined;

/**
 * The checkout root of the caller folder `real`: the kept one while its location still places the folder, else read
 * afresh. As the checkout resolver does, only a location in Git is kept: a folder outside Git is read again at each
 * request, so a later `git init` above it is seen, and so is the next answer after a Git that gave none.
 */
function callerCheckout(real: string): string {
  const known = callerCheckouts.get(real);
  if (known !== undefined && stillPlaces(known.kept)) return known.checkout;
  const kept = locationNow(real);
  const checkout = physicalPath(kept.location.workspaceRoot);
  if (!kept.location.git) return checkout;
  if (callerCheckouts.size >= CALLER_CACHE_LIMIT) callerCheckouts.clear();
  callerCheckouts.set(real, { kept, checkout });
  return checkout;
}

/** Forgets every kept caller checkout once `recordedVersion`, the recorded checkouts' version, moves. */
function forgetOnRecordedChange(recordedVersion: string | undefined): void {
  if (recordedVersion === undefined || recordedVersion === callerCacheVersion) return;
  callerCheckouts.clear();
  callerCacheVersion = recordedVersion;
}

/**
 * The real path of the folder `header` names, URI-encoded; undefined where it is not the absolute path of an existing
 * folder.
 */
function callerFolder(header: string): string | undefined {
  try {
    const cwd = decodeURIComponent(header);
    if (cwd.includes("\0") || !isAbsolute(cwd)) return undefined;
    // Spelt as the checkout is (on Windows each name's long form), or a folder sent with 8.3 short names (an account
    // folder named `RUNNER~1`) lies outside it.
    const real = checkoutRealpath(cwd);
    return statSync(real).isDirectory() ? real : undefined;
  } catch {
    return undefined;
  }
}

/**
 * The calling agent from {@link CALLER_CWD_HEADER}: the URI-encoded real path of the folder its bridge started in,
 * resolved to the checkout that folder is in and the folder inside it. No header is an agent with no checkout, which
 * must name its target. Returns the reason when the header is unusable.
 */
export function callerFromHeader(header: string | null, recordedVersion?: string): ApiCaller | string {
  if (header === null) return { kind: "agent", checkout: undefined, folder: "" };
  forgetOnRecordedChange(recordedVersion);
  const real = callerFolder(header);
  if (real === undefined) return INVALID_CALLER;
  try {
    const checkout = callerCheckout(real);
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
    const header = request.headers.get(CALLER_CWD_HEADER);
    const caller = callerFromHeader(header, header === null ? undefined : recordedCheckoutsVersion(stores.database()));
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
