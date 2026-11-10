import { lstat, mkdir, unlink } from "node:fs/promises";
import { createServer, request, type Server, type ServerResponse } from "node:http";
import { connect } from "node:net";
import { dirname, resolve as resolvePath } from "node:path";
import { setFlagsFromString } from "node:v8";
import { runInNewContext } from "node:vm";
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
import { ENGINE_CHALLENGE_HEADER, ENGINE_PROOF_PATH, engineHoldsCapability, engineProof, isEngineChallenge } from "./engine-proof.js";
import { isNamedPipe } from "./engine-pipe.js";
import { RerailRuntime, type RerailRuntimeOptions } from "./runtime.js";
import { SessionLanes } from "./session-lanes.js";

export interface EngineService {
  /** Where it listens: the socket path it was given, or its `pipeFallback` when another process held that pipe. */
  readonly socketPath: string;
  close(): Promise<void>;
  ingest(
    event: AdapterEvent,
    configuration?: EngineRuntimeConfiguration,
  ): Promise<readonly AdapterDelivery[]>;
  /** How many runtimes are open: one per checkout some live session uses. */
  openRuntimes(): number;
  /**
   * Closes every session that has sent no event for the idle limit and owes its agent nothing; the service also does
   * this every minute.
   */
  closeIdleSessions(): Promise<void>;
}

/**
 * How long a session may send no event before the engine service closes it as its end would. Its end is not certain to
 * arrive: a coding agent that is killed sends none, and Claude Code gives its SessionEnd hook 1.5 s, which a hook on a
 * busy Windows machine outlasts. A session whose end was lost kept its state (its own copy of its checkout's source
 * walk, its conversation history, its evaluator) until the daemon stopped. A closed session's next event opens it again
 * from its ledger. A session waiting on a tool call, or holding a steer for its agent, stays open: its ledger holds
 * neither.
 */
export const IDLE_SESSION_MS = 30 * 60_000;
/** How often the engine service looks for idle sessions. */
const IDLE_SWEEP_MS = 60_000;

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
    if (isNamedPipe(candidate)) return candidate;
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
  /**
   * A pipe name to listen on instead when `socketPath`, a Windows named pipe, is held by a process that cannot prove it
   * is this home's engine service (`engineHoldsCapability`). Any local user can create a pipe of any name, and
   * `socketPath` is derived from the database path, which anyone can work out, so without one a pipe made first stops
   * every daemon start. Clients find the pipe through the state file, never by computing it. A socket path the caller
   * named outright gets none.
   */
  pipeFallback?: () => string;
  /** Defaults to `IDLE_SESSION_MS`; tests pass their own. */
  idleSessionMs?: number;
  /** The clock idle sessions are measured by; defaults to `Date.now`. */
  now?: () => number;
  /** How long a checkout's runtime stays open once no session uses it; {@link IDLE_RUNTIME_MS} by default. */
  idleRuntimeMs?: number;
};

/**
 * How long a checkout's runtime stays open after its last session ends. A session that starts there again within it
 * finds its rules compiled and its checkout walked: the five packs a TypeScript repository adds take 12 s of CPU to
 * compile on a Windows laptop. After it the runtime closes, and the engine frees what its rules held.
 */
export const IDLE_RUNTIME_MS = 15 * 60_000;

interface EngineRoutingState {
  runtimes: Map<string, RerailRuntime>;
  runtimeKeysBySession: Map<string, Set<string>>;
  runtimeKeysByConfiguration: Map<string, string>;
  /** When each open session last sent an event, by its lane (`laneOf`). */
  lastEventAt: Map<string, number>;
  lanes: SessionLanes;
  pending: Set<Promise<readonly AdapterDelivery[]>>;
  /** The runtimes no session uses, each with the timer that closes it. */
  idle: Map<string, NodeJS.Timeout>;
  /** How long a runtime no session uses stays open: `idleRuntimeMs`, or `IDLE_RUNTIME_MS`. */
  idleRuntimeMs: number;
}

