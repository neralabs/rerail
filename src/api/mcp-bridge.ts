import { once } from "node:events";
import { join } from "node:path";
import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";
import {
  DAEMON_STATE_FILE,
  daemonEndpoint,
  probeDaemon,
  readDaemonState,
  recordedDaemonStatus,
  type DaemonEndpoint,
  type DaemonMachine,
  type DaemonState,
} from "../runtime/daemon-control.js";
import { checkoutRealpath } from "../engine/catalog/checkout-control.js";
import { CALLER_CWD_HEADER } from "../engine/catalog/repository-model.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";

export type RerailMcpBridgeFetch = (endpoint: URL, init: RequestInit) => Promise<Response>;

export interface RerailMcpBridgeOptions {
  /** The folder the coding agent runs in; defaults to the process cwd. */
  cwd?: string;
  input?: Readable;
  stdout?: Writable;
  stderr?: Writable;
  /** Sends every request (the health probe included) in place of the daemon's own transport (`daemonFetch`). */
  fetch?: RerailMcpBridgeFetch;
  /** The Rerail home whose daemon state names the daemon; defaults to `defaultRerailHome()`. */
  home?: string;
  /**
   * The machine whose processes, pipes and engine proof say whether the recorded daemon may be sent the capability;
   * this one unless a test passes its own, so each platform's check runs on every OS.
   */
  machine?: DaemonMachine;
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

/**
 * How long the daemon has to answer each part of a relayed call. The checks before it may be sent the capability (on
 * Windows its engine's proof, then the health check of a newly recorded daemon) have it from the moment the bridge
 * takes the call; the call itself has it from its send, as it always had. A daemon whose one event loop is in an audit
 * or a long garbage collection is waited for and answers, and one that never answers fails the call here. The coding
 * agent's own limit on an MCP call is far longer.
 */
const CALL_TIMEOUT_MS = 30_000;
/** The daemon runs, and took the call, but gave no answer by the call's bound: busy or stuck, never "not running". */
const DAEMON_NOT_ANSWERING = `Rerail daemon is running but did not answer within ${CALL_TIMEOUT_MS / 1_000} s; try again, and if it stays silent run rerail daemon stop, then rerail daemon start`;

/**
 * The least time a wait of the call is given, the 2 s each check had before the call had a bound: a check that starts
 * late, such as the health check after a proof that passed only once the bound had, still gets that long.
 */
const WAIT_MIN_MS = 2_000;

/**
 * One relayed call's bound, {@link CALL_TIMEOUT_MS} for each of its two parts. The checks have it from the moment the
 * bridge takes the call; from {@link CallBound.sent} the daemon's answer has it whole, on a signal of its own, so no
 * check, however long it waited, shortens the time a tool has to answer. Each wait ends on its answer or at the end of
 * its part, and gets at least {@link WAIT_MIN_MS}. The bound reads the monotonic clock its timer runs on, so a step of
 * the wall clock moves neither, and holds one timer at a time, ended with the call, so an idle bridge holds none.
 */
class CallBound {
  /** When the part under way ends, on the monotonic clock: first the checks' end, then the answer's. */
  #ends = performance.now() + CALL_TIMEOUT_MS;
  /** Aborts at `#ends`: made when a wait first carries it, and anew for each part, since an abort is for good. */
  #abort: AbortController | undefined;
  #timer: ReturnType<typeof setTimeout> | undefined;

  /**
   * The signal a wait that starts now carries: it aborts at the end of the part under way, with the call's answer as
   * its reason, and no sooner than {@link WAIT_MIN_MS} from now.
   */
  get signal(): AbortSignal {
    this.#start();
    this.#abort ??= this.#armed();
    return this.#abort.signal;
  }

  /** Whether the last wait that carried {@link CallBound.signal} reached the end of its part unanswered. */
  get passed(): boolean {
    return this.#abort?.signal.aborted === true;
  }

