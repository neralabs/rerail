import { spawn } from "node:child_process";
import { once } from "node:events";
import { createServer as createHttpServer, type IncomingMessage, type Server as HttpServer, type ServerResponse } from "node:http";
import { connect, createServer, type AddressInfo, type Server, type Socket } from "node:net";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { daemonFetch } from "../../../src/runtime/index.js";

const transportModule = pathToFileURL(resolve(import.meta.dirname, "..", "..", "..", "src", "runtime", "daemon-control", "transport.ts")).href;
const ACCESS = { "x-rerail-access": "capability" };

/** A connection the daemon admitted: the bytes it read, and when it closed. */
interface Admitted {
  readonly socket: Socket;
  text: string;
  answered: number;
  readonly closed: Promise<void>;
}

/** A daemon port that admits connections and hands each one's bytes to `onData`. */
class FakeDaemon {
  readonly admitted: Admitted[] = [];
  readonly server: Server;
  #waiting: Array<() => void> = [];

  constructor(onData: (connection: Admitted, chunk: Buffer) => void = () => undefined) {
    this.server = createServer(socket => {
      const connection: Admitted = { socket, text: "", answered: 0, closed: once(socket, "close").then(() => undefined) };
      this.admitted.push(connection);
      socket.on("error", () => undefined);
      socket.on("data", (chunk: Buffer) => {
        connection.text += chunk.toString("latin1");
        onData(connection, chunk);
      });
      for (const wake of this.#waiting.splice(0)) wake();
    });
  }

  async listen(): Promise<string> {
    this.server.listen(0, "127.0.0.1");
    await once(this.server, "listening");
    return `http://127.0.0.1:${(this.server.address() as AddressInfo).port}/`;
  }

  /** Resolves once the daemon has admitted `count` connections. */
  async admittedCount(count: number): Promise<void> {
    while (this.admitted.length < count) await new Promise<void>(wake => this.#waiting.push(wake));
  }

  /**
   * How many connections the daemon admitted before one opened now: the kernel queues connections in the order they
   * were made, so one this process made earlier, an empty one too, is admitted first.
   */
  async admittedBeforeNow(): Promise<number> {
    const marker = connect((this.server.address() as AddressInfo).port, "127.0.0.1");
    marker.on("error", () => undefined);
    const local = await once(marker, "connect").then(() => marker.localPort);
    while (!this.admitted.some(connection => connection.socket.remotePort === local)) await this.admittedCount(this.admitted.length + 1);
    marker.destroy();
    return this.admitted.findIndex(connection => connection.socket.remotePort === local);
  }

  async close(): Promise<void> {
    for (const { socket } of this.admitted) socket.destroy();
    this.server.close();
    await once(this.server, "close");
  }
}

/** Requests ended by a blank line, as each GET here is: how many the connection carried. */
const requests = (text: string): number => text.split("\r\n\r\n").length - 1;
const answer = (body: string): string => `HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\nconnection: keep-alive\r\n\r\n${body}`;

let servers: Array<{ close(): Promise<void> }> = [];
afterEach(async () => {
  await Promise.all(servers.map(async server => await server.close()));
  servers = [];
});

async function fakeDaemon(onData?: (connection: Admitted, chunk: Buffer) => void): Promise<{ daemon: FakeDaemon; url: string }> {
  const daemon = new FakeDaemon(onData);
  servers.push(daemon);
  return { daemon, url: await daemon.listen() };
}

async function httpDaemon(handle: (request: IncomingMessage, response: ServerResponse) => void): Promise<{ server: HttpServer; url: string }> {
  const server = createHttpServer(handle);
  servers.push({ close: async () => { server.closeAllConnections(); server.close(); await once(server, "close"); } });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return { server, url: `http://127.0.0.1:${(server.address() as AddressInfo).port}/` };
}

describe("daemonFetch over http", () => {
  it("leaves the daemon no connection that carries nothing when a read gives up", async () => {
    const { daemon, url } = await fakeDaemon();
    const giveUp = new AbortController();
    const reading = daemonFetch({ url })(new URL("/api/presentation", url), { headers: ACCESS, signal: giveUp.signal });
    await daemon.admittedCount(1);
    while (requests(daemon.admitted[0]!.text) === 0) await once(daemon.admitted[0]!.socket, "data");
    giveUp.abort(new DOMException("The operation was aborted due to timeout", "TimeoutError"));

    await expect(reading).rejects.toMatchObject({ name: "TimeoutError" });
    await daemon.admitted[0]!.closed;
    // Node's fetch opened a second connection here, after the request had given up, and sent nothing on it.
    expect(await daemon.admittedBeforeNow()).toBe(1);
  });

  it("sends a session's reads over one kept connection", async () => {
    let connections = 0;
    const { server, url } = await httpDaemon((request, response) => response.end(JSON.stringify({ path: request.url })));
    server.on("connection", () => { connections += 1; });
    const send = daemonFetch({ url });

    for (const path of ["/api/presentation", "/healthz"]) {
      const response = await send(new URL(path, url), { headers: ACCESS });
      expect(await response.json()).toEqual({ path });
    }
    expect(connections).toBe(1);
  });

  it("sends a GET once more on a new connection when the daemon closed the kept one as it arrived, and never a POST", async () => {
    // The daemon answers the first request on a connection and closes the connection when a second one reaches it.
    const onData = (connection: Admitted): void => {
      if (requests(connection.text) > 1) connection.socket.destroy();
      else if (requests(connection.text) === 1 && connection.answered === 0) {
        connection.answered = 1;
        connection.socket.write(answer("{}"));
      }
    };
    const { daemon, url } = await fakeDaemon(onData);
    const send = daemonFetch({ url });
    const status = async (target: string, init: RequestInit): Promise<number> => {
      const response = await send(new URL(target), init);
      await response.text();
      return response.status;
    };
    expect(await status(new URL("/healthz", url).href, { headers: ACCESS })).toBe(200);
    expect(await status(new URL("/healthz", url).href, { headers: ACCESS })).toBe(200);
    expect(daemon.admitted.map(connection => requests(connection.text))).toEqual([2, 1]);

    const posting = await fakeDaemon(onData);
    const post = { method: "POST", headers: ACCESS, body: "{}" };
    expect(await status(new URL("/native/shutdown", posting.url).href, post)).toBe(200);
    await expect(status(new URL("/native/shutdown", posting.url).href, post)).rejects.toThrow();
    expect(posting.daemon.admitted.map(connection => requests(connection.text))).toEqual([2]);
  });

  it("sends the capability and follows no redirect, so the capability reaches no other address", async () => {
    const elsewhere: Array<string | undefined> = [];
    const other = await httpDaemon((request, response) => {
      elsewhere.push(request.headers["x-rerail-access"] as string | undefined);
      response.end("{}");
    });
    const seen: Array<string | undefined> = [];
    const { url } = await httpDaemon((request, response) => {
      seen.push(request.headers["x-rerail-access"] as string | undefined);
      response.writeHead(302, { location: new URL("/healthz", other.url).href }).end();
    });

    const response = await daemonFetch({ url })(new URL("/healthz", url), { headers: ACCESS });
    expect({ status: response.status, location: response.headers.get("location") }).toEqual({ status: 302, location: new URL("/healthz", other.url).href });
    expect(seen).toEqual(["capability"]);
    expect(elsewhere).toEqual([]);
  });

  it("carries bodies larger than a socket's buffer both ways, and answers with no body where fetch gives none", async () => {
    const big = Buffer.alloc(4 * 1024 * 1024, "r");
    const { url } = await httpDaemon((request, response) => {
      const chunks: Buffer[] = [];
      request.on("data", (chunk: Buffer) => chunks.push(chunk));
      request.on("end", () => {
        if (request.url === "/none") return response.writeHead(204).end();
        response.end(request.method === "POST" ? String(Buffer.concat(chunks).length) : big);
      });
    });
    const send = daemonFetch({ url });

    expect(await (await send(new URL("/native/session", url), { method: "POST", headers: ACCESS, body: big.toString() })).text()).toBe(String(big.length));
    expect((await (await send(new URL("/page", url), { headers: ACCESS })).arrayBuffer()).byteLength).toBe(big.length);
    expect((await send(new URL("/none", url), { headers: ACCESS })).body).toBeNull();
    const head = await send(new URL("/page", url), { method: "HEAD", headers: ACCESS });
    expect({ status: head.status, body: head.body }).toEqual({ status: 200, body: null });
  });

  it.each(["read", "unread"])("lets a process end at once after a request, its answer %s, while the daemon keeps the connection", async (read) => {
    const { server, url } = await httpDaemon((_request, response) => response.end(JSON.stringify({ status: "ok" })));
    // The daemon never closes an idle connection: a process that waited on its kept connection would never end.
    server.keepAliveTimeout = 0;
    const child = spawn(process.execPath, [
      "--import", "tsx", "--input-type=module", "-e",
      `const { httpDaemonFetch } = await import(${JSON.stringify(transportModule)});
       const response = await httpDaemonFetch(new URL("/healthz", ${JSON.stringify(url)}), { headers: ${JSON.stringify(ACCESS)} });
       if (${JSON.stringify(read)} === "read") await response.json();
       process.stdout.write(String(response.status));`,
    ], { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    child.stdout.on("data", (chunk: Buffer) => { out += chunk.toString(); });
    child.stderr.on("data", (chunk: Buffer) => { out += chunk.toString(); });

    const [code] = await once(child, "exit") as [number | null];
    expect({ code, out }).toEqual({ code: 0, out: "200" });
  }, 20_000);
});
