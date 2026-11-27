import { once } from "node:events";
import { realpathSync } from "node:fs";
import { resolve } from "node:path";
import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";
import { fileURLToPath } from "node:url";
import { answeredFacts, type AdapterEvent, type EventAnswerFacts, type RerailEventLoop, type RerailHost } from "../engine/adapter-contract.js";
import { autostartTurnedOff, DaemonUnreachableError, hookRerailStarted, lostEventReason, unreachable } from "./hook-host.js";
import { recordHookFailure } from "./hook-failures.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import {
  decodeRerailEventFrame,
  encodeRerailEventResult,
  eventErrorCode,
  RERAIL_EVENT_PROTOCOL_PREFACE,
  type RerailEventErrorFrame,
} from "../runtime/event-protocol.js";
import { RerailRuntime } from "../runtime/runtime.js";
import { defaultRerailDatabasePath } from "../engine/storage/database.js";
import {
  autostartOff,
  daemonEngineEndpoint,
  engineEndpointAt,
  launchAutostart,
  runningDaemonEngineEndpoint,
  type EngineEndpoint,
} from "../runtime/daemon-control.js";
import { HOOK_DAEMON_WAIT_MS } from "../runtime/daemon/daemon-waits.js";
import { ingestEngineEventAnswer, type EngineRuntimeConfiguration } from "../runtime/engine-service.js";
import { isMainRerailBuild } from "../engine/native/rerail-build.js";
import { linkDeliveries } from "../presentation/session-presentation.js";

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

/**
 * The protocol's code the error carries (`eventErrorCode`), thrown by the runtime in this process or by the daemon's
 * answer in a main build, else what the stream says of it. An event no daemon took (`eventEndpoint` or its send throws
 * `DaemonUnreachableError`, as a hook's do) may be sound, so its frame says `internal-error`, not `invalid-event`.
 */
function errorCode(error: unknown): RerailEventErrorFrame["error"]["code"] {
  return eventErrorCode(error) ?? (error instanceof DaemonUnreachableError ? "internal-error" : "invalid-event");
}

function errorFrame(line: string, error: unknown): RerailEventErrorFrame {
  const message = error instanceof Error ? error.message : String(error);
  return { eventId: eventIdFromLine(line), error: { code: errorCode(error), message } };
}

/**
 * Ingests one frame and returns its result. The runtime records no steer links; this stream is the
 * harness's adapter, so it links each delivery's rules for the session's checkout (remembered from events that name it).
 */