  /**
   * How long a wait that starts now and takes a duration (the engine's proof) may take, in ms, as long as
   * {@link CallBound.signal} would give it; never 0, which a socket's idle timeout reads as no limit at all.
   */
  left(): number {
    this.#start();
    return this.#ends - performance.now();
  }

  /** The call goes to the daemon now: its answer has the whole bound from here, whatever the checks took. */
  sent(): void {
    this.#part(CALL_TIMEOUT_MS);
  }

  /** Ends the bound's timer once the call has its answer. */
  end(): void {
    clearTimeout(this.#timer);
  }

  /** A wait starts now: when its part has less than {@link WAIT_MIN_MS} left, a part of that length starts for it. */
  #start(): void {
    if (this.#ends - performance.now() < WAIT_MIN_MS) this.#part(WAIT_MIN_MS);
  }

  /** Starts a part that ends `ms` from now; the last part's timer and signal go with it. */
  #part(ms: number): void {
    clearTimeout(this.#timer);
    this.#abort = undefined;
    this.#ends = performance.now() + ms;
  }

  /** A signal's controller that aborts at the end of the part under way. */
  #armed(): AbortController {
    const abort = new AbortController();
    this.#timer = setTimeout(() => abort.abort(new Error(DAEMON_NOT_ANSWERING)), this.#ends - performance.now());
    return abort;
  }
}

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

  constructor(
    private readonly home: string,
    private readonly fetchImpl: RerailMcpBridgeFetch | undefined,
    private readonly machine: DaemonMachine | undefined,
  ) {}

  /** The `/mcp` endpoint of the home's live daemon and the headers that admit a request to it, found within `bound`. */
  async endpoint(bound: CallBound): Promise<DaemonEndpoint> {
    const state = await this.#recorded(bound);
    if (this.#confirmed === undefined || !sameDaemon(this.#confirmed, state)) await this.#confirm(state, bound);
    return daemonEndpoint(state, "/mcp");
  }

  /**
   * The daemon the home's state file records, once it may be sent the capability. A pid that cannot be the recorded
   * daemon (`recordedDaemonStatus`) never has its URL probed with the capability; one whose engine is still silent to
   * the proof at the bound runs and is busy, so it is never called not running.
   */
  async #recorded(bound: CallBound): Promise<DaemonState> {
    const state = await readDaemonState(join(this.home, DAEMON_STATE_FILE), this.machine?.platform);
    if (state === undefined) throw new Error(DAEMON_NOT_RUNNING);
    const recorded = await recordedDaemonStatus(state, this.machine, bound.left());
    if (recorded === "silent") throw new Error(DAEMON_NOT_ANSWERING);
    if (recorded === "gone") throw new Error(DAEMON_NOT_RUNNING);
    return state;
  }

  /**
   * Probes the daemon `state` records within `bound` and keeps it as the confirmed one; throws when it refuses, another
   * pid answers, nothing listens, or it is still silent at the bound.
   */
  async #confirm(state: DaemonState, bound: CallBound): Promise<void> {
    this.#confirmed = undefined;
    const probe = await probeDaemon(state, { fetchImpl: this.fetchImpl, signal: bound.signal });
    if (probe.status === "refused") throw new Error(DAEMON_REFUSED);
    // Only the bound cuts a probe whose daemon took it and stays silent; a refused connection fails before it.
    if (probe.status === "unreachable" && bound.passed) throw new Error(DAEMON_NOT_ANSWERING);
    if (probe.status !== "ok" || probe.health.pid !== state.pid) throw new Error(DAEMON_NOT_RUNNING);
    this.#confirmed = state;
  }

  /** After a request failed to connect: checks the home's daemon again within `bound`, and throws when none answers. */
  async recheck(bound: CallBound): Promise<void> {
    this.#confirmed = undefined;
    await this.endpoint(bound);
  }
}

/**
 * Sends `request` to the home's daemon once its checks pass within `bound`, then gives the daemon the bound whole to
 * answer; a failure to connect has the home's daemon checked again within that time.
 */