/** The lane one session's work in one runtime runs in, one at a time. */
function laneOf(runtimeKey: string, sessionId: string): string {
  return JSON.stringify([runtimeKey, sessionId]);
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
  const { accessToken: _accessToken, idleSessionMs: _idleSessionMs, now: _now, idleRuntimeMs: _idleRuntimeMs, ...serviceDefaults } = options;
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
  const lane = laneOf(runtimeKey, sessionId);
  clearTimeout(state.idle.get(runtimeKey));
  state.idle.delete(runtimeKey);
  const execution = state.lanes.run(lane, () => {
    let runtime = state.runtimes.get(runtimeKey);
    if (runtime === undefined) {
      if (runtimeOptions === undefined) throw new Error(`Kernel runtime is unavailable for session ${sessionId}`);
      runtime = new RerailRuntime({ ...runtimeOptions, autostart: false, startGate: true });
      state.runtimes.set(runtimeKey, runtime);
    }
    // Every event, not only a start: a session whose start was lost still uses this runtime until it ends.
    if (event.kind !== "session-end") {
      const keys = state.runtimeKeysBySession.get(sessionId) ?? new Set<string>();
      keys.add(runtimeKey);
      state.runtimeKeysBySession.set(sessionId, keys);
      state.lastEventAt.set(lane, (options.now ?? Date.now)());
    }
    return runtime.ingest(event);
  });
  track(state, execution);
  const deliveries = await execution;
  if (event.kind === "session-end") {
    state.lastEventAt.delete(lane);
    releaseSessionRuntime(state, sessionId, runtimeKey);
  }
  return deliveries;
}

/** Keeps `work` among what `close` waits for until it settles. */
function track(state: EngineRoutingState, work: Promise<unknown>): void {
  const settled = work.then(() => [] as const, () => [] as const);
  state.pending.add(settled);
  void settled.then(() => state.pending.delete(settled));
}

/** `sessionId` no longer uses the runtime at `runtimeKey`, which closes once no session has used it a while. */
function releaseSessionRuntime(state: EngineRoutingState, sessionId: string, runtimeKey: string): void {
  const keys = state.runtimeKeysBySession.get(sessionId);
  keys?.delete(runtimeKey);
  if (keys?.size === 0) state.runtimeKeysBySession.delete(sessionId);
  retireUnusedRuntime(state, runtimeKey);
}

/**
 * Closes, as its end would, each session that has sent no event for `idleMs` (`IDLE_SESSION_MS`), and then retires its
 * runtime as an end does. The check runs again in the session's lane, so an event that came in meanwhile keeps the
 * session open. A session that still owes its agent a tool call's check, a steer or a scan stays open
 * (`RerailRuntime.closeIdleSession`), and so does one whose close fails, which is reported; the next sweep looks again.
 */
