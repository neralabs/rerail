import { lstat, mkdir, unlink } from "node:fs/promises";
import { createServer, request, type Server } from "node:http";
import { connect } from "node:net";
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
import { DAEMON_ACCESS_HEADER, sameSecret, type EngineEndpoint } from "./daemon-control.js";
import { RepositoryWorktreeWalks } from "../engine/code-rule/worktree-walks.js";
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

/** Why `directory` cannot hold the engine socket, and what the person can do about it. */
interface DirectoryRefusal {
  problem: string;
  remedy: string;
}

/**
 * Makes `directory` (mode 0700, with any missing parents) and answers why it is refused, or undefined when it is this
 * user's alone: a real directory, not a link, owned by `uid`, with no group or other permission bits. Windows has no
 * POSIX owner or mode to check (`uid` undefined).
 */
async function privateDirectoryRefusal(directory: string, uid: number | undefined): Promise<DirectoryRefusal | undefined> {
  try {
    await mkdir(directory, { recursive: true, mode: 0o700 });
  } catch (error) {
    // Something that is not a directory already holds the name; `lstat` below says what and whose.
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") {
      return { problem: `could not be made (${(error as Error).message})`, remedy: `Check that this user may create ${directory}.` };
    }
  }
  if (uid === undefined) return undefined;
  const info = await lstat(directory);
  const problem = !info.isDirectory() ? "is a link or not a directory"
    : info.uid !== uid ? `is owned by uid ${info.uid}, not ${uid}`
      : (info.mode & 0o077) !== 0 ? `has mode ${(info.mode & 0o777).toString(8)}, not 700`
        : undefined;
  if (problem === undefined) return undefined;
  // In a shared temp directory (sticky bit) only its owner or an administrator can remove what another user made.
  const remedy = info.uid !== uid
    ? `Another user (uid ${info.uid}) created it; they or an administrator must remove ${directory}.`
    : `Remove ${directory} and Rerail makes it again, for this user alone.`;
  return { problem, remedy };
}

/**
 * Makes `directory` (mode 0700, with any missing parents) and refuses it unless it is this user's alone: a real
 * directory, not a link, owned by `uid`, with no group or other permission bits. The socket inside then takes no
 * permission change after `listen`: nobody else can reach, replace or pre-create it. A directory another user made
 * first at a predictable path in a shared temp directory is refused rather than used. Windows has no POSIX owner or
 * mode to check (`uid` undefined).
 */
export async function preparePrivateDirectory(directory: string, uid: number | undefined = process.getuid?.()): Promise<void> {
  const refusal = await privateDirectoryRefusal(directory, uid);
  if (refusal !== undefined) {
    throw new Error(`Rerail will not put its engine socket in ${directory}: it ${refusal.problem}. The socket needs a directory only this user can open (mode 700). ${refusal.remedy}`);
  }
}

/**
 * The first of `candidates` that can carry the engine socket: a Windows named pipe as it is, a Unix socket path that
 * fits `sun_path` and whose directory `preparePrivateDirectory` accepts (and has made). Anyone can make a directory at a
 * predictable temp path before this user does; a later candidate keeps Rerail working instead of every hook failing
 * open. When none can, the error names each one refused, why, and what to do.
 */
export async function chooseEngineSocketPath(
  candidates: readonly string[],
  uid: number | undefined = process.getuid?.(),
  platform: NodeJS.Platform = process.platform,
): Promise<string> {
  const refused: string[] = [];
  // The longest Unix socket path in bytes, the terminating NUL aside: `sun_path` holds 108 on Linux, 104 on macOS and
  // the BSDs. A longer path fails to bind, or on some systems binds a truncated name no client finds.
  const limit = platform === "linux" || platform === "android" ? 107 : 103;
  for (const candidate of candidates) {
    if (isWindowsNamedPipe(candidate)) return candidate;
    const bytes = Buffer.byteLength(candidate);
    if (bytes > limit) {
      refused.push(`${candidate} is ${bytes} bytes, longer than the ${limit} a socket path may be here. Pointing RERAIL_HOME (or TMPDIR) at a shorter folder shortens it.`);
      continue;
    }
    const directory = dirname(candidate);
    const refusal = await privateDirectoryRefusal(directory, uid);
    if (refusal === undefined) return candidate;
    refused.push(`${directory}: it ${refusal.problem}. ${refusal.remedy}`);
  }
  throw new Error(`Rerail found nowhere to put its engine socket, which needs a directory only this user can open (mode 700):\n${refused.map(line => `- ${line}`).join("\n")}`);
}

type EngineServiceOptions = Omit<RerailRuntimeOptions, "workspaceRoot"> & {
  socketPath: string;
  workspaceRoot?: string;
  /**
   * The capability every request must carry as `x-rerail-access`: the daemon's own. A request decides where its
   * runtime writes (`storageRoot`, `databasePath`), and the daemon runs outside any coding agent's sandbox, so a
   * process that can connect to the socket but cannot read the Rerail home must not be able to use it. Only a
   * standalone service (tests, benchmarks) omits it.
   */
  accessToken?: string;
};

