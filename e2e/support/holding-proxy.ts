/**
 * A reverse proxy for the browser tests that holds the daemon's change stream (`/api/changes`) whole, as a proxy that
 * buffers responses does: it hands the page nothing of a stream until the daemon's side of it ends, then everything it
 * held, also when the daemon's connection closed midway (a stop or a restart), as such a proxy passes on a truncated
 * response. Every other request passes through as it comes; one the daemon cannot take (it is down) answers 502.
 */
import { createServer, request, type IncomingHttpHeaders, type IncomingMessage, type ServerResponse } from "node:http";

export interface HoldingProxy {
  /** `http://127.0.0.1:<port>`, the origin the page opens. */
  readonly origin: string;
  close(): Promise<void>;
}

/** Headers of one connection, which the proxy's own connection to the browser sets for itself. */
const HOP_BY_HOP = new Set(["connection", "keep-alive", "transfer-encoding"]);

function endToEnd(headers: IncomingHttpHeaders): IncomingHttpHeaders {
  return Object.fromEntries(Object.entries(headers).filter(([name]) => !HOP_BY_HOP.has(name)));
}

/** Hands on the stream `answer` once the daemon's side of it ends, however it ends. */
function holdUntilEnd(answer: IncomingMessage, response: ServerResponse): void {
  const held: Buffer[] = [];
  const handOn = (): void => {
    if (response.headersSent) return;
    response.writeHead(answer.statusCode ?? 502, endToEnd(answer.headers));
    response.end(Buffer.concat(held));
  };
  answer.on("data", (chunk: Buffer) => held.push(chunk));
  answer.on("end", handOn);
  answer.on("close", handOn);
  answer.on("error", handOn);
}

/** Hands on any other answer as it comes. */
function passOn(answer: IncomingMessage, response: ServerResponse): void {
  response.writeHead(answer.statusCode ?? 502, endToEnd(answer.headers));
  answer.pipe(response);
}

/** The daemon could not be reached, or its connection broke midway. */
function unreachable(response: ServerResponse): void {
  if (response.headersSent) response.destroy();
  else response.writeHead(502).end();
}

function forward(port: number, incoming: IncomingMessage, response: ServerResponse): void {
  const hand = incoming.url?.startsWith("/api/changes") === true ? holdUntilEnd : passOn;
  const headers = { ...incoming.headers, host: `127.0.0.1:${port}` };
  const outgoing = request({ host: "127.0.0.1", port, method: incoming.method, path: incoming.url, headers }, answer => hand(answer, response));
  outgoing.on("error", () => unreachable(response));
  // The page gave up on the request (a stream it stopped following): the daemon's side ends with it.
  response.on("close", () => outgoing.destroy());
  incoming.pipe(outgoing);
}

/** Starts the proxy in front of the daemon listening on `port` of 127.0.0.1; the daemon may stop and start there. */
export async function startHoldingProxy(port: number): Promise<HoldingProxy> {
  const server = createServer((incoming, response) => forward(port, incoming, response));
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("the proxy has no port");
  return {
    origin: `http://127.0.0.1:${address.port}`,
    close: async () => {
      server.closeAllConnections();
      await new Promise<void>(resolve => server.close(() => resolve()));
    },
  };
}
