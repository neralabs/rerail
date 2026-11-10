import { randomBytes } from "node:crypto";
import { createServer, type Server } from "node:net";
import { join } from "node:path";

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
 * Holds `endpoint` open, as a live daemon holds its engine pipe, until the returned function closes it. A daemon lookup
 * on Windows counts a recorded daemon whose pipe is gone as stale (`lookupDaemon`); elsewhere nothing is held.
 */
export async function holdEngineEndpoint(endpoint: string): Promise<() => Promise<void>> {
  if (process.platform !== "win32") return async () => {};
  const server: Server = createServer(socket => socket.destroy());
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(endpoint, () => resolve());
  });
  return async () => await new Promise<void>(resolve => server.close(() => resolve()));
}
