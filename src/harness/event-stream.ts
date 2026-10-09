import { once } from "node:events";
import { realpathSync } from "node:fs";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";
import { fileURLToPath } from "node:url";
import type { RerailEventLoop, RerailHost } from "./adapter-contract.js";
import { hookRerailStarted } from "./repository-start.js";
import { bundledModelPath } from "./extension.js";
import {
  decodeRerailEventFrame,
  encodeRerailEventResult,
  RERAIL_EVENT_PROTOCOL_PREFACE,
  type RerailEventErrorFrame,
} from "./event-protocol.js";
import { RerailRuntime } from "./runtime.js";
import { defaultRerailDatabasePath } from "./database.js";
import { daemonEngineSocket } from "./daemon-control.js";
import { ingestEngineEvent } from "./engine-service.js";
import { isMainRerailBuild } from "./rerail-build.js";
import { linkDeliveries } from "./session-presentation.js";

export interface RerailEventStreamOptions {
  eventLoop: RerailEventLoop;
  input: Readable;
  output: Writable;
  /** The checkout steers link to when an event names no session cwd; defaults to the process cwd. */
  workspaceRoot?: string | undefined;
}

async function writeLine(output: Writable, line: string): Promise<void> {
  if (!output.write(`${line}\n`)) await once(output, "drain");
}

function eventIdFromLine(line: string): string | null {
  try {
    const value = JSON.parse(line) as unknown;
    if (typeof value !== "object" || value === null || !("event" in value)) return null;
    const event = value.event;
    if (typeof event !== "object" || event === null || !("eventId" in event)) return null;
    return typeof event.eventId === "string" && event.eventId !== "" ? event.eventId : null;
  } catch {
    return null;
  }
}

function errorFrame(line: string, error: unknown): RerailEventErrorFrame {
  const message = error instanceof Error ? error.message : String(error);
  const code = message.startsWith("Inactive session:") ? "inactive-session" : "invalid-event";
  return { eventId: eventIdFromLine(line), error: { code, message } };
}

/**
 * Ingests one frame and returns its result. The runtime keeps steer links relative; this stream is the
 * harness's adapter, so it links them for the session's checkout (remembered from events that name it).
 */
async function frameResult(line: string, eventLoop: RerailEventLoop, sessionCwds: Map<string, string>, workspaceRoot: string): Promise<string> {
  try {
    const { event } = decodeRerailEventFrame(line);
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    if (event.session !== undefined) sessionCwds.set(sessionId, event.session.cwd);
    const deliveries = await eventLoop.ingest(event);
    const linked = await linkDeliveries(deliveries, sessionCwds.get(sessionId) ?? workspaceRoot);
    if (event.kind === "session-end") sessionCwds.delete(sessionId);
    return encodeRerailEventResult({ eventId: event.eventId, deliveries: linked });
  } catch (error) {
    return encodeRerailEventResult(errorFrame(line, error));
  }
}

export async function serveRerailEventStream(options: RerailEventStreamOptions): Promise<void> {
  const lines = createInterface({ input: options.input, crlfDelay: Infinity });
  let initialized = false;
  const workspaceRoot = options.workspaceRoot ?? process.cwd();
  const sessionCwds = new Map<string, string>();
  for await (const line of lines) {
    if (!initialized) {
      initialized = true;
      if (line === RERAIL_EVENT_PROTOCOL_PREFACE) continue;
      await writeLine(options.output, encodeRerailEventResult({
        eventId: null,
        error: { code: "unsupported-version", message: `Expected ${RERAIL_EVENT_PROTOCOL_PREFACE}` },
      }));
      return;
    }
    if (line !== "") await writeLine(options.output, await frameResult(line, options.eventLoop, sessionCwds, workspaceRoot));
  }
}

export interface RunRerailEventStreamOptions {
  storageRoot?: string;
  modelPath?: string;
  databasePath?: string;
  /** The checkout the stream serves; the process's working directory by default. */
  workspaceRoot?: string;
  input?: Readable;
  output?: Writable;
}

export async function runRerailEventStream(options: RunRerailEventStreamOptions = {}): Promise<void> {
  const workspaceRoot = resolve(options.workspaceRoot ?? process.cwd());
  // Absolute paths: in a main build the daemon, running elsewhere, opens them.
  const storageRoot = resolve(options.storageRoot ?? process.env.RERAIL_STORAGE_ROOT ?? join(workspaceRoot, ".rerail", "sessions"));
  const modelPath = options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath();
  const configuration = {
    storageRoot,
    modelPath,
    databasePath: resolve(options.databasePath ?? defaultRerailDatabasePath()),
    workspaceRoot,
  };
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  if (isMainRerailBuild()) {
    // The main daemon is the machine's one main instance; an in-process runtime here would be a second one.
    // The first event may replace a daemon from another package, as hooks do; later ones keep whichever daemon
    // runs, so a stream that outlives `rerail update` follows the newer daemon instead of displacing it.
    let replaceOtherPackage = true;
    let host: RerailHost = "claude-code";
    const explicitDatabase = options.databasePath === undefined ? undefined : configuration.databasePath;
    const eventLoop: RerailEventLoop = {
      ingest: async (event) => {
        if (event.kind === "session-start") host = event.session.host;
        // As hooks do: where Rerail is off nothing is recorded, and asking for the socket would autostart the daemon.
        if (!await hookRerailStarted(host, workspaceRoot, explicitDatabase)) return [];
        const socketPath = process.env.RERAIL_KERNEL_SOCKET
          ?? await daemonEngineSocket(replaceOtherPackage);
        replaceOtherPackage = false;
        return await ingestEngineEvent(socketPath, event, configuration);
      },
    };
    await serveRerailEventStream({ eventLoop, input, output, workspaceRoot });
    return;
  }
  const runtime = new RerailRuntime({ ...configuration, startGate: true });
  try {
    await serveRerailEventStream({ eventLoop: runtime, input, output, workspaceRoot });
  } finally {
    await runtime.dispose();
  }
}

const launchedPath = process.argv[1];
if (
  launchedPath !== undefined
  && realpathSync(fileURLToPath(import.meta.url)) === realpathSync(resolve(launchedPath))
) {
  await runRerailEventStream();
}
