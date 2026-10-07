import { once } from "node:events";
import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";
import { resolveRerailWorkspaceRoute, type RerailWorkspaceRoute } from "./dev-routing.js";

export type RerailMcpBridgeFetch = (endpoint: URL, init: RequestInit) => Promise<Response>;

export interface RerailMcpBridgeOptions {
  cwd?: string;
  input?: Readable;
  stdout?: Writable;
  stderr?: Writable;
  fetch?: RerailMcpBridgeFetch;
}

type JsonRpcId = string | number;

interface ParsedRequestLine {
  batch: boolean;
  ids: readonly JsonRpcId[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isJsonRpcMessage(value: unknown): boolean {
  return isRecord(value) && value.jsonrpc === "2.0";
}

function isJsonRpcBody(body: string): boolean {
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    return false;
  }
  return Array.isArray(parsed) ? parsed.length > 0 && parsed.every(isJsonRpcMessage) : isJsonRpcMessage(parsed);
}

/** Collects the ids of JSON-RPC requests (not responses or notifications) in one stdin line. */
function parseRequestLine(line: string): ParsedRequestLine {
  let parsed: unknown;
  try {
    parsed = JSON.parse(line);
  } catch {
    return { batch: false, ids: [] };
  }
  const messages = Array.isArray(parsed) ? parsed : [parsed];
  const ids = messages.flatMap((message): JsonRpcId[] =>
    isRecord(message) && "method" in message && (typeof message.id === "string" || typeof message.id === "number")
      ? [message.id]
      : []);
  return { batch: Array.isArray(parsed), ids };
}

function errorResponses(request: ParsedRequestLine, message: string): string | undefined {
  if (request.ids.length === 0) return undefined;
  const responses = request.ids.map((id) => ({ jsonrpc: "2.0", id, error: { code: -32000, message } }));
  return JSON.stringify(request.batch ? responses : responses[0]);
}

async function writeLine(output: Writable, line: string): Promise<void> {
  if (!output.write(`${line}\n`)) await once(output, "drain");
}

type RerailMcpDestination =
  | { kind: "endpoint"; url: URL }
  | { kind: "error"; message: string };

function resolveMcpDestination(route: RerailWorkspaceRoute): RerailMcpDestination {
  switch (route.kind) {
    case "workspace":
      return { kind: "endpoint", url: new URL("/mcp", route.instance.url) };
    case "host":
      return { kind: "endpoint", url: new URL("http://127.0.0.1:4737/mcp") };
    case "disabled":
      return { kind: "error", message: `Rerail is disabled for workspace ${route.workspaceRoot}` };
  }
}


/** Relays one request; returns the line to answer with, or undefined when the server answered with no body. */
async function relay(endpoint: URL, request: string, fetchImpl: RerailMcpBridgeFetch): Promise<string | undefined> {
  const response = await fetchImpl(endpoint, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
    body: request,
    signal: AbortSignal.timeout(30_000),
  });
  const body = await response.text();
  if (response.ok) return body === "" ? undefined : body;
  const json = response.headers.get("content-type")?.includes("application/json") ?? false;
  if (json || isJsonRpcBody(body)) return body;
  throw new Error(`Rerail MCP bridge request failed (${response.status}): ${body}`);
}

async function relayRequest(
  destination: RerailMcpDestination,
  request: string,
  stdout: Writable,
  stderr: Writable,
  fetchImpl: RerailMcpBridgeFetch,
): Promise<void> {
  try {
    if (destination.kind === "error") throw new Error(destination.message);
    const body = await relay(destination.url, request, fetchImpl);
    if (body !== undefined) await writeLine(stdout, body);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    stderr.write(`rerail: mcp bridge: ${message}\n`);
    const response = errorResponses(parseRequestLine(request), message);
    if (response !== undefined) await writeLine(stdout, response);
  }
}

/**
 * Immutable host-side MCP bridge. Its process cwd selects a checkout instance
 * once, so a harness session can never be redirected by another worktree.
 * A failed request answers its own ids with a JSON-RPC error; only stdin EOF ends the bridge.
 */
export async function runRerailMcpBridge(options: RerailMcpBridgeOptions = {}): Promise<void> {
  const destination = resolveMcpDestination(await resolveRerailWorkspaceRoute(options.cwd ?? process.cwd()));
  const stdout = options.stdout ?? process.stdout;
  const stderr = options.stderr ?? process.stderr;
  const fetchImpl = options.fetch ?? fetch;
  const lines = createInterface({ input: options.input ?? process.stdin, crlfDelay: Infinity });
  for await (const request of lines) {
    if (request.trim() === "") continue;
    await relayRequest(destination, request, stdout, stderr, fetchImpl);
  }
}
