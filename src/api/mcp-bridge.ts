import { once } from "node:events";
import { realpath } from "node:fs/promises";
import { join } from "node:path";
import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";
import { DAEMON_STATE_FILE, daemonEndpoint, ownProcessAlive, probeDaemon, readDaemonState, type DaemonEndpoint, type DaemonState } from "../runtime/daemon-control.js";
import { CALLER_CWD_HEADER } from "../engine/catalog/repository-model.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";

export type RerailMcpBridgeFetch = (endpoint: URL, init: RequestInit) => Promise<Response>;

export interface RerailMcpBridgeOptions {
  /** The folder the coding agent runs in; defaults to the process cwd. */
  cwd?: string;
  input?: Readable;
  stdout?: Writable;
  stderr?: Writable;
  fetch?: RerailMcpBridgeFetch;
  /** The Rerail home whose daemon state names the daemon; defaults to `defaultRerailHome()`. */
  home?: string;
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

const DAEMON_NOT_RUNNING = "Rerail daemon is not running; start it with rerail daemon start";
/** The daemon is up but does not accept the capability its state file records: what to do, not "not running". */
const DAEMON_REFUSED = "Rerail daemon refused the access capability in daemon.json (401); run rerail daemon stop, then rerail daemon start";

function sameDaemon(left: DaemonState, right: DaemonState): boolean {
  return left.pid === right.pid && left.url === right.url && left.startedAt === right.startedAt;
}

/**
 * The home's daemon as its state file records it, read again for every request: the daemon takes a free port
 * when the default one is held, and a restart or update may move it while the bridge runs. A state file left
 * behind by a SIGKILL or a reboot can name a pid since reused, with nothing or another home's daemon on its
 * port, so each state file is checked once against `/healthz` before any request goes to its URL; after that a
 * request costs one file read until a request fails to connect.
 */
class DaemonLocator {
  #confirmed: DaemonState | undefined;

  constructor(private readonly home: string, private readonly fetchImpl: RerailMcpBridgeFetch) {}

  /** The `/mcp` endpoint of the home's live daemon and the headers that admit a request to it. */
  async endpoint(): Promise<DaemonEndpoint> {
    const state = await readDaemonState(join(this.home, DAEMON_STATE_FILE));
    // Another user's process cannot be this user's daemon; its URL is never probed with the capability.
    if (state === undefined || !ownProcessAlive(state.pid)) throw new Error(DAEMON_NOT_RUNNING);
    if (this.#confirmed === undefined || !sameDaemon(this.#confirmed, state)) {
      this.#confirmed = undefined;
      const probe = await probeDaemon(state, this.fetchImpl);
      if (probe.status === "refused") throw new Error(DAEMON_REFUSED);
      if (probe.status !== "ok" || probe.health.pid !== state.pid) throw new Error(DAEMON_NOT_RUNNING);
      this.#confirmed = state;
    }
    return daemonEndpoint(state, "/mcp");
  }

  /** After a request failed to connect: checks the home's daemon again, and throws when none answers. */
  async recheck(): Promise<void> {
    this.#confirmed = undefined;
    await this.endpoint();
  }
}

/** Relays one request; returns the line to answer with, or undefined when the server answered with no body. */
async function relay(
  request: string,
  daemon: DaemonLocator,
  callerHeaders: Readonly<Record<string, string>>,
  fetchImpl: RerailMcpBridgeFetch,
): Promise<string | undefined> {
  const endpoint = await daemon.endpoint();
  let response: Response;
  try {
    response = await fetchImpl(endpoint.url, {
      method: "POST",
      // The capability last: nothing the bridge was started with names another.
      headers: { "content-type": "application/json", accept: "application/json, text/event-stream", ...callerHeaders, ...endpoint.headers },
      body: request,
      signal: AbortSignal.timeout(30_000),
    });
  } catch (error) {
    await daemon.recheck();
    throw error;
  }
  const body = await response.text();
  if (response.ok) return body === "" ? undefined : body;
  // A refusal is not an MCP answer: the client gets a JSON-RPC error that says what to do, not the daemon's body.
  if (response.status === 401) throw new Error(DAEMON_REFUSED);
  const json = response.headers.get("content-type")?.includes("application/json") ?? false;
  if (json || isJsonRpcBody(body)) return body;
  throw new Error(`Rerail MCP bridge request failed (${response.status}): ${body}`);
}

async function relayRequest(
  request: string,
  daemon: DaemonLocator,
  callerHeaders: Readonly<Record<string, string>>,
  stdout: Writable,
  stderr: Writable,
  fetchImpl: RerailMcpBridgeFetch,
): Promise<void> {
  try {
    const body = await relay(request, daemon, callerHeaders, fetchImpl);
    if (body !== undefined) await writeLine(stdout, body);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    stderr.write(`rerail: mcp bridge: ${message}\n`);
    const response = errorResponses(parseRequestLine(request), message);
    if (response !== undefined) await writeLine(stdout, response);
  }
}

/**
 * Stdio MCP bridge to the machine's Rerail daemon, found for each request through the Rerail home's daemon state.
 * It names its caller: every relay carries the real path of the folder the bridge started in, URI-encoded, in
 * `CALLER_CWD_HEADER`, so a tool called without a target acts on the agent's own checkout. The JSON-RPC body is
 * never rewritten.
 * A failed request answers its own ids with a JSON-RPC error; only stdin EOF ends the bridge.
 */
export async function runRerailMcpBridge(options: RerailMcpBridgeOptions = {}): Promise<void> {
  const callerCwd = await realpath(options.cwd ?? process.cwd()).catch(() => undefined);
  const callerHeaders: Record<string, string> = callerCwd === undefined ? {} : { [CALLER_CWD_HEADER]: encodeURIComponent(callerCwd) };
  const stdout = options.stdout ?? process.stdout;
  const stderr = options.stderr ?? process.stderr;
  const fetchImpl = options.fetch ?? fetch;
  const daemon = new DaemonLocator(options.home ?? defaultRerailHome(), fetchImpl);
  const lines = createInterface({ input: options.input ?? process.stdin, crlfDelay: Infinity });
  for await (const request of lines) {
    if (request.trim() === "") continue;
    await relayRequest(request, daemon, callerHeaders, stdout, stderr, fetchImpl);
  }
}