async function frameResult(line: string, eventLoop: RerailEventLoop, sessionCwds: Map<string, string>, workspaceRoot: string): Promise<string> {
  try {
    const { event } = decodeRerailEventFrame(line);
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    if (event.session !== undefined) sessionCwds.set(sessionId, event.session.cwd);
    const deliveries = await eventLoop.ingest(event);
    const linked = await linkDeliveries(deliveries, sessionCwds.get(sessionId) ?? workspaceRoot, answeredFacts(eventLoop));
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

/**
 * A main build's event loop: each event goes to the daemon, as a hook's does. Only a session's start waits for a daemon
 * to start; any other event goes to the daemon that runs, or is answered at once with why none took it, and a daemon
 * starts in the background for the next one (`eventEndpoint`), as when the daemon is gone at the send (`sendFailure`).
 * Each event that fails is written to the hook failure log that `rerail status` reads, as a hook's is (`runHostHook`),
 * but one RERAIL_AUTOSTART=0 kept from a daemon: that is what the person asked for, and `rerail status` says the daemon
 * is not running and why.
 */
function daemonEventLoop(configuration: EngineRuntimeConfiguration & { workspaceRoot: string }, explicitDatabase: string | undefined): RerailEventLoop {
  // The first event may replace a daemon from another package, as hooks do; later ones keep whichever daemon
  // runs, so a stream that outlives `rerail update` follows the newer daemon instead of displacing it.
  let replaceOtherPackage = true;
  let host: RerailHost = "claude-code";
  // The facts of the last event's answer alone: the next event may be another session's, in another checkout.
  let answered: EventAnswerFacts = {};
  return {
    answerFacts: () => answered,
    ingest: async (event) => {
      answered = {};
      // As a hook waits (`withDaemonDeadline`): a busy daemon gets HOOK_DAEMON_WAIT_MS from the event's arrival, the
      // start check's read of it included, though on Windows that read may first wait up to 30 s for a free instance
      // of a busy engine pipe.
      const deadline = Date.now() + HOOK_DAEMON_WAIT_MS;
      if (event.kind === "session-start") host = event.session.host;
      // As hooks do: where Rerail is off nothing is recorded, and asking for the socket would autostart the daemon.
      if (!await hookRerailStarted(host, configuration.workspaceRoot, explicitDatabase)) return [];
      try {
        const endpoint = await eventEndpoint(event, replaceOtherPackage, deadline);
        replaceOtherPackage = false;
        const { eventId: _eventId, deliveries, ...facts } = await ingestEngineEventAnswer(endpoint, event, configuration).catch((error: unknown) => {
          throw sendFailure(error);
        });
        answered = facts;
        return deliveries;
      } catch (error) {
        recordLostEvent(event.session?.host ?? host, event.kind, error);
        throw error;
      }
    },
  };
}

/**
 * The send's failure as a hook reads it (`unreachable`): a daemon gone since the lookup (it crashed, or was stopped)
 * took no event, and a start is launched in the background for the next one, as when the lookup finds none. A socket
 * RERAIL_ENGINE_SOCKET names is not the daemon a start brings up, so none is launched for it, as at its lookup.
 */
function sendFailure(error: unknown): unknown {
  const failure = unreachable(error);
  if (failure instanceof DaemonUnreachableError && process.env.RERAIL_ENGINE_SOCKET === undefined) launchAutostart();
  return failure;
}

/** The engine `event` goes to (`endpointOrWhyNot`); any failure to find one throws `DaemonUnreachableError`, as a hook's lookup does. */
async function eventEndpoint(event: AdapterEvent, replaceOtherPackage: boolean, deadline: number): Promise<EngineEndpoint> {
  try {
    return await endpointOrWhyNot(event, replaceOtherPackage, deadline);
  } catch (error) {
    throw new DaemonUnreachableError(error instanceof Error ? error.message : String(error), { cause: error });
  }
}

/**
 * The socket RERAIL_ENGINE_SOCKET names, else the daemon's. A session's start waits for a daemon to start
 * (`daemonEngineEndpoint`), and every event asks that way under RERAIL_AUTOSTART=0, where no event starts one. Any
 * other event takes the daemon that runs, and starts none (`runningDaemonEngineEndpoint`): it waits on the daemon's
 * record and, for a daemon that is alive but busy, up to `deadline`, never on a start. With no daemon running, a start
 * is launched in the background for the next event (`launchAutostart`) and this one throws at once.
 */
async function endpointOrWhyNot(event: AdapterEvent, replaceOtherPackage: boolean, deadline: number): Promise<EngineEndpoint> {
  const named = process.env.RERAIL_ENGINE_SOCKET;
  if (named !== undefined) return await engineEndpointAt(named, deadline);
  if (event.kind === "session-start" || autostartOff()) return await daemonEngineEndpoint(replaceOtherPackage, deadline);
  const daemon = await runningDaemonEngineEndpoint(deadline, undefined, replaceOtherPackage);
  if (daemon.status === "running") return daemon.endpoint;
  if (daemon.status === "absent") launchAutostart();
  throw new Error(lostEventReason(daemon));
}

/** Writes an event that failed to the hook failure log under the stream's name for it, as `runHostHook` writes a hook's. */
function recordLostEvent(host: RerailHost, event: AdapterEvent["kind"], error: unknown): void {
  if (!autostartTurnedOff(error)) recordHookFailure({ host, event, reason: error instanceof Error ? error.message : String(error) });
}

export async function runRerailEventStream(options: RunRerailEventStreamOptions = {}): Promise<void> {
  const workspaceRoot = resolve(options.workspaceRoot ?? process.cwd());
  // Absolute paths: in a main build the daemon, running elsewhere, opens them.
  const configuration = {
    ...resolveSessionPaths(workspaceRoot, options),
    databasePath: resolve(options.databasePath ?? defaultRerailDatabasePath()),
    workspaceRoot,
  };
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  if (isMainRerailBuild()) {
    // The main daemon is the machine's one main instance; an in-process runtime here would be a second one.
    const explicitDatabase = options.databasePath === undefined ? undefined : configuration.databasePath;
    await serveRerailEventStream({ eventLoop: daemonEventLoop(configuration, explicitDatabase), input, output, workspaceRoot });
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