interface EngineRoutingState {
  runtimes: Map<string, RerailRuntime>;
  runtimeKeysBySession: Map<string, Set<string>>;
  runtimeKeysByConfiguration: Map<string, string>;
  lanes: SessionLanes;
  pending: Set<Promise<readonly AdapterDelivery[]>>;
  /**
   * The walk of each repository's other worktrees, shared by the sessions of every runtime: hooks name the session's
   * folder, so sessions in different worktrees of one repository run in different runtimes, and each walking every
   * worktree for itself held every file of every worktree once per session.
   */
  worktreeWalks: RepositoryWorktreeWalks;
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
  const { accessToken: _accessToken, ...serviceDefaults } = options;
  const runtimeOptions: RerailRuntimeOptions = configuration === undefined
    ? {
        ...serviceDefaults,
        storageRoot: resolvePath(options.storageRoot),
        databasePath: resolvePath(options.databasePath ?? defaultRerailDatabasePath()),
        workspaceRoot: resolvePath(workspaceRoot),
      }
    : {
        storageRoot: resolvePath(configuration.storageRoot),
        modelPath: resolvePath(configuration.modelPath),
        databasePath: resolvePath(configuration.databasePath ?? options.databasePath ?? defaultRerailDatabasePath()),
        workspaceRoot: resolvePath(workspaceRoot),
      };
  const runtimeKey = JSON.stringify([
    runtimeOptions.storageRoot,
    runtimeOptions.modelPath,
    runtimeOptions.databasePath,
    runtimeOptions.workspaceRoot,
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
      runtime = new RerailRuntime({ ...runtimeOptions, autostart: false, startGate: true, worktreeWalks: state.worktreeWalks });
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

function listen(server: Server, socketPath: string): Promise<void> {
  const listening = Promise.withResolvers<void>();
  server.once("error", listening.reject);
  server.listen(socketPath, () => {
    server.off("error", listening.reject);
    listening.resolve();
  });
  return listening.promise;
}

/** Whether a process accepts connections on the Unix socket at `socketPath`; false when the file is a dead daemon's. */
async function socketAnswers(socketPath: string): Promise<boolean> {
  const probed = Promise.withResolvers<boolean>();
  const probe = connect(socketPath);
  probe.once("connect", () => {
    probe.destroy();
    probed.resolve(true);
  });
  probe.once("error", (error: NodeJS.ErrnoException) => {
    if (error.code === "ECONNREFUSED" || error.code === "ENOENT") probed.resolve(false);
    else probed.reject(error);
  });
  return probed.promise;
}

/**
 * Listens on `socketPath`. A daemon killed without cleanup (`kill -9`, a crash) leaves its Unix socket file behind, and
 * binding over it fails with EADDRINUSE: when no process answers on that socket, the file is removed and the bind tried
 * once more; when one does, the error says a live daemon owns it. Windows named pipes leave no file to clear.
 */
async function listenOnSocket(server: Server, socketPath: string): Promise<void> {
  try {
    await listen(server, socketPath);
  } catch (error) {
    if (isWindowsNamedPipe(socketPath) || (error as NodeJS.ErrnoException).code !== "EADDRINUSE") throw error;
    // Only a socket is a dead daemon's to clear: anything else at the path is left alone, whatever a probe would say.
    if (!(await lstat(socketPath)).isSocket()) {
      throw new Error(`Rerail's engine socket path ${socketPath} holds something that is not a socket; move it aside before starting the daemon.`, { cause: error });
    }
    if (await socketAnswers(socketPath)) {
      throw new Error(`Rerail's engine socket ${socketPath} is in use by a running daemon; stop it with \`rerail daemon stop\` before starting another.`, { cause: error });
    }
    await unlink(socketPath).catch((unlinkError: NodeJS.ErrnoException) => {
      if (unlinkError.code !== "ENOENT") throw unlinkError;
    });
    await listen(server, socketPath);
  }
}

export async function startEngineService(options: EngineServiceOptions): Promise<EngineService> {
  if (!isWindowsNamedPipe(options.socketPath)) await preparePrivateDirectory(dirname(options.socketPath));
  const routing: EngineRoutingState = {
    runtimes: new Map(),
    runtimeKeysBySession: new Map(),
    runtimeKeysByConfiguration: new Map(),
    lanes: new SessionLanes(),
    pending: new Set(),
    worktreeWalks: options.worktreeWalks ?? new RepositoryWorktreeWalks(),
  };
  const server = createServer(async (incoming, response) => {
    response.setHeader("Content-Type", "application/json");
    if (incoming.method !== "POST" || incoming.url !== "/events") {
      response.writeHead(404).end();
      return;
    }
    const presented = incoming.headers[DAEMON_ACCESS_HEADER];
    if (options.accessToken !== undefined && !sameSecret(typeof presented === "string" ? presented : undefined, options.accessToken)) {
      // Refused before the body is read: nothing of the request reaches a runtime.
      response.writeHead(401).end(JSON.stringify({ error: "engine-access-required" }));
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
    await listenOnSocket(server, options.socketPath);
  } catch (error) {
    server.close();
    await Promise.all([...routing.runtimes.values()].map(runtime => runtime.dispose()));
    throw error;
  }
  return engineService;
}

export async function ingestEngineEvent(
  endpoint: EngineEndpoint,
  event: AdapterEvent,
  configuration?: EngineRuntimeConfiguration,
): Promise<readonly AdapterDelivery[]> {
  const body = JSON.stringify(RequestSchema.parse({ event, configuration }));
  if (Buffer.byteLength(body) > RERAIL_EVENT_MAX_FRAME_BYTES) throw new RangeError("Kernel event exceeds frame limit");
  const { promise, resolve, reject } = Promise.withResolvers<readonly AdapterDelivery[]>();
  const outgoing = request(
    {
      socketPath: endpoint.socketPath,
      method: "POST",
      path: "/events",
      agent: false,
      headers: {
        "content-type": "application/json",
        "content-length": Buffer.byteLength(body),
        ...(endpoint.accessToken === undefined ? {} : { [DAEMON_ACCESS_HEADER]: endpoint.accessToken }),
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
