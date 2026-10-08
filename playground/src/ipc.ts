import { EventEmitter } from "node:events";
import { createServer, createConnection, type Socket } from "node:net";
import { randomBytes } from "node:crypto";
import { chmod, rm } from "node:fs/promises";

const MAX_FRAME = 4 * 1_048_576;
function receive(socket: Socket, message: (value: unknown) => void): void {
  let pending = Buffer.alloc(0);
  socket.on("data", (chunk: Buffer) => {
    pending = Buffer.concat([pending, chunk]);
    while (pending.length) {
      const end = pending.indexOf(10);
      if (end < 0) {
        if (pending.length > MAX_FRAME) socket.destroy(new Error("Worker IPC frame exceeded limit"));
        return;
      }
      if (end > MAX_FRAME) { socket.destroy(new Error("Worker IPC frame exceeded limit")); return; }
      const frame = pending.subarray(0, end);
      pending = pending.subarray(end + 1);
      try { message(JSON.parse(frame.toString("utf8"))); }
      catch { socket.destroy(new Error("Invalid worker IPC frame")); return; }
    }
  });
}
function send(socket: Socket | undefined, value: unknown): void {
  if (!socket || socket.destroyed) throw new Error("Worker control channel is disconnected");
  const frame = JSON.stringify(value) + "\n";
  if (Buffer.byteLength(frame) > MAX_FRAME || socket.writableLength > MAX_FRAME)
    throw new Error("Worker control channel exceeded output limit");
  socket.write(frame);
}

export interface WorkerChannel {
  env: { RERAIL_PLAYGROUND_IPC: string; RERAIL_PLAYGROUND_TOKEN: string };
  send(value: unknown): void;
  close(): Promise<void>;
}
/** A private, single-owner capability channel; workers never choose the command workspace. */
export async function createWorkerChannel(path: string, message: (value: unknown) => void): Promise<WorkerChannel> {
  let closing: Promise<void> | undefined;
  const token = randomBytes(32).toString("hex");
  let owner: Socket | undefined;
  const clients = new Set<Socket>();
  const server = createServer(socket => {
    clients.add(socket);
    socket.on("error", () => socket.destroy());
    socket.on("close", () => { clients.delete(socket); if (owner === socket) owner = undefined; });
    let authenticated = false;
    socket.setTimeout(5_000, () => socket.destroy());
    receive(socket, value => {
      if (!authenticated) {
        if (owner || typeof value !== "object" || value === null || !("token" in value) || value.token !== token) {
          socket.destroy(); return;
        }
        authenticated = true;
        owner = socket;
        socket.setTimeout(0);
        return;
      }
      message(value);
    });
  });
  const listening = Promise.withResolvers<void>();
  server.once("error", listening.reject);
  server.listen(path, listening.resolve);
  await listening.promise;
  await chmod(path, 0o600);
  return {
    env: { RERAIL_PLAYGROUND_IPC: path, RERAIL_PLAYGROUND_TOKEN: token },
    send(value: unknown) { send(owner, value); },
    close() {
      closing ??= (async () => {
        for (const socket of clients) socket.destroy();
        const closed = Promise.withResolvers<void>();
        server.close(error => error ? closed.reject(error) : closed.resolve());
        await closed.promise;
        await rm(path, { force: true });
      })();
      return closing;
    },
  };
}
export async function connectWorkerChannel(message: (value: unknown) => void, disconnect: () => void) {
  const path = process.env.RERAIL_PLAYGROUND_IPC;
  const token = process.env.RERAIL_PLAYGROUND_TOKEN;
  if (!path || !token) throw new Error("Playground supervisor capability is missing");
  delete process.env.RERAIL_PLAYGROUND_IPC;
  delete process.env.RERAIL_PLAYGROUND_TOKEN;
  const socket = createConnection(path);
  const connected = Promise.withResolvers<void>();
  socket.once("connect", connected.resolve);
  socket.once("error", connected.reject);
  socket.on("error", disconnect);
  socket.once("close", disconnect);
  receive(socket, message);
  await connected.promise;
  send(socket, { token });
  return { send(value: unknown) { send(socket, value); } };
}

export const workerControl = Object.assign(new EventEmitter(), {
  send(_value: unknown): void { throw new Error("Worker control channel is disconnected"); },
});
export async function initializeWorkerChannel(): Promise<void> {
  const channel = await connectWorkerChannel(
    value => workerControl.emit("message", value),
    () => workerControl.emit("disconnect"),
  );
  workerControl.send = channel.send;
}
