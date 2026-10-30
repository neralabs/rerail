import { lstat, mkdir } from "node:fs/promises";
import { createServer, request } from "node:http";
import { dirname, resolve as resolvePath } from "node:path";
import { z } from "zod/v4";
import type { AdapterDelivery, AdapterEvent } from "../engine/adapter-contract.js";
import {
  AdapterEventSchema,
  decodeRerailEventResult,
  encodeRerailEventResult,
  RERAIL_EVENT_MAX_FRAME_BYTES,
} from "./event-protocol.js";
import { defaultRerailDatabasePath } from "../engine/storage/database.js";
import { RerailRuntime, type RerailRuntimeOptions } from "./runtime.js";
import { SessionLanes } from "./session-lanes.js";

export interface EngineService {
  close(): Promise<void>;
  ingest(
    event: AdapterEvent,
    configuration?: EngineRuntimeConfiguration,
  ): Promise<readonly AdapterDelivery[]>;
}

const ConfigurationSchema = z
  .object({
    storageRoot: z.string().min(1),
    modelPath: z.string().min(1),
    /** Absent selects the engine service's own database. */
    databasePath: z.string().min(1).optional(),
    workspaceRoot: z.string().min(1).optional(),
    /** False for a caller without the daemon token: its runtime records, sets up and imports nothing. */
    trusted: z.boolean().optional(),
  })
  .strict();
export type EngineRuntimeConfiguration = z.infer<typeof ConfigurationSchema>;
const RequestSchema = z
  .object({
    event: AdapterEventSchema,
    configuration: ConfigurationSchema.optional(),
  })
  .strict();

interface EngineRequest {
  event: AdapterEvent;
  configuration: EngineRuntimeConfiguration | undefined;
}

function parseEngineRequest(value: unknown): EngineRequest {
  const { event, configuration } = RequestSchema.parse(value);
  return { event, configuration };
}


function isWindowsNamedPipe(endpoint: string): boolean {
  return endpoint.startsWith("\\\\.\\pipe\\");
}

/**
 * Makes `directory` (mode 0700, with any missing parents) and refuses it unless it is this user's alone: a real
 * directory, not a link, owned by `uid`, with no group or other permission bits. The socket inside then takes no
 * permission change after `listen`: nobody else can reach, replace or pre-create it. A directory another user made
 * first at a predictable path in a shared temp directory is refused rather than used. Windows has no POSIX owner or
 * mode to check (`uid` undefined).
 */
export async function preparePrivateDirectory(directory: string, uid: number | undefined = process.getuid?.()): Promise<void> {
  await mkdir(directory, { recursive: true, mode: 0o700 });
  if (uid === undefined) return;
  const info = await lstat(directory);
  const problem = !info.isDirectory() ? "is a link or not a directory"
    : info.uid !== uid ? `is owned by uid ${info.uid}, not ${uid}`
      : (info.mode & 0o077) !== 0 ? `has mode ${(info.mode & 0o777).toString(8)}, not 700`
        : undefined;
  if (problem !== undefined) {
    throw new Error(`Rerail will not put its engine socket in ${directory}: it ${problem}. The socket needs a directory only this user can open (mode 700).`);
  }
}

type EngineServiceOptions = Omit<RerailRuntimeOptions, "workspaceRoot"> & {
  socketPath: string;
  workspaceRoot?: string;
};

interface EngineRoutingState {
  runtimes: Map<string, RerailRuntime>;
  runtimeKeysBySession: Map<string, Set<string>>;
  runtimeKeysByConfiguration: Map<string, string>;
  lanes: SessionLanes;
  pending: Set<Promise<readonly AdapterDelivery[]>>;
}

interface RuntimeRoute {
  runtimeKey: string;
  runtimeOptions?: RerailRuntimeOptions;
}

function resolveRuntimeRoute(
  event: AdapterEvent,
  configuration: EngineRuntimeConfiguration | undefined,
  options: EngineServiceOptions,
  state: EngineRoutingState,
): RuntimeRoute {
  const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
  const eventWorkspace = event.kind === "session-start" ? event.session.cwd : event.session?.cwd;
  const workspaceRoot = configuration?.workspaceRoot ?? options.workspaceRoot ?? eventWorkspace;
  const configurationKey = configuration === undefined
    ? undefined
    : JSON.stringify([
        resolvePath(configuration.storageRoot),
        resolvePath(configuration.modelPath),
        resolvePath(configuration.databasePath ?? options.databasePath ?? defaultRerailDatabasePath()),
        configuration.trusted ?? true,
      ]);
  if (workspaceRoot === undefined) {
    const configuredRuntimeKey = configurationKey === undefined
      ? undefined
      : state.runtimeKeysByConfiguration.get(configurationKey);
    const candidates = state.runtimeKeysBySession.get(sessionId);
    if (configuredRuntimeKey === undefined && candidates?.size !== 1) {
      throw new Error("Kernel runtime requires workspaceRoot or unambiguous session context");
    }
    return { runtimeKey: configuredRuntimeKey ?? candidates!.values().next().value! };
  }
  const runtimeOptions: RerailRuntimeOptions = configuration === undefined
    ? {
        ...options,
        storageRoot: resolvePath(options.storageRoot),
        databasePath: resolvePath(options.databasePath ?? defaultRerailDatabasePath()),
        workspaceRoot: resolvePath(workspaceRoot),
      }
    : {
        storageRoot: resolvePath(configuration.storageRoot),
        modelPath: resolvePath(configuration.modelPath),
        databasePath: resolvePath(configuration.databasePath ?? options.databasePath ?? defaultRerailDatabasePath()),
        workspaceRoot: resolvePath(workspaceRoot),
        trusted: configuration.trusted ?? true,
      };
  // A trusted and an untrusted caller never share a runtime.
  const runtimeKey = JSON.stringify([
    runtimeOptions.storageRoot,
    runtimeOptions.modelPath,
    runtimeOptions.databasePath,
    runtimeOptions.workspaceRoot,
    runtimeOptions.trusted ?? true,
  ]);
  if (configurationKey !== undefined) state.runtimeKeysByConfiguration.set(configurationKey, runtimeKey);
  return { runtimeKey, runtimeOptions };
}

