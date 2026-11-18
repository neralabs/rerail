import { Agent as HttpAgent, request as httpRequest, type ClientRequest, type IncomingMessage, type RequestOptions } from "node:http";
import { Readable } from "node:stream";

/** How a Rerail process sends one request to the daemon: `fetch`'s shape, over the transport the daemon serves. */
export type DaemonFetch = (endpoint: URL, init: RequestInit) => Promise<Response>;

/** Node's `http.request` or `https.request`, as the daemon's scheme needs. */
export type SendRequest = (url: URL, options: RequestOptions, onResponse: (response: IncomingMessage) => void) => ClientRequest;

/** Statuses whose response has no body, which `Response` refuses one for. */
const NULL_BODY_STATUSES: ReadonlySet<number> = new Set([101, 103, 204, 205, 304]);

/**
 * Methods sent once more, on a new connection, when the kept connection they went out on closed before any answer: the
 * daemon may have closed it as the request left. Only methods that read: a POST may have reached the daemon, which must
 * never take it twice.
 */
const SENT_AGAIN: ReadonlySet<string> = new Set(["GET", "HEAD"]);

/** One request as it leaves this process. */
interface Outgoing {
  readonly endpoint: URL;
  readonly method: string;
  readonly headers: Record<string, string>;
  readonly body: string | Uint8Array | undefined;
  readonly signal: AbortSignal | undefined;
}

/** A kept connection closed before the daemon answered the request sent on it ({@link SENT_AGAIN}). */
class KeptConnectionClosed extends Error {
  constructor(readonly original: Error) {
    super(original.message, { cause: original });
  }
}

function requestBody(body: RequestInit["body"]): string | Uint8Array | undefined {
  if (body === undefined || body === null || typeof body === "string" || body instanceof Uint8Array) return body ?? undefined;
  throw new TypeError("A request to a Rerail daemon carries a string or bytes body");
}

/** A Node response's headers as `fetch` gives them: each value of a header that repeats appended in turn. */
function fetchHeaders(incoming: Readonly<Record<string, string | string[] | undefined>>): Headers {
  const headers = new Headers();
  for (const [name, value] of Object.entries(incoming)) {
    for (const item of [value ?? []].flat()) headers.append(name, item);
  }
  return headers;
}

/**
 * `response` as `fetch` gives it. The body streams as the daemon sends it; one that fits in the stream's buffer is read
 * off the connection even when the caller never reads it, so the connection goes back to its agent and holds no
 * process open.
 */
function fetchResponse(method: string, response: IncomingMessage): Response {
  const status = response.statusCode ?? 502;
  const empty = method === "HEAD" || NULL_BODY_STATUSES.has(status);
  if (empty) response.resume();
  // Node's web stream and the DOM's `ReadableStream` type are the same object at run time; only their typings differ.
  const body = empty ? null : Readable.toWeb(response) as unknown as ReadableStream;
  return new Response(body, { status, statusText: response.statusMessage ?? "", headers: fetchHeaders(response.headers) });
}

/**
 * Why `request` failed, as `fetch` says it: the signal's own reason once it aborted, {@link KeptConnectionClosed} when a
 * kept connection closed before any answer, else Node's error.
 */
function requestFailure(error: Error, request: ClientRequest, signal: AbortSignal | undefined): unknown {
  if (signal?.aborted === true) return signal.reason;
  return request.reusedSocket ? new KeptConnectionClosed(error) : error;
}

/** Sends `outgoing` once through `agent`; the answer as a `Response`, its body still streaming. */
function sendOnce(send: SendRequest, agent: HttpAgent, outgoing: Outgoing): Promise<Response> {
  const { promise, resolve, reject } = Promise.withResolvers<Response>();
  const options: RequestOptions = { method: outgoing.method, headers: outgoing.headers, agent };
  if (outgoing.signal !== undefined) options.signal = outgoing.signal;
  const request = send(outgoing.endpoint, options, response => resolve(fetchResponse(outgoing.method, response)));
  request.once("error", error => reject(requestFailure(error, request, outgoing.signal)));
  request.end(outgoing.body);
  return promise;
}

/**
 * `fetch` over `agent`, for the request shapes Rerail's own clients send. Redirects are never followed, so the access
 * capability never goes to another address. A request that gives up (its signal aborts) ends its connection and nothing
 * is sent in its place.
 */
export function agentFetch(agent: HttpAgent, send: SendRequest): DaemonFetch {
  return async (endpoint, init) => {
    const outgoing: Outgoing = {
      endpoint,
      method: (init.method ?? "GET").toUpperCase(),
      headers: Object.fromEntries(new Headers(init.headers)),
      body: requestBody(init.body),
      signal: init.signal ?? undefined,
    };
    try {
      return await sendOnce(send, agent, outgoing);
    } catch (error) {
      if (!(error instanceof KeptConnectionClosed)) throw error;
      if (!SENT_AGAIN.has(outgoing.method)) throw error.original;
      return await sendOnce(send, agent, outgoing);
    }
  };
}

/**
 * How long a connection to an `http:` daemon stays open with no request on it: a second less when the daemon announces
 * a shorter keep-alive (`Keep-Alive: timeout=5` from Node's server, which Node's agent reads only when it has a timeout
 * of its own), so this process closes it before the daemon does.
 */
const KEPT_CONNECTION_MS = 4_000;

/**
 * The agent of every request this process sends to an `http:` daemon. It keeps one connection open between requests, so
 * a hook's start check and health probe go over one; Node unrefs a kept connection, so it holds no process open.
 */
const KEPT_CONNECTIONS = new HttpAgent({ keepAlive: true, timeout: KEPT_CONNECTION_MS });

/**
 * `fetch` to an `http:` daemon over Node's http client and this process's {@link KEPT_CONNECTIONS}. Not `fetch`: Node's
 * bundled fetch opens a new connection when a request gives up and sends nothing on it, so a busy daemon admits an
 * empty connection, one event loop turn each, for every read that waited out its limit.
 */
export const httpDaemonFetch: DaemonFetch = agentFetch(KEPT_CONNECTIONS, httpRequest);
