import { randomBytes } from "node:crypto";
import { createServer as createHttpServer } from "node:http";
import { createServer, type Server, type Socket } from "node:net";
import { join } from "node:path";
import { ENGINE_CHALLENGE_HEADER, ENGINE_PROOF_PATH, engineProof } from "../../../src/runtime/index.js";

/**
 * An engine endpoint for a test to record or listen on: `engine.sock` in `folder`, or on Windows a named pipe, the only
 * endpoint a Windows engine listens on (`isUsableEngineSocketPath`), since Windows refuses to listen on a path in a
 * folder (`listen EACCES`). The pipe is named as an engine names the fallback pipe it moves to, `rerail-<24 hex>-<hex>`,
 * so a daemon lookup reads it as an engine's. Pipes share one namespace, so each call names a new one.
 */
export function testEngineEndpoint(folder: string): string {
  if (process.platform !== "win32") return join(folder, "engine.sock");
  return `${String.raw`\\.\pipe\rerail-`}${randomBytes(12).toString("hex")}-${process.pid.toString(16)}`;
}

/**
 * Listens on `endpoint` as a live daemon's engine does to a lookup: given the `accessToken` it holds, it answers the
 * engine proof (`engineCapabilityProof`) as the engine does, and refuses anything else; without one it hangs up on
 * every connection. On Windows a lookup sends the capability daemon.json records to no daemon whose engine has not
 * proved it holds it. Closing drops open connections, so a close never waits on a lookup that gave up on its proof.
 */
export async function listenAsEngine(endpoint: string, accessToken?: string): Promise<() => Promise<void>> {
  const server: Server = accessToken === undefined
    ? createServer(socket => socket.destroy())
    : createHttpServer((request, response) => {
      request.resume();
      const challenge = request.headers[ENGINE_CHALLENGE_HEADER];
      if (request.method !== "POST" || request.url !== ENGINE_PROOF_PATH || typeof challenge !== "string") {
        response.writeHead(404).end();
        return;
      }
      response.setHeader("Content-Type", "application/json");
      response.end(JSON.stringify({ proof: engineProof(accessToken, challenge) }));
    });
  const sockets = new Set<Socket>();
  server.on("connection", (socket: Socket) => {
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
  });
  const listening = Promise.withResolvers<void>();
  server.once("error", listening.reject);
  server.listen(endpoint, () => listening.resolve());
  await listening.promise;
  return async () => {
    for (const socket of sockets) socket.destroy();
    const closed = Promise.withResolvers<void>();
    server.close(() => closed.resolve());
    await closed.promise;
  };
}

/**
 * Holds `endpoint` open, as a live daemon holds its engine pipe, until the returned function closes it: a daemon lookup
 * on Windows counts a recorded daemon whose pipe is gone as stale (`lookupDaemon`), and one whose engine does not prove
 * the capability daemon.json records as not its daemon, so pass the `accessToken` the state records. Elsewhere nothing
 * is held.
 */
export async function holdEngineEndpoint(endpoint: string, accessToken?: string): Promise<() => Promise<void>> {
  if (process.platform !== "win32") return async () => {};
  return await listenAsEngine(endpoint, accessToken);
}