async function post(
  request: string,
  daemon: DaemonLocator,
  callerHeaders: Readonly<Record<string, string>>,
  fetchImpl: RerailMcpBridgeFetch | undefined,
  bound: CallBound,
): Promise<Response> {
  const endpoint = await daemon.endpoint(bound);
  bound.sent();
  try {
    return await (fetchImpl ?? endpoint.fetch)(endpoint.url, {
      method: "POST",
      // The capability last: nothing the bridge was started with names another.
      headers: { "content-type": "application/json", accept: "application/json, text/event-stream", ...callerHeaders, ...endpoint.headers },
      body: request,
      signal: bound.signal,
    });
  } catch (error) {
    // A daemon silent until the bound took the request and runs; any other failure may be one that stopped or moved.
    if (bound.passed) throw new Error(DAEMON_NOT_ANSWERING);
    await daemon.recheck(bound);
    throw error;
  }
}

/** The line the daemon's `response` with `body` answers the call with; undefined when the server sent no body. */
function answerLine(response: Response, body: string): string | undefined {
  if (response.ok) return body === "" ? undefined : body;
  // A refusal is not an MCP answer: the client gets a JSON-RPC error that says what to do, not the daemon's body.
  if (response.status === 401) throw new Error(DAEMON_REFUSED);
  const json = response.headers.get("content-type")?.includes("application/json") ?? false;
  if (json || isJsonRpcBody(body)) return body;
  throw new Error(`Rerail MCP bridge request failed (${response.status}): ${body}`);
}

/** Relays one request; returns the line to answer with, or undefined when the server answered with no body. */
async function relay(
  request: string,
  daemon: DaemonLocator,
  callerHeaders: Readonly<Record<string, string>>,
  fetchImpl: RerailMcpBridgeFetch | undefined,
  bound: CallBound,
): Promise<string | undefined> {
  const response = await post(request, daemon, callerHeaders, fetchImpl, bound);
  let body: string;
  try {
    body = await response.text();
  } catch (error) {
    // An answer cut at the bound is a daemon that runs and is slow; each transport words the cut its own way.
    throw bound.passed ? new Error(DAEMON_NOT_ANSWERING) : error;
  }
  return answerLine(response, body);
}

async function relayRequest(
  request: string,
  daemon: DaemonLocator,
  callerHeaders: Readonly<Record<string, string>>,
  stdout: Writable,
  stderr: Writable,
  fetchImpl: RerailMcpBridgeFetch | undefined,
): Promise<void> {
  const bound = new CallBound();
  try {
    const body = await relay(request, daemon, callerHeaders, fetchImpl, bound);
    if (body !== undefined) await writeLine(stdout, body);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    stderr.write(`rerail: mcp bridge: ${message}\n`);
    const response = errorResponses(parseRequestLine(request), message);
    if (response !== undefined) await writeLine(stdout, response);
  } finally {
    bound.end();
  }
}

/** The folder the bridge started in, spelt as checkouts are (`checkoutRealpath`); undefined when it is gone. */
function callerFolder(cwd: string): string | undefined {
  try {
    return checkoutRealpath(cwd);
  } catch {
    return undefined;
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
  const callerCwd = callerFolder(options.cwd ?? process.cwd());
  const callerHeaders: Record<string, string> = callerCwd === undefined ? {} : { [CALLER_CWD_HEADER]: encodeURIComponent(callerCwd) };
  const stdout = options.stdout ?? process.stdout;
  const stderr = options.stderr ?? process.stderr;
  const fetchImpl = options.fetch;
  const daemon = new DaemonLocator(options.home ?? defaultRerailHome(), fetchImpl, options.machine);
  const lines = createInterface({ input: options.input ?? process.stdin, crlfDelay: Infinity });
  for await (const request of lines) {
    if (request.trim() === "") continue;
    await relayRequest(request, daemon, callerHeaders, stdout, stderr, fetchImpl);
  }
}
