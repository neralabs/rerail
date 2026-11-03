import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

export interface SinkRequest {
  readonly authorization: string | undefined;
  readonly contentType: string | undefined;
  readonly bytes: number;
  readonly header: Record<string, unknown>;
  readonly records: Array<Record<string, unknown> & { type: string; id: string }>;
}

export interface FakeSink {
  readonly url: string;
  /** Every request, accepted or not, in arrival order. */
  readonly requests: SinkRequest[];
  /** The records of accepted batches, each batch id stored once (C1). */
  readonly stored: Map<string, SinkRequest>;
  /** The status the next requests get: 200 by default. */
  respondWith(status: number): void;
  /** Answers 400 to any batch for which `refuse` is true (a record the remote cannot take). */
  refuseWhen(refuse: ((records: SinkRequest["records"]) => boolean) | undefined): void;
  /** Runs once, while the next request is in flight, before it is answered. */
  duringNextRequest(action: () => void): void;
  close(): Promise<void>;
}

/** A local stand-in for a remote: `POST /v1/batches` with a bearer token, NDJSON body, header line first (C1). */
export async function startFakeSink(token = "test-token"): Promise<FakeSink> {
  const requests: SinkRequest[] = [];
  const stored = new Map<string, SinkRequest>();
  let status = 200;
  let refuse: ((records: SinkRequest["records"]) => boolean) | undefined;
  let during: (() => void) | undefined;
  const server: Server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += String(chunk);
    if (request.method !== "POST" || request.url !== "/v1/batches") {
      response.writeHead(404).end();
      return;
    }
    const lines = body.split("\n").filter(line => line.length > 0).map(line => JSON.parse(line) as Record<string, unknown> & { type: string; id: string });
    const [header, ...records] = lines;
    const received: SinkRequest = {
      authorization: request.headers.authorization,
      contentType: request.headers["content-type"],
      bytes: Buffer.byteLength(body),
      header: header ?? {},
      records,
    };
    requests.push(received);
    const action = during;
    during = undefined;
    action?.();
    if (request.headers.authorization !== `Bearer ${token}`) {
      response.writeHead(401, { "content-type": "application/json" }).end(JSON.stringify({ error: "unauthorized" }));
      return;
    }
    if (refuse?.(records) === true) {
      response.writeHead(400).end("bad record");
      return;
    }
    if (status !== 200) {
      response.writeHead(status).end("down");
      return;
    }
    const batchId = String(received.header.batchId);
    if (!stored.has(batchId)) stored.set(batchId, received);
    response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ accepted: batchId }));
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return {
    url: `http://127.0.0.1:${port}`,
    requests,
    stored,
    respondWith(next) { status = next; },
    refuseWhen(next) { refuse = next; },
    duringNextRequest(action) { during = action; },
    close: () => new Promise<void>(resolve => server.close(() => resolve())),
  };
}

/** Every stored record, batch after batch. */
export function storedRecords(sink: FakeSink): SinkRequest["records"] {
  return [...sink.stored.values()].flatMap(request => request.records);
}