async function closeIdleSessions(state: EngineRoutingState, idleMs: number, now: () => number): Promise<void> {
  const idle = (lane: string): boolean => {
    const at = state.lastEventAt.get(lane);
    return at !== undefined && now() - at >= idleMs;
  };
  await Promise.all([...state.lastEventAt.keys()].filter(idle).map(async (lane) => {
    const [runtimeKey, sessionId] = JSON.parse(lane) as [string, string];
    const closing = state.lanes.run(lane, async () => {
      if (!idle(lane)) return false;
      const closed = await state.runtimes.get(runtimeKey)?.closeIdleSession(sessionId) ?? true;
      if (closed) state.lastEventAt.delete(lane);
      return closed;
    });
    track(state, closing);
    try {
      if (await closing) releaseSessionRuntime(state, sessionId, runtimeKey);
    } catch (error) {
      process.stderr.write(`rerail: closing idle session ${sessionId} failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }));
}

function runtimeInUse(state: EngineRoutingState, runtimeKey: string): boolean {
  for (const keys of state.runtimeKeysBySession.values()) if (keys.has(runtimeKey)) return true;
  return false;
}

/**
 * Disposes the runtime at `runtimeKey` once no session has used it for `state.idleRuntimeMs`. A runtime holds its
 * checkout's whole source walk (one observation for every file of the working tree, ignored ones included, about
 * 100,000 in a large repository) and its compiled rules, so a daemon that kept the runtime of every checkout and
 * worktree it ever served grew until its heap ran out. The next event for that checkout makes a new runtime, as the
 * first one did.
 */
function retireUnusedRuntime(state: EngineRoutingState, runtimeKey: string): void {
  if (runtimeInUse(state, runtimeKey) || !state.runtimes.has(runtimeKey) || state.idle.has(runtimeKey)) return;
  if (state.idleRuntimeMs <= 0) {
    disposeRuntime(state, runtimeKey);
    return;
  }
  const timer = setTimeout(() => {
    state.idle.delete(runtimeKey);
    if (!runtimeInUse(state, runtimeKey)) disposeRuntime(state, runtimeKey);
  }, state.idleRuntimeMs);
  timer.unref();
  state.idle.set(runtimeKey, timer);
}

/**
 * A full collection once a runtime closed, so the engine frees the programs of the rules it compiled. They are native
 * memory V8 does not count (512 MB for the Node pack's builtins rule alone), and a daemon with no session open
 * allocates too little to start a collection: the closed runtime's rules stayed in memory until the next session came.
 */
function collectGarbage(): void {
  setFlagsFromString("--expose-gc");
  const gc: unknown = runInNewContext("gc");
  if (typeof gc === "function") gc();
}

function disposeRuntime(state: EngineRoutingState, runtimeKey: string): void {
  const runtime = state.runtimes.get(runtimeKey);
  if (runtime === undefined) return;
  state.runtimes.delete(runtimeKey);
  for (const [configuration, key] of state.runtimeKeysByConfiguration) if (key === runtimeKey) state.runtimeKeysByConfiguration.delete(configuration);
  const disposal = runtime.dispose().then(() => {
    collectGarbage();
    return [] as const;
  }, (error: unknown) => {
    process.stderr.write(`rerail: closing an idle runtime failed: ${error instanceof Error ? error.message : String(error)}\n`);
    return [] as const;
  });
  state.pending.add(disposal);
  void disposal.then(() => state.pending.delete(disposal));
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
 * Whether this home's engine service answers on the pipe `socketPath`: it proves it holds `accessToken` by answering a
 * challenge (`engineHoldsCapability`), which never shows the holder the capability. Any local user can create the pipe
 * name first, and a probe that presented the capability would hand it to them. Without a capability to check, any
 * holder counts as this home's.
 */
async function ownEngineAnswers(socketPath: string, accessToken: string | undefined): Promise<boolean> {
  return accessToken === undefined || await engineHoldsCapability(socketPath, accessToken);
}

/** Answers a challenge sent to {@link ENGINE_PROOF_PATH} with the proof that this engine holds `accessToken`. */
function answerEngineChallenge(challenge: unknown, accessToken: string, response: ServerResponse): void {
  if (!isEngineChallenge(challenge)) {
    response.writeHead(400).end(JSON.stringify({ error: "engine-challenge-required" }));
    return;
  }
  response.end(JSON.stringify({ proof: engineProof(accessToken, challenge) }));
}

/** What listening beside a held pipe needs; tests pass their own. */
export interface HeldPipeListening {
  listen: (socketPath: string) => Promise<void>;
  /** Whether this home's engine service answers on the held pipe. */
  ownEngineAnswers: (socketPath: string) => Promise<boolean>;
  fallback: (() => string) | undefined;
}

/**
 * Where to listen when `pipe` is already held (`cause`, EACCES or EADDRINUSE): on the fallback pipe when the holder is not this
 * home's engine service, which a second daemon must never start beside. Returns the pipe it listens on.
 */
export async function listenBesideHeldPipe(pipe: string, cause: unknown, listening: HeldPipeListening): Promise<string> {
  if (listening.fallback === undefined) throw cause;
  if (await listening.ownEngineAnswers(pipe)) {
    throw new Error(`Rerail's engine pipe ${pipe} is in use by a running daemon; stop it with \`rerail daemon stop\` before starting another.`, { cause });
  }
  const fallback = listening.fallback();
  await listening.listen(fallback);
  return fallback;
}

/**
 * Listens on `socketPath`, and returns where it listens. A daemon killed without cleanup (`kill -9`, a crash) leaves
 * its Unix socket file behind, and binding over it fails with EADDRINUSE: when no process answers on that socket, the
 * file is removed and the bind tried once more; when one does, the error says a live daemon owns it. Windows named
 * pipes leave no file to clear; a held one moves to `pipeFallback` (`listenBesideHeldPipe`).
 */
async function listenOnSocket(
  server: Server,
  socketPath: string,
  options: Pick<EngineServiceOptions, "accessToken" | "pipeFallback">,
): Promise<string> {
  try {
    await listen(server, socketPath);
    return socketPath;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    // libuv creates a pipe's first instance alone, and reports a name another process holds as EACCES or EADDRINUSE.
    if (isNamedPipe(socketPath) && (code === "EACCES" || code === "EADDRINUSE")) {
      return await listenBesideHeldPipe(socketPath, error, {
        listen: async path => await listen(server, path),
        ownEngineAnswers: async path => await ownEngineAnswers(path, options.accessToken),
        fallback: options.pipeFallback,
      });
    }
    if (code !== "EADDRINUSE") throw error;
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
    return socketPath;
  }
}

export async function startEngineService(options: EngineServiceOptions): Promise<EngineService> {
  if (!isNamedPipe(options.socketPath)) await preparePrivateDirectory(dirname(options.socketPath));
  const routing: EngineRoutingState = {
    runtimes: new Map(),
    runtimeKeysBySession: new Map(),
    runtimeKeysByConfiguration: new Map(),
    lastEventAt: new Map(),
    lanes: new SessionLanes(),
    pending: new Set(),
    idle: new Map(),
    idleRuntimeMs: options.idleRuntimeMs ?? IDLE_RUNTIME_MS,
  };
  const sweepIdleSessions = (): Promise<void> => closeIdleSessions(routing, options.idleSessionMs ?? IDLE_SESSION_MS, options.now ?? Date.now);
  const server = createServer(async (incoming, response) => {
    response.setHeader("Content-Type", "application/json");
    if (incoming.method === "POST" && incoming.url === ENGINE_PROOF_PATH && options.accessToken !== undefined) {
      answerEngineChallenge(incoming.headers[ENGINE_CHALLENGE_HEADER], options.accessToken, response);
      incoming.resume();
      return;
    }
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
  let socketPath = options.socketPath;
  const engineService: EngineService = {
    get socketPath() {
      return socketPath;
    },
    ingest: (event, configuration) => routeEngineEvent(event, configuration, options, routing),
    openRuntimes: () => routing.runtimes.size,
    closeIdleSessions: sweepIdleSessions,
    async close() {
      clearInterval(idleSweep);
      const closed = Promise.withResolvers<void>();
      server.close((error) => (error ? closed.reject(error) : closed.resolve()));
      await closed.promise;
      for (const timer of routing.idle.values()) clearTimeout(timer);
      routing.idle.clear();
      await Promise.allSettled(routing.pending);
      await Promise.all([...routing.runtimes.values()].map(runtime => runtime.dispose()));
    },
  };
  server.requestTimeout = 30_000;
  // Never keeps the process alive; `closeIdleSessions` reports its own failures.
  const idleSweep = setInterval(() => void sweepIdleSessions(), IDLE_SWEEP_MS);
  idleSweep.unref();
  try {
    socketPath = await listenOnSocket(server, options.socketPath, options);
  } catch (error) {
    clearInterval(idleSweep);
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
