import { lstat, mkdir, unlink } from "node:fs/promises";
import { createServer, request, type Server, type ServerResponse } from "node:http";
import { connect } from "node:net";
import { dirname } from "node:path";
import { z } from "zod/v4";
import type { AdapterDelivery, AdapterEvent } from "../engine/adapter-contract.js";
import {
  AdapterEventSchema,
  decodeRerailEventResult,
  encodeRerailEventResult,
  RERAIL_EVENT_MAX_FRAME_BYTES,
} from "./event-protocol.js";
import { DAEMON_ACCESS_HEADER, sameSecret, type EngineEndpoint } from "./daemon-control.js";
import { ENGINE_CHALLENGE_HEADER, ENGINE_PROOF_PATH, engineHoldsCapability, engineProof, isEngineChallenge } from "./engine-proof.js";
import { isNamedPipe } from "./engine-pipe.js";
import {
  RuntimeConfigurationSchema,
  RuntimePool,
  type RuntimeConfiguration,
  type RuntimeDefaults,
  type RuntimeLifetimes,
} from "./runtime-pool.js";

export interface EngineService {
  /** Where it listens: the socket path it was given, or its `pipeFallback` when another process held that pipe. */
  readonly socketPath: string;
  close(): Promise<void>;
  ingest(
    event: AdapterEvent,
    configuration?: EngineRuntimeConfiguration,
  ): Promise<readonly AdapterDelivery[]>;
  /** How many runtimes are open: one per checkout a session uses, or used within `idleRuntimeMs`. */
  openRuntimes(): number;
  /**
   * Closes every session that has sent no event for the idle limit and owes its agent nothing, and every one quiet for
   * a day whatever it owes; the service also does this every minute.
   */
  closeIdleSessions(): Promise<void>;
}

/** How often the engine service looks for idle sessions (`RuntimePool.closeIdleSessions`). */
const IDLE_SWEEP_MS = 60_000;

export type EngineRuntimeConfiguration = RuntimeConfiguration;
const RequestSchema = z
  .object({
    event: AdapterEventSchema,
    configuration: RuntimeConfigurationSchema.optional(),
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

type EngineServiceOptions = RuntimeDefaults & RuntimeLifetimes & {
  socketPath: string;
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
};

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
  // The pool gets every option but the capability, which only this service checks, and where the service listens.
  const { accessToken: _accessToken, socketPath: _socketPath, pipeFallback: _pipeFallback, ...pooled } = options;
  const pool = new RuntimePool(pooled);
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
    ingest: (event, configuration) => pool.ingest(event, configuration),
    openRuntimes: () => pool.openRuntimes(),
    closeIdleSessions: () => pool.closeIdleSessions(),
    async close() {
      clearInterval(idleSweep);
      const closed = Promise.withResolvers<void>();
      server.close((error) => (error ? closed.reject(error) : closed.resolve()));
      await closed.promise;
      await pool.close();
    },
  };
  server.requestTimeout = 30_000;
  // Never keeps the process alive; `closeIdleSessions` reports its own failures.
  const idleSweep = setInterval(() => void pool.closeIdleSessions(), IDLE_SWEEP_MS);
  idleSweep.unref();
  try {
    socketPath = await listenOnSocket(server, options.socketPath, options);
  } catch (error) {
    clearInterval(idleSweep);
    server.close();
    await pool.close();
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