async function routeEngineEvent(
  event: AdapterEvent,
  configuration: EngineRuntimeConfiguration | undefined,
  options: EngineServiceOptions,
  state: EngineRoutingState,
): Promise<readonly AdapterDelivery[]> {
  const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
  const { runtimeKey, runtimeOptions } = resolveRuntimeRoute(event, configuration, options, state);
  const execution = state.lanes.run(JSON.stringify([runtimeKey, sessionId]), () => {
    let runtime = state.runtimes.get(runtimeKey);
    if (runtime === undefined) {
      if (runtimeOptions === undefined) throw new Error(`Kernel runtime is unavailable for session ${sessionId}`);
      runtime = new RerailRuntime({ ...runtimeOptions, autostart: false, startGate: true });
      state.runtimes.set(runtimeKey, runtime);
    }
    if (event.kind === "session-start") {
      const keys = state.runtimeKeysBySession.get(sessionId) ?? new Set<string>();
      keys.add(runtimeKey);
      state.runtimeKeysBySession.set(sessionId, keys);
    }
    return runtime.ingest(event);
  });
  state.pending.add(execution);
  void execution.then(
    () => state.pending.delete(execution),
    () => state.pending.delete(execution),
  );
  const deliveries = await execution;
  if (event.kind === "session-end") {
    const keys = state.runtimeKeysBySession.get(sessionId);
    keys?.delete(runtimeKey);
    if (keys?.size === 0) state.runtimeKeysBySession.delete(sessionId);
  }
  return deliveries;
}

export async function startEngineService(options: EngineServiceOptions): Promise<EngineService> {
  if (!isWindowsNamedPipe(options.socketPath)) await preparePrivateDirectory(dirname(options.socketPath));
  const routing: EngineRoutingState = {
    runtimes: new Map(),
    runtimeKeysBySession: new Map(),
    runtimeKeysByConfiguration: new Map(),
    lanes: new SessionLanes(),
    pending: new Set(),
  };
  const server = createServer(async (incoming, response) => {
    response.setHeader("Content-Type", "application/json");
    if (incoming.method !== "POST" || incoming.url !== "/events") {
      response.writeHead(404).end();
      return;
    }
    try {
      const chunks: Buffer[] = [];
      let size = 0;
      for await (const chunk of incoming) {
        const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
        size += bytes.length;
        if (size > RERAIL_EVENT_MAX_FRAME_BYTES) throw new RangeError("Kernel event exceeds frame limit");
        chunks.push(bytes);
      }
      const { event, configuration } = parseEngineRequest(JSON.parse(Buffer.concat(chunks).toString("utf8")));
      const deliveries = await engineService.ingest(event, configuration);
      response.end(encodeRerailEventResult({ eventId: event.eventId, deliveries: [...deliveries] }));
    } catch (error) {
      response.writeHead(400).end(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }));
    }
  });
  const engineService: EngineService = {
    ingest: (event, configuration) => routeEngineEvent(event, configuration, options, routing),
    async close() {
      const closed = Promise.withResolvers<void>();
      server.close((error) => (error ? closed.reject(error) : closed.resolve()));
      await closed.promise;
      await Promise.allSettled(routing.pending);
      await Promise.all([...routing.runtimes.values()].map(runtime => runtime.dispose()));
    },
  };
  server.requestTimeout = 30_000;
  try {
    const listening = Promise.withResolvers<void>();
    server.once("error", listening.reject);
    server.listen(options.socketPath, () => {
      server.off("error", listening.reject);
      listening.resolve();
    });
    await listening.promise;
  } catch (error) {
    server.close();
    await Promise.all([...routing.runtimes.values()].map(runtime => runtime.dispose()));
    throw error;
  }
  return engineService;
}

export async function ingestEngineEvent(
  socketPath: string,
  event: AdapterEvent,
  configuration?: EngineRuntimeConfiguration,
): Promise<readonly AdapterDelivery[]> {
  const body = JSON.stringify(RequestSchema.parse({ event, configuration }));
  if (Buffer.byteLength(body) > RERAIL_EVENT_MAX_FRAME_BYTES) throw new RangeError("Kernel event exceeds frame limit");
  const { promise, resolve, reject } = Promise.withResolvers<readonly AdapterDelivery[]>();
  const outgoing = request(
    {
      socketPath,
      method: "POST",
      path: "/events",
      agent: false,
      headers: {
        "content-type": "application/json",
        "content-length": Buffer.byteLength(body),
      },
    },
    (response) => {
      const chunks: Buffer[] = [];
      response.on("data", (chunk: Buffer) => chunks.push(chunk));
      response.on("error", reject);
      response.on("end", () => {
        try {
          const text = Buffer.concat(chunks).toString("utf8");
          if (response.statusCode !== 200) throw new Error(`Kernel request failed (${response.statusCode}): ${text}`);
          const result = decodeRerailEventResult(text);
          if (result.eventId !== event.eventId) throw new Error("Kernel response event identity mismatch");
          resolve(result.deliveries);
        } catch (error) {
          reject(error);
        }
      });
    },
  );
  outgoing.setTimeout(30_000, () => outgoing.destroy(new Error("Kernel request timed out")));
  outgoing.on("error", reject);
  outgoing.end(body);
  return promise;
}
