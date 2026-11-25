import { randomBytes } from "node:crypto";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { createServer as createHttpServer } from "node:http";
import { createServer, type Server, type Socket } from "node:net";
import { join } from "node:path";
import { defaultRerailDatabasePath } from "../../../src/engine/index.js";
import { DAEMON_STATE_FILE, ENGINE_CHALLENGE_HEADER, ENGINE_PROOF_PATH, engineProof, startEngineService } from "../../../src/runtime/index.js";

/**
 * Starts the engine service at `options.socketPath` as the daemon starts its own, which in the product is the only
 * process that starts one: holding a capability, and recorded in daemon.json in `home` with this process's pid and a
 * start stamped after the process began, as the daemon stamps its own. A hook takes that capability from daemon.json,
 * and on Windows sends nothing to the pipe `RERAIL_ENGINE_SOCKET` names until Windows names the daemon daemon.json
 * records as the pipe's holder, or its engine proves it holds that capability. Closing the service removes daemon.json.
 */
export async function startEngineAsDaemon(home: string, options: Parameters<typeof startEngineService>[0]): Promise<{ close(): Promise<void> }> {
  const accessToken = randomBytes(16).toString("hex");
  const service = await startEngineService({ ...options, accessToken });
  const stateFile = await recordAsDaemon(home, { socketPath: options.socketPath, databasePath: options.databasePath, accessToken });
  return {
    close: async () => {
      await rm(stateFile, { force: true });
      await service.close();
    },
  };
}

/**
 * Records this process in daemon.json in `home` as the daemon whose engine listens on `engine.socketPath` holding
 * `engine.accessToken`, with a start stamped after the process began, as the daemon stamps its own: on Windows a hook
 * then finds Windows naming the recorded daemon as the pipe's holder and sends to it. Returns daemon.json's path.
 */
export async function recordAsDaemon(home: string, engine: { socketPath: string; databasePath?: string | undefined; accessToken: string }): Promise<string> {
  const stateFile = join(home, DAEMON_STATE_FILE);
  await mkdir(home, { recursive: true });
  await writeFile(stateFile, JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 1, url: "http://127.0.0.1:1/", version: "test",
    databasePath: engine.databasePath ?? defaultRerailDatabasePath(), kernelSocketPath: engine.socketPath, ui: "bundled", startedAt: Date.now(),
    packageRoot: home, accessToken: engine.accessToken }), { mode: 0o600 });
  return stateFile;
}

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
