import type { SpawnOptions } from "node:child_process";
import { createHash, timingSafeEqual } from "node:crypto";
import { closeSync, existsSync, openSync, readFileSync, rmSync, statSync, utimesSync, writeFileSync, writeSync } from "node:fs";
import { mkdir, open, readFile, realpath, rm, stat } from "node:fs/promises";
import { Agent as HttpsAgent, request as httpsRequest, type RequestOptions as HttpsRequestOptions } from "node:https";
import { connect } from "node:net";
import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { Readable, type Duplex } from "node:stream";
import { connect as tlsConnect, type ConnectionOptions } from "node:tls";
import { setTimeout as sleep } from "node:timers/promises";
import semver from "semver";
import { spawn, spawnShown } from "../engine/child-process.js";
import { programOnPath, windowsSystemProgram } from "../engine/system-programs.js";
import { loadNativeBinding } from "../engine/native/native.js";
import {
  forgetHomeDaemonHolder,
  homeDaemonHolder,
  homeDaemonHolderFile,
  homeDaemonHolderWrittenAt,
  homeDaemonLockHeld,
  homeDaemonStarting,
} from "../engine/native/home-daemon-lock.js";
import { forgetMainInstanceHolder, mainInstanceLockHeld } from "../engine/native/main-instance.js";
import { withHomeLock } from "../engine/native/file-lock.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../engine/package-root.js";
import { ensureSqlite, type SqliteRepairResult } from "./sqlite-repair.js";
import { BUSY_DAEMON_WAIT_MS, HOOK_DAEMON_WAIT_MS, PROBE_TIMEOUT_MS, START_TIMEOUT_MS } from "./daemon/daemon-waits.js";
import { engineCapabilityProof, type EngineProofAnswer, type EngineProofOptions } from "./daemon/engine-proof.js";
import { untakenFor } from "../engine/storage/untaken-events.js";
import { databasePipeListed, enginePipeListed, isNamedPipe, listPipes, pipeListed, pipeNameOf, pipeServer, type PipeServer, type PipeServerAnswer } from "./daemon/engine-pipe.js";
import {
  endProcessStartedBy,
  endProcessStartedByPowerShell,
  processStartedAt,
  processStartedAtThroughEngine,
  processState,
  START_CLOCK_SLACK_MS,
  startNamesProcess,
  type ProcessEnd,
  type ProcessSignal,
} from "../engine/process-liveness.js";
import {
  DAEMON_ACCESS_TOKEN_FILE,
  DAEMON_LOG_FILE,
  DAEMON_STATE_FILE,
  defaultRerailHome,
  RERAIL_DATABASE_FILE,
} from "../engine/storage/rerail-home.js";
import { keepFolderPrivate, makeRerailHome } from "../engine/storage/private-files.js";
import type { ModelStatus } from "../engine/native/model-status.js";
import type { ExperimentalFeature } from "../engine/experimental-features.js";

/** What `rerail status` and the launcher need to find a running daemon. */
export interface DaemonState {
  pid: number;
  host: string;
  port: number;
  url: string;
  version: string;
  /** The Rerail database: rules and every repository's history. */
  databasePath: string;
  /** Local IPC endpoint used by host adapters: a Unix socket or Windows named pipe. */
  kernelSocketPath: string;
  /**
   * `bundled` serves the packaged page from `dist/ui`; `dev` is a daemon started with `--dev`
   * (or `RERAIL_DEV=1`), serving the committed `ui/` source, or one embedded without any page.
   */
  ui: "bundled" | "dev";
  startedAt: number;
  /** The package the daemon was launched from; `rerail setup` restarts a daemon whose root is no longer `current`. */
  packageRoot: string;
  workspaceRoot?: string;
  /** Same-user capability for normal native sessions; never exposed by health/API responses. */
  nativeToken?: string;
  /**
   * The capability every request must carry: kept in the home across restarts for a loopback daemon, minted on every
   * start for one reachable off loopback. Absent only in a state file an older daemon wrote. Never exposed by
   * health/API responses.
   */
  accessToken?: string;
  /**
   * The SHA-256 fingerprint (`AA:BB:…`, as `X509Certificate.fingerprint256` writes it) of the certificate a daemon
   * started with `--tls-cert` presents; present exactly when `url` is `https:`. Rerail's own clients accept that
   * certificate and no other (see {@link daemonFetch}), so a self-signed one needs no CA.
   */
  tlsFingerprint256?: string;
  /**
   * Why a daemon started with a TLS key serves this computer alone, over plain http on loopback: other accounts can read
   * the key, and this names it and the command that fixes it; or the key is empty or no key, and this names it and what
   * it must hold. Absent for every other daemon.
   */
  remoteRefused?: string;
  /**
   * The id of the home's untaken folder (`untakenFolderId`) the daemon watches, taking each event hooks leave there as
   * soon as its event loop runs (`untaken-events.ts`). Only for an event left in that very folder does a hook skip its
   * wait. Absent in a state file an older daemon wrote, or while no watch runs.
   */
  untakenFolder?: string;
  /**
   * True where the daemon's `GET /api/presentation` takes `spelling`, the folder as a Windows host spelt it. Absent in
   * a state file an older daemon wrote: that one refuses a query value it does not know, so it is not sent one.
   */
  readsSpelling?: boolean;
}

/** Header a Rerail process, and the dashboard page, send the daemon's {@link DaemonState.accessToken} in. */
export const DAEMON_ACCESS_HEADER = "x-rerail-access";
/** The query parameter of the dashboard link; the page keeps its value in its origin's storage and sends the header. */
export const DAEMON_ACCESS_PARAM = "rerail-access";

/** The headers that admit a request to the daemon `state` records: its access capability, when it has one. */
export function daemonAccessHeaders(state: Pick<DaemonState, "accessToken">): Record<string, string> {
  return state.accessToken === undefined ? {} : { [DAEMON_ACCESS_HEADER]: state.accessToken };
}

/** Whether `presented` is `token`, in time independent of where they differ. */
export function sameSecret(presented: string | undefined, token: string): boolean {
  if (presented === undefined) return false;
  const digest = (text: string) => createHash("sha256").update(text).digest();
  return timingSafeEqual(digest(presented), digest(token));
}

/** Where a hook sends events: the daemon's engine socket and the capability that socket requires. */
export interface EngineEndpoint {
  socketPath: string;
  accessToken?: string;
  /**
   * How long, in ms, the engine took to answer a proof asked before the event (one that shows the Windows pipe
   * `RERAIL_ENGINE_SOCKET` names to be the home's own daemon, {@link engineEndpointAt}): the event then waits what the
   * proof left of {@link ENGINE_ANSWER_WAIT_MS}, from its own request, however long the hook waited between the two
   * (its session's lock). Absent, the event waits that long from its own request.
   */
  proofTookMs?: number;
}

function engineEndpointOf(state: Pick<DaemonState, "kernelSocketPath" | "accessToken">): EngineEndpoint {
  return state.accessToken === undefined ? { socketPath: state.kernelSocketPath } : { socketPath: state.kernelSocketPath, accessToken: state.accessToken };
}

/**
 * How long a hook waits for the engine to answer: the event's own request (`ingestEngineEvent`), a proof asked first that
 * the Windows pipe `RERAIL_ENGINE_SOCKET` names is the home's own daemon ({@link engineEndpointAt}) included.
 */
export const ENGINE_ANSWER_WAIT_MS = 30_000;

/**
 * The endpoint for an engine socket named outright (`RERAIL_ENGINE_SOCKET`, or a test's path). On macOS and Linux it
 * carries the capability of the home's daemon when its state names that same socket, and none otherwise, which only a
 * standalone engine service (one no daemon started) accepts. On Windows any local user can create a pipe of any name, so
 * the engine there is sent nothing until it shows it is the home's daemon ({@link provenEnginePipe}). `deadline` is the
 * hook's ({@link HOOK_DAEMON_WAIT_MS} from the event's arrival): a proof the pipe gave since then answers for it.
 */
export async function engineEndpointAt(socketPath: string, deadline?: number, machine: DaemonMachine = THIS_MACHINE): Promise<EngineEndpoint> {
  const home = defaultRerailHome(machine.env);
  const state = await readDaemonState(join(home, DAEMON_STATE_FILE), machine.platform).catch(() => undefined);
  if (machine.platform === "win32") return await provenEnginePipe({ socketPath, home, state, machine }, deadline);
  return state !== undefined && resolve(state.kernelSocketPath) === resolve(socketPath) ? engineEndpointOf(state) : { socketPath };
}

/** The Windows engine pipe `RERAIL_ENGINE_SOCKET` names, and what decides whether it is the home's own daemon. */
interface NamedPipe {
  socketPath: string;
  /** The Rerail home whose daemon.json records the capability. */
  home: string;
  /** That daemon.json, where it could be read. */
  state: DaemonState | undefined;
  machine: DaemonMachine;
}

const NOTHING_SENT = "so nothing was sent to it";

/**
 * The endpoint for the Windows engine pipe `RERAIL_ENGINE_SOCKET` names. A hook sends its event there, with the
 * capability daemon.json records, only once the pipe shows it is the home's own daemon: Windows says the process
 * holding the pipe's server end is the daemon daemon.json records ({@link heldByRecordedDaemon}), which wrote that
 * capability there itself, as a daemon of any Rerail version is, one before 0.1.1 that records none included; or else,
 * only where Windows says no pipe of that name exists or could not be asked, the engine on the pipe proves it holds
 * that capability ({@link provedEndpoint}). A holder Windows names that is not that daemon, and a pipe that refuses to
 * let Windows name its holder, are sent nothing, not even the proof's challenge: the server of a pipe can act as the
 * account of a client that writes to it, since Node opens pipes without `SECURITY_SQOS_PRESENT`. A proof the pipe gave this process for the same capability since the event arrived
 * ({@link provedSinceArrival}) answers for both. Windows' wait for a free instance of the pipe stands where the event's
 * own wait to connect would, so the event keeps its whole wait from its request.
 */
async function provenEnginePipe(pipe: NamedPipe, deadline: number | undefined): Promise<EngineEndpoint> {
  const { socketPath, machine } = pipe;
  const state = recordedState(pipe);
  const recorded = engineEndpointOf({ ...state, kernelSocketPath: socketPath });
  if (provedSinceArrival(pipe, state.accessToken, deadline)) return recorded;
  const holder = await machine.pipeServer(socketPath, ENGINE_ANSWER_WAIT_MS);
  if (typeof holder === "string") return await provedEndpoint(pipe, state, holder);
  if (heldByRecordedDaemon(holder, state)) return recorded;
  throw new Error(heldElsewhereReason(pipe, holder, state));
}

/**
 * The endpoint for the Windows engine pipe `pipe` once Windows named no holder (`unnamed`: it says no pipe of that name
 * exists, or could not be asked): the engine on it must prove it holds the capability daemon.json (`state`) records,
 * whichever pipe daemon.json names, and the event then waits what the proof left of {@link ENGINE_ANSWER_WAIT_MS}
 * ({@link EngineEndpoint.proofTookMs}). An engine that does not prove it gets nothing, nor does any engine when
 * daemon.json records no capability: the error says why ({@link notSentReason}).
 */
async function provedEndpoint(pipe: NamedPipe, state: DaemonState, unnamed: Exclude<PipeServerAnswer, PipeServer>): Promise<EngineEndpoint> {
  const { socketPath, machine } = pipe;
  const { accessToken } = state;
  if (accessToken === undefined) throw new Error(notSentReason(pipe, unnamed, undefined));
  const asked = Date.now();
  const answer = await machine.capabilityProof(socketPath, accessToken, ENGINE_ANSWER_WAIT_MS);
  if (answer === "proved") return { socketPath, accessToken, proofTookMs: Date.now() - asked };
  throw new Error(notSentReason(pipe, unnamed, answer));
}

/**
 * daemon.json, which says what the engine on `pipe` must be; throws where there is none, saying the daemon is not
 * running when nothing listens on the pipe either, as after `rerail daemon stop`, which removes daemon.json.
 */
function recordedState({ socketPath, home, state, machine }: NamedPipe): DaemonState {
  if (state !== undefined) return state;
  if (!pipeListed(socketPath, () => machine.pipes())) throw new Error(nothingListens(socketPath));
  throw new Error(`no daemon.json in ${home} says which engine the pipe ${socketPath} must be, ${NOTHING_SENT}`);
}

/** Why a hook sent nothing to the engine pipe `socketPath` when no pipe of that name exists. */
function nothingListens(socketPath: string): string {
  return `nothing listens on the engine pipe ${socketPath}: the Rerail daemon is not running, so this event was not recorded. Start it with rerail daemon start`;
}

/** When each engine pipe last proved to this process that it holds a capability, epoch ms, by machine ({@link proofKey}). */
const provedAt = new WeakMap<DaemonMachine, Map<string, number>>();

/** A pipe's name, whatever its case, as Windows reads it, and the capability it proved it holds. */
function proofKey(socketPath: string, accessToken: string): string {
  return `${socketPath.toLowerCase()}\n${accessToken}`;
}

/** Notes that the engine on `socketPath` proved to this process now that it holds `accessToken`. */
function noteProof(machine: DaemonMachine, socketPath: string, accessToken: string): void {
  const proofs = provedAt.get(machine) ?? new Map<string, number>();
  proofs.set(proofKey(socketPath, accessToken), Date.now());
  provedAt.set(machine, proofs);
}

/**
 * Whether the engine on `pipe` proved to this process that it holds `accessToken` since the event arrived: `deadline`,
 * the hook's, less {@link HOOK_DAEMON_WAIT_MS}. A hook's start check asks the pipe daemon.json names for that proof
 * ({@link windowsRecordedDaemon}) before the hook looks up its endpoint, so a hook whose variable names that pipe asks
 * for one proof, as a hook asked before the named pipe was asked at all.
 */
function provedSinceArrival({ socketPath, machine }: NamedPipe, accessToken: string | undefined, deadline: number | undefined): boolean {
  if (accessToken === undefined || deadline === undefined) return false;
  const at = provedAt.get(machine)?.get(proofKey(socketPath, accessToken));
  return at !== undefined && at >= deadline - HOOK_DAEMON_WAIT_MS;
}

/**
 * Whether Windows names the daemon `state` records as the holder of the pipe's server end (`holder`): its pid, in a
 * process that started no later than the daemon recorded its start. A process keeps its pid for life, so one that started
 * later took a dead daemon's pid; a start Windows does not say counts as later.
 */
function heldByRecordedDaemon(holder: PipeServer, state: DaemonState): boolean {
  return holder.pid === state.pid && (holder.startedAt ?? Number.POSITIVE_INFINITY) <= state.startedAt;
}

/**
 * Why the engine on the Windows pipe `RERAIL_ENGINE_SOCKET` names was sent nothing once Windows answered for it
 * (`holder`) but did not name the daemon daemon.json (`state`) records: the pipe is there but Windows refused the
 * lookup's open or would not name its holder, other clients kept every instance taken past the wait, so Windows could
 * not be asked, or it names another process, which may have taken that daemon's pid.
 */
function heldElsewhereReason({ socketPath, home }: NamedPipe, holder: PipeServer, state: DaemonState): string {
  if (typeof holder.windowsError === "number") return `the engine pipe ${socketPath} is there, but Windows refused to say which process holds it (Windows error ${holder.windowsError}), so it could not be shown to be this Rerail home's daemon, ${NOTHING_SENT}`;
  if (typeof holder.pid !== "number") return `other clients kept every instance of the engine pipe ${socketPath} taken for ${ENGINE_ANSWER_WAIT_MS / 1_000} s, so Windows could not say whether this Rerail home's daemon holds it, ${NOTHING_SENT}`;
  const file = join(home, DAEMON_STATE_FILE);
  const which = holder.pid === state.pid ? `which Windows does not show to have started before ${file} recorded the daemon's start` : `and ${file} records process ${state.pid}`;
  return `the engine pipe ${socketPath} is not this Rerail home's daemon: Windows names process ${holder.pid} as its holder, ${which}, ${NOTHING_SENT}`;
}

/** Why Windows named no holder of a pipe ({@link PipeServerAnswer}), as a hook's error says it. */
const UNNAMED_HOLDER: Readonly<Record<Exclude<PipeServerAnswer, PipeServer>, string>> = {
  "no-pipe": "Windows named no process as its holder",
  "not-asked": "Windows could not be asked which process holds it, since the engine's native binding would not load in this process, predates that question or failed while asking",
};

/**
 * Why the engine on the Windows pipe `RERAIL_ENGINE_SOCKET` names was sent nothing once Windows named no holder
 * (`unnamed`) and no proof (`answer`, undefined where none was asked) showed it to be the home's own daemon: nothing
 * listens on the pipe, or else it could not be shown to be that daemon, and why: daemon.json records no capability to
 * prove, its engine said nothing within the wait, or it did not prove it. The home's own daemon of Rerail 0.1.4 or
 * earlier, which gives no proof, is one of those, so the error never says the pipe is another process's.
 */
function notSentReason({ socketPath, home, machine }: NamedPipe, unnamed: Exclude<PipeServerAnswer, PipeServer>, answer: EngineProofAnswer | undefined): string {
  if (!pipeListed(socketPath, () => machine.pipes())) return nothingListens(socketPath);
  const file = join(home, DAEMON_STATE_FILE);
  const unproved = answer === undefined ? `${file} records no capability for the engine to prove`
    : `the engine on it did not prove ${answer === "silent" ? `within ${ENGINE_ANSWER_WAIT_MS / 1_000} s that ` : ""}it holds the capability in ${file}`;
  return `the engine pipe ${socketPath} could not be shown to be this Rerail home's daemon: ${UNNAMED_HOLDER[unnamed]}, and ${unproved}, ${NOTHING_SENT}`;
}

/** How a Rerail process sends one request to the daemon: `fetch`'s shape, over the transport the daemon serves. */
export type DaemonFetch = (endpoint: URL, init: RequestInit) => Promise<Response>;

/** Where a Rerail process sends requests to a daemon, the headers that admit them, and the transport that reaches it. */
export interface DaemonEndpoint {
  url: URL;
  headers: Record<string, string>;
  fetch: DaemonFetch;
}

export function daemonEndpoint(state: Pick<DaemonState, "url" | "accessToken" | "tlsFingerprint256">, path = "/"): DaemonEndpoint {
  return { url: new URL(path, state.url), headers: daemonAccessHeaders(state), fetch: daemonFetch(state) };
}

/**
 * An https agent that hands the request a TLS connection only once the daemon has presented the certificate whose
 * fingerprint is pinned: the CA chain and the name are not checked (a self-signed certificate for `0.0.0.0` is the
 * usual case), the exact certificate is. Nothing, the capability included, is written before the check passes.
 */
class PinnedDaemonAgent extends HttpsAgent {
  readonly #fingerprint256: string;

  constructor(fingerprint256: string) {
    super({ keepAlive: false });
    this.#fingerprint256 = fingerprint256;
  }

  override createConnection(options: HttpsRequestOptions, callback?: (error: Error | null, stream: Duplex) => void): undefined {
    const socket = tlsConnect({ ...options, rejectUnauthorized: false } as ConnectionOptions);
    const fail = (error: Error): void => {
      socket.destroy();
      callback?.(error, socket);
    };
    socket.once("error", fail);
    socket.once("secureConnect", () => {
      socket.off("error", fail);
      const presented = socket.getPeerCertificate().fingerprint256;
      if (presented !== this.#fingerprint256) {
        fail(new Error(`Rerail daemon presented TLS certificate ${presented}, not the ${this.#fingerprint256} daemon.json pins; nothing was sent`));
        return;
      }
      callback?.(null, socket);
    });
    return undefined;
  }
}

/** Statuses whose response has no body, which `Response` refuses one for. */
const NULL_BODY_STATUSES: ReadonlySet<number> = new Set([101, 103, 204, 205, 304]);

function requestBody(body: RequestInit["body"]): string | Uint8Array | undefined {
  if (body === undefined || body === null || typeof body === "string" || body instanceof Uint8Array) return body ?? undefined;
  throw new TypeError("A request to an https Rerail daemon carries a string or bytes body");
}

/** A Node response's headers as `fetch` gives them: each value of a header that repeats appended in turn. */
function fetchHeaders(incoming: Readonly<Record<string, string | string[] | undefined>>): Headers {
  const headers = new Headers();
  for (const [name, value] of Object.entries(incoming)) {
    for (const item of [value ?? []].flat()) headers.append(name, item);
  }
  return headers;
}

/** `fetch` over `agent`, for the request shapes Rerail's own clients send. Redirects are never followed. */
function pinnedFetch(agent: PinnedDaemonAgent): DaemonFetch {
  return async (endpoint, init) => {
    const method = (init.method ?? "GET").toUpperCase();
    const body = requestBody(init.body);
    const { promise, resolve: answer, reject } = Promise.withResolvers<Response>();
    const request = httpsRequest(endpoint, {
      method,
      headers: Object.fromEntries(new Headers(init.headers)),
      agent,
      ...(init.signal === undefined || init.signal === null ? {} : { signal: init.signal }),
    }, (response) => {
      const status = response.statusCode ?? 502;
      const empty = method === "HEAD" || NULL_BODY_STATUSES.has(status);
      if (empty) response.resume();
      answer(new Response(empty ? null : Readable.toWeb(response) as unknown as ReadableStream, {
        status, statusText: response.statusMessage ?? "", headers: fetchHeaders(response.headers),
      }));
    });
    request.once("error", reject);
    request.end(body);
    return await promise;
  };
}

/**
 * How a Rerail process reaches the daemon `state` records: plain `fetch` for an `http:` daemon; for an `https:` one, a
 * client that pins the certificate fingerprint the daemon recorded. An `https:` state without a fingerprint is refused
 * rather than trusted.
 */
export function daemonFetch(state: Pick<DaemonState, "url" | "tlsFingerprint256">): DaemonFetch {
  if (new URL(state.url).protocol !== "https:") return fetch;
  const fingerprint = state.tlsFingerprint256;
  if (fingerprint === undefined) {
    return async () => {
      throw new Error("daemon.json names an https Rerail daemon without the certificate fingerprint to pin");
    };
  }
  return pinnedFetch(new PinnedDaemonAgent(fingerprint));
}

/** `url` on the origin a browser on this machine reaches: an unspecified bind address (`0.0.0.0`, `::`) is loopback. */
function browserReachable(url: string): URL {
  const reachable = new URL(url);
  if (reachable.hostname === "0.0.0.0" || reachable.hostname === "[::]") reachable.hostname = "127.0.0.1";
  return reachable;
}

/**
 * `page` (a URL on the daemon `state` records; its root by default) as the link a person opens in a browser: on the
 * origin the browser reaches, with the capability the page keeps and sends. A page of another origin is returned as it
 * is. Only for a person's own browser or terminal; a link handed to a coding agent never carries it.
 */
export function dashboardLink(state: Pick<DaemonState, "url" | "accessToken">, page: string = state.url): string {
  const link = browserReachable(page);
  if (state.accessToken === undefined || link.origin !== browserReachable(state.url).origin) return page;
  link.searchParams.set(DAEMON_ACCESS_PARAM, state.accessToken);
  return link.href;
}

/** Whether the daemon `state` records keeps its capability in `home` across restarts; false when that cannot be read. */
export function accessTokenKept(home: string, state: Pick<DaemonState, "accessToken">): boolean {
  try {
    return state.accessToken !== undefined && readFileSync(join(home, DAEMON_ACCESS_TOKEN_FILE), "utf8").trim() === state.accessToken;
  } catch {
    return false;
  }
}

export type NativeStatus = { ok: true; kernelVersion: string } | { ok: false; error: string };

export interface DaemonHealth extends Pick<DaemonState, "version" | "pid" | "ui"> {
  status: "ok";
  uptimeMs: number;
  database: { path: string; exists: boolean };
  native: NativeStatus;
  /** The bundled model against `model/manifest.json` (`engine/native/model-status.ts`). */
  model: ModelStatus;
  /** Whether the engine and the model can serve sessions (`native.ok && model.ok`); `status` says only that the daemon answers. */
  healthy: boolean;
  /** Absolute checkout path native sessions address. */
  workspaceRoot: string;
  /** When Rerail was installed on this machine (epoch ms); null before the database exists. */
  installedAt: number | null;
  /** The experimental features this daemon runs (docs/internal-testing.md); the dashboard shows only theirs. */
  experiments: ExperimentalFeature[];
  /**
   * The newest Rerail on the update channel and when npm said so (epoch ms), from the daily update check
   * (`setup/update-check.ts`); null before its first answer, and always for a daemon that does not check.
   */
  update: { latest: string; checkedAt: number } | null;
}

/**
 * Rerail CLI children require Node; a compiled Bun host's execPath may launch the host itself. There the name
 * is resolved through PATH once, so the SQLite probe, its rebuild and the daemon all run under one binary. On Windows
 * only a `node.exe` counts: a version manager's `node.cmd` shim cannot be spawned without a shell, and a bare `node`
 * would be looked for in the current folder first.
 */
export function nodeExecutable(platform: NodeJS.Platform = process.platform): string {
  if (process.versions.bun === undefined) return process.execPath;
  const node = programOnPath("node", { platform });
  if (node !== undefined) return node;
  if (platform !== "win32") return "node";
  throw new Error("Rerail runs its CLI under Node.js, and no node.exe is on PATH");
}

export { DAEMON_ACCESS_TOKEN_FILE, DAEMON_LOG_FILE, DAEMON_STATE_FILE };
/** The lock a daemon start holds, `locks/start.lock` in the Rerail home. */
const START_LOCK = "start";
/**
 * In the Rerail home: the folder Rerail before `locks/` held as the start lock. Uninstall keeps it with the home and
 * names it there as one of Rerail's own entries (`RERAIL_HOME_ENTRIES` in setup-cli.ts).
 */
export const START_LOCK_DIR = "start.lock";
/** In the Rerail home: the claim of the one autostart launch in progress, holding its launcher's pid (`claimAutostart`). */
export const AUTOSTART_CLAIM_FILE = "autostart.claim";

/**
 * The longest a launcher keeps its autostart claim (`keepAutostartClaim`): each of its start's waits of
 * {@link START_TIMEOUT_MS} in turn: for the start lock, for a daemon that holds the home to record itself
 * (`lookupHomeDaemon`), and for the daemon it launched or found still in its start. A start that found one and looks
 * again once it answers can wait longer; past this the claim lapses, and a hook that then finds the daemon down starts
 * a second launcher, which waits for the start lock or for the daemon in its start, as the first does. A launcher that
 * hangs past it no longer keeps the processes that find the daemon down from launching.
 */
const AUTOSTART_CLAIM_MS = 3 * START_TIMEOUT_MS;
/**
 * How long an autostart claim holds once nothing renews it, even while the pid it names is alive: a launcher renews it
 * every {@link AUTOSTART_CLAIM_RENEW_MS} while it runs, so past this its launcher was killed or hung, and the pid can be
 * another process the system gave a dead launcher's pid to. It also covers a launcher's start, from the claim naming it
 * to its first renewal; a launch slower than that lets the next process to find the daemon down launch too, and the
 * start lock still lets one start a daemon.
 */
const AUTOSTART_CLAIM_LEASE_MS = 5_000;
const AUTOSTART_CLAIM_RENEW_MS = 1_000;
const STOP_TIMEOUT_MS = 10_000;
const KILL_TIMEOUT_MS = 5_000;
const POLL_INTERVAL_MS = 100;
const MAX_MACOS_UNIX_SOCKET_PATH_BYTES = 103;

export interface RunningDaemon {
  state: DaemonState;
  health: DaemonHealth;
}

/**
 * A daemon of a release before the engine's proof, Rerail 0.1.4 or earlier, as a lookup finds it on Windows by its
 * process's start ({@link olderDaemon}). It proved nothing, so it is asked nothing and has no health, and its state
 * holds neither capability: no caller can send it one. It is ended through its process ({@link endOlderDaemon}).
 */
export interface OlderDaemon {
  status: "older";
  state: DaemonState;
}

/** A daemon a lookup found running: one that answered as itself, or one of a release before the proof ({@link OlderDaemon}). */
export type FoundRunning = Extract<DaemonLookup, { status: "running" | "older" }>;

/**
 * What the state file says, checked against reality.
 *
 * - `running`: the recorded pid is alive and `/healthz` answers with that pid.
 * - `older`: a daemon of a release before the engine's proof, found by its process's start on Windows, which is not
 *   asked ({@link OlderDaemon}). A status of its own, so a caller that takes a `running` daemon for one it may ask never
 *   gets this one, and the compiler names each caller that must say what it does with it.
 * - `unresponsive`: the pid is alive but the probe failed. The state is kept:
 *   a transient timeout must not orphan a live daemon, and `stop` still needs
 *   the pid. `start` refuses rather than spawning a second instance.
 * - `refused`: the listener answered `401`: it does not accept the capability the state file records (edited, or
 *   written by another daemon). It is running; `start` refuses and says to restart it.
 * - `absent`: no state, or a stale file (pid dead, or a different process
 *   answering on that URL) which has been cleared. `notDaemon` is the live pid a cleared file named that is not the
 *   home's daemon ({@link recordedProcess}).
 */
export type DaemonLookup =
  | ({ status: "running" } & RunningDaemon)
  | OlderDaemon
  | { status: "unresponsive"; state: DaemonState }
  | { status: "refused"; state: DaemonState }
  | { status: "absent"; notDaemon?: number };

/** What the daemon at a recorded URL answered a health probe: its health, a refusal of the capability, or nothing usable. */
export type DaemonProbe = { status: "ok"; health: DaemonHealth } | { status: "refused" } | { status: "unreachable" };

/** Why `start` will not use, or replace, a recorded daemon that is alive: what to tell the person. */
function liveDaemonProblem(lookup: Extract<DaemonLookup, { status: "unresponsive" | "refused" }>): string {
  return lookup.status === "refused"
    ? `Rerail daemon pid ${lookup.state.pid} at ${lookup.state.url} refused the access capability in daemon.json (401); run rerail daemon stop, then rerail daemon start.`
    : `Rerail daemon pid ${lookup.state.pid} is alive but not answering at ${lookup.state.url}; run rerail daemon stop, then rerail daemon start.`;
}

/** `start` found a recorded daemon alive that it will neither use nor replace; `lookup` says which way it failed. */
export class LiveDaemonError extends Error {
  override readonly name = "LiveDaemonError";
  readonly lookup: Extract<DaemonLookup, { status: "unresponsive" | "refused" }>;

  constructor(lookup: Extract<DaemonLookup, { status: "unresponsive" | "refused" }>) {
    super(liveDaemonProblem(lookup));
    this.lookup = lookup;
  }
}

export interface DaemonLaunchOptions {
  dev: boolean;
  host?: string;
  /** Required for a `host` off loopback; see `assertHostBindable`. */
  allowRemote?: boolean;
  /** Extra hostnames the daemon answers to; passed through as `--public-host`. */
  publicHosts?: readonly string[];
  /** PEM files the daemon serves https with; passed through as `--tls-cert` / `--tls-key`. */
  tls?: DaemonTlsFiles;
  /** Serve plain http off loopback anyway; passed through as `--insecure-http`. See `parseDaemonTransport`. */
  insecureHttp?: boolean;
  port?: number;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
}

/** The certificate (chain) and private key a daemon serves https with, as absolute paths to PEM files. */
export interface DaemonTlsFiles {
  certFile: string;
  keyFile: string;
}

export interface StartDaemonResult {
  /** `false` when a daemon was already running and has been reused. */
  started: boolean;
  daemon: RunningDaemon;
  /** The daemon of a release before the engine's proof this start ended through its process first ({@link OlderDaemon}). */
  replaced?: Pick<DaemonState, "pid" | "version">;
}

/** Whether `pid` is a live process of any user: a process another user runs still holds its pid (`EPERM`). */
export function processAlive(pid: number, kill?: ProcessSignal): boolean {
  return processState(pid, kill) !== "gone";
}

const isString = (value: unknown): boolean => typeof value === "string";
const isOptionalString = (value: unknown): boolean => value === undefined || typeof value === "string";

/**
 * A finite number above 0, as a pid and a start time must be to name a process. Signal 0 answers for a pid of 0 or
 * below, which POSIX `kill` reads as a process group and libuv's Windows `uv_kill` reads 0 as the caller, so such a
 * record would read as a live daemon. A `startedAt` of Infinity, which `JSON.parse` makes of 1e400, makes no record. A
 * finite one past the reader's clock still makes one, as a clock set back since the daemon stamped it leaves it, but
 * proves nothing of the process at its pid ({@link startVerdict}).
 */
const isPositiveFinite = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value) && value > 0;

/** The check each field of daemon.json passes; `kernelSocketPath` is checked for the platform (`isDaemonState`). */
const DAEMON_STATE_FIELDS: Readonly<Record<string, (value: unknown) => boolean>> = {
  pid: value => isPositiveFinite(value) && Number.isInteger(value),
  url: isString,
  host: isString,
  port: Number.isInteger,
  version: isString,
  ui: value => value === "bundled" || value === "dev",
  nativeToken: isOptionalString,
  accessToken: isOptionalString,
  tlsFingerprint256: isOptionalString,
  remoteRefused: isOptionalString,
  startedAt: isPositiveFinite,
  packageRoot: isString,
  workspaceRoot: isOptionalString,
};

function hasDaemonStateFields(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return Object.entries(DAEMON_STATE_FIELDS).every(([field, check]) => check(record[field]));
}

/** The longest Unix socket path, in bytes, where Rerail checks it before a bind fails on it. */
const MAX_UNIX_SOCKET_PATH_BYTES: Partial<Record<NodeJS.Platform, number>> = { darwin: MAX_MACOS_UNIX_SOCKET_PATH_BYTES };

/** Whether `path` is a named pipe's path with one name after `\\.\pipe\`, and nothing past it. */
function isSinglePipePath(path: string): boolean {
  const name = isNamedPipe(path) ? pipeNameOf(path) : "";
  return name.length > 0 && !name.includes("\\");
}

export function isUsableEngineSocketPath(
  value: unknown,
  platform: NodeJS.Platform = process.platform,
): value is string {
  if (typeof value !== "string" || value.length <= 1 || value.includes("\0")) return false;
  if (platform === "win32") return isSinglePipePath(value);
  return isAbsolute(value) && Buffer.byteLength(value) <= (MAX_UNIX_SOCKET_PATH_BYTES[platform] ?? Number.POSITIVE_INFINITY);
}

function isDaemonState(value: unknown, platform: NodeJS.Platform): value is DaemonState {
  return hasDaemonStateFields(value) && isUsableEngineSocketPath(value.kernelSocketPath, platform);
}

export async function readDaemonState(stateFile: string, platform: NodeJS.Platform = process.platform): Promise<DaemonState | undefined> {
  let text: string;
  try {
    text = await readFile(stateFile, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
  try {
    const value: unknown = JSON.parse(text);
    return isDaemonState(value, platform) ? value : undefined;
  } catch {
    return undefined;
  }
}

/** How {@link probeDaemon} asks the daemon. */
export interface DaemonProbeOptions {
  /** Another transport than the daemon's own (`daemonFetch`), as the MCP bridge's tests pass. */
  readonly fetchImpl?: DaemonFetch | undefined;
  /** How long it waits for the answer; a caller that can wait out a busy daemon passes more. */
  readonly timeoutMs?: number;
  /** Ends the wait in place of `timeoutMs`: a caller whose whole call has one bound passes that bound's signal. */
  readonly signal?: AbortSignal | undefined;
}

/**
 * What the daemon `state` records answers on `/healthz`, with the capability it records, over its own transport
 * unless `fetchImpl` names another. A `401` is a refusal, kept apart from silence: the daemon is up, and "not
 * answering" would send the person after the wrong cause.
 */
export async function probeDaemon(
  state: Pick<DaemonState, "url" | "accessToken" | "tlsFingerprint256">,
  { fetchImpl, timeoutMs = PROBE_TIMEOUT_MS, signal }: DaemonProbeOptions = {},
): Promise<DaemonProbe> {
  try {
    const endpoint = daemonEndpoint(state, "/healthz");
    const response = await (fetchImpl ?? endpoint.fetch)(endpoint.url, { headers: endpoint.headers, signal: signal ?? AbortSignal.timeout(timeoutMs) });
    // Awaited here, so a body that cannot be read is caught below as no answer.
    return await probeAnswer(response);
  } catch {
    return { status: "unreachable" };
  }
}

/** What a daemon's answer on `/healthz` makes of it ({@link probeDaemon}); a body that cannot be read throws. */
async function probeAnswer(response: Response): Promise<DaemonProbe> {
  if (response.status === 401) return { status: "refused" };
  if (!response.ok) return { status: "unreachable" };
  const body = await response.json() as DaemonHealth;
  return body.status === "ok" ? { status: "ok", health: body } : { status: "unreachable" };
}

/** A process Node's spawn started, as daemon-control reads it: its pid, how it ended, and letting this process exit first. */
export interface StartedChild {
  readonly pid?: number | undefined;
  once(event: "error", listener: (error: Error) => void): unknown;
  once(event: "exit", listener: (code: number | null) => void): unknown;
  unref(): void;
}

/** What Windows' native spawn starts: a program, its arguments, its folder (null keeps this process's), and its log. */
export interface NativeLaunch {
  program: string;
  args: string[];
  cwd: string | null;
  env: Record<string, string>;
  logFile: string;
}

/**
 * A process the native spawn started: its pid, and its exit code (null while it runs), read from the handle the native
 * module kept, so a pid Windows has given to another process is no answer. A binary built before that has no `exitCode`.
 */
export interface NativeProcess {
  pid: number;
  exitCode?: () => number | null;
}

/**
 * Everything daemon-control asks of the machine it runs on. Each entry point that reaches the OS takes one,
 * {@link THIS_MACHINE} unless a test passes its own, so a test states the platform it checks and runs on every OS. The
 * helpers that only compute from a platform (`readDaemonState`, `stillRunningMessage`, `browserOpener`) take the
 * machine's.
 */
export interface DaemonMachine {
  platform: NodeJS.Platform;
  /** `process.kill`; signal 0 asks whether a pid is live (`processState`). */
  kill: ProcessSignal;
  /** This process's environment: `RERAIL_HOME`, `RERAIL_AUTOSTART`, `SystemRoot`, and what a launched process inherits. */
  env: NodeJS.ProcessEnv;
  /** The named pipes that exist on Windows, read from `\\.\pipe\` without connecting to any. */
  pipes(): readonly string[];
  /**
   * What the engine on `socketPath` answers when asked, within `timeoutMs`, to prove it holds `accessToken`, which it
   * never sees (`engineCapabilityProof`); with `waitForInstance`, after it waited for a free instance of a busy pipe.
   */
  capabilityProof(socketPath: string, accessToken: string, timeoutMs: number, options?: Pick<EngineProofOptions, "waitForInstance">): Promise<EngineProofAnswer>;
  /**
   * Windows: the process holding the server end of the named pipe `socketPath` and when it started, as Windows records
   * them, opening again while other clients hold every instance until `timeoutMs` has passed, with no pid once it has;
   * `no-pipe` where Windows says no pipe of that name exists and `not-asked` where it could not be asked (`pipeServer`). This
   * process's own thread runs on while it waits.
   */
  pipeServer(socketPath: string, timeoutMs: number): Promise<PipeServerAnswer>;
  /** Removes the main-instance record when it names `pid` (`forgetMainInstanceHolder`). */
  forgetMainInstanceHolder(pid: number): void;
  /**
   * Whether a process holds the main-instance lock in `directory` (`mainInstanceLockHeld`); undefined when the native
   * binding does not load.
   */
  mainInstanceLockHeld(directory: string): boolean | undefined;
  /**
   * Whether a daemon holds the Rerail home `home` (`homeDaemonLockHeld`), its daemon.json or not; undefined when the
   * lock cannot be asked.
   */
  homeDaemonLockHeld(home: string): boolean | undefined;
  /**
   * When the process holding `pid` started, in epoch ms, or undefined where it cannot be read within `timeoutMs`
   * (`processStartedAt`).
   */
  processStartedAt(pid: number, timeoutMs: number): number | undefined;
  /**
   * Windows: ends the process holding `pid` if it started no later than `startedAt` (epoch ms), its start read and its
   * end made through one handle, and waits up to `waitMs` for it to exit (`endProcessStartedBy`).
   */
  endProcessStartedBy(pid: number, startedAt: number, waitMs: number): ProcessEnd;
  /** The Node binary Rerail's own processes run under ({@link nodeExecutable}). */
  nodeExecutable(): string;
  /** Windows: starts a process through the native module's spawn ({@link launchDetached} says why). */
  spawnDetached(launch: NativeLaunch): NativeProcess;
  /** Node's spawn, through `child-process.ts`, which hides its window: the daemon's on macOS and Linux, and Orca's CLI. */
  spawn(command: string, args: readonly string[], options: SpawnOptions): StartedChild;
  /** Starts the browser opener, whose window the person is meant to see (`spawnShown`). */
  spawnShown(command: string, args: readonly string[], options: SpawnOptions): StartedChild;
}

/** Starts `launch` through the native module's spawn; its exit code comes from the handle the module kept, if it keeps one. */
function nativeSpawnDetached(launch: NativeLaunch): NativeProcess {
  const native = loadNativeBinding();
  const pid = native.spawnDetached(launch.program, launch.args, launch.cwd, launch.env, launch.logFile);
  if (native.detachedExitCode === undefined) return { pid };
  return { pid, exitCode: () => native.detachedExitCode?.(pid) ?? null };
}

const THIS_MACHINE: DaemonMachine = {
  platform: process.platform,
  kill: (pid, signal) => process.kill(pid, signal),
  env: process.env,
  pipes: listPipes,
  capabilityProof: engineCapabilityProof,
  pipeServer,
  forgetMainInstanceHolder: pid => forgetMainInstanceHolder(pid),
  mainInstanceLockHeld: directory => mainInstanceLockHeld(directory),
  homeDaemonLockHeld: home => homeDaemonLockHeld(home),
  processStartedAt: pid => processStartedAtThroughEngine(pid),
  endProcessStartedBy: (pid, startedAt, waitMs) => endProcessStartedBy(pid, startedAt, waitMs),
  nodeExecutable: () => nodeExecutable(),
  spawnDetached: nativeSpawnDetached,
  spawn: (command, args, options) => spawn(command, args, options),
  spawnShown,
};

/**
 * This machine as a process that must not load the engine reads it: one that deletes the engine's folder after it
 * stops the daemon, as the uninstall deletes `versions/`, since Windows keeps a loaded DLL from being deleted. It reads
 * a process's start, and ends a daemon of a release before the engine's proof, through Windows PowerShell
 * (`processStartedAt`, `endProcessStartedByPowerShell`), which takes about a second each time. PowerShell starts before
 * it can end the process, so its end gets the time a stop gives a daemon to end, {@link STOP_TIMEOUT_MS}, before the
 * wait for the exit it is given.
 */
export const MACHINE_WITHOUT_ENGINE: DaemonMachine = {
  ...THIS_MACHINE,
  processStartedAt,
  endProcessStartedBy: (pid, startedAt, waitMs) => endProcessStartedByPowerShell(pid, startedAt, STOP_TIMEOUT_MS + waitMs),
};

/**
 * Whether the engine pipe `state` records exists. Windows ends a daemon without its shutdown handler (logoff, reboot,
 * `rerail daemon stop`), so its state file outlives it, and Windows soon hands its pid to another process; a live
 * daemon always holds its pipe, so a live pid without one is not the daemon. Unknown counts as there.
 */
function enginePipeExists(state: DaemonState, machine: DaemonMachine): boolean {
  return pipeListed(state.kernelSocketPath, () => machine.pipes());
}

/**
 * Whether `pid`, recorded by a daemon that may be gone, can still be a Rerail daemon: a live process, and on Windows only
 * while some Rerail engine pipe exists, since every live daemon holds one. Windows ends a daemon without its cleanup at
 * logoff or reboot, so a record it wrote can name a pid Windows has given to another process since. Any Rerail home's
 * pipe counts, so this over-counts while another home's daemon runs: {@link mainInstanceHolderRuns} asks the lock where it can.
 */
export function daemonMayHold(pid: number, machine: DaemonMachine = THIS_MACHINE): boolean {
  if (!processAlive(pid, machine.kill)) return false;
  return machine.platform !== "win32" || enginePipeListed(() => machine.pipes());
}

/**
 * Whether the main instance the record in `directory` names (`pid`) still runs. The pid rule ({@link daemonMayHold})
 * answers first and loads nothing; only where it says the holder may run is the main-instance lock asked, which the OS
 * drops however the holder ends. Never on Windows: asking loads the engine from the running version's folder, Windows
 * keeps a loaded DLL from being deleted, and the uninstall that asks is about to delete that folder. There a live pid
 * with a Rerail engine pipe counts, which at worst keeps a cache another daemon no longer uses. Where the native
 * binding does not load to try the lock, the pid rule's answer stands.
 */
export function mainInstanceHolderRuns(directory: string, pid: number, machine: DaemonMachine = THIS_MACHINE): boolean {
  if (!daemonMayHold(pid, machine)) return false;
  return machine.platform === "win32" || (machine.mainInstanceLockHeld(directory) ?? true);
}

/**
 * What a reader of a state file may make of the daemon it records ({@link recordedDaemon}): it may be this user's and
 * be sent the capability (`alive`), it cannot be (`gone`), or its engine said nothing to the proof in time (`silent`).
 */
export type RecordedDaemon = "alive" | "gone" | "silent";

/**
 * {@link RecordedDaemon} as the engine's proof leaves it, with `no-proof` for a live pid this user may signal whose
 * engine has no proof to give and whose record a release before the proof wrote, Rerail 0.1.4 or earlier
 * ({@link noProofVerdict}). A lookup reads on ({@link lookedUpEngine}); every other reader takes it as `gone`
 * ({@link recordedDaemon}).
 */
type ProofVerdict = RecordedDaemon | "no-proof";

/**
 * {@link RecordedDaemon} as a lookup makes it, with `older` for the daemon that wrote the record, of a release before
 * the engine's proof, Rerail 0.1.4 or earlier, which is sent nothing ({@link olderDaemon}). A hook's start replaces such
 * a daemon whatever package it ran from (`startDaemonForPackage`).
 */
type RecordedEngine = RecordedDaemon | "older";

/** What an engine's answer to the proof makes of the daemon a Windows state records ({@link windowsRecordedDaemon}). */
const PROOF_VERDICT: Readonly<Record<EngineProofAnswer, ProofVerdict>> = { proved: "alive", "no-proof": "no-proof", refused: "gone", silent: "silent" };

/**
 * How long a reader of a state file gives the engine's proof on Windows ({@link recordedDaemon}): its limit, and whether
 * a pipe whose instances are all taken is first waited for until one is free (`EngineProofOptions.waitForInstance`).
 */
type ProofWait = { timeoutMs: number } & Pick<EngineProofOptions, "waitForInstance">;

/**
 * Whether the daemon `state` records may still run, as this user's: the one check every reader of a state file makes
 * before it sends the capability the file records to the URL it records. A state file outlives a daemon killed without
 * its cleanup, and the pid it names can since belong to another process, which would get the capability. The pid must
 * be live, and on macOS and Linux this user's to signal: another user's process cannot be this user's daemon. Windows
 * asks more ({@link windowsRecordedDaemon}).
 */
async function proofVerdict(state: DaemonState, machine: DaemonMachine, wait: ProofWait): Promise<ProofVerdict> {
  const daemon = processState(state.pid, machine.kill);
  if (daemon === "gone") return "gone";
  if (machine.platform === "win32") return await windowsRecordedDaemon(state, daemon === "running", machine, wait);
  return daemon === "running" ? "alive" : "gone";
}

/**
 * {@link proofVerdict} for a reader that only reads the daemon or sends it the capability: each hook's check whether
 * Rerail runs in its repository, the status line, the MCP bridge and a stop's shutdown request. An engine with no proof
 * is sent nothing, as one that refuses the proof, and its process's start is not read: a hook asks this before every
 * tool call, and a start read there loads the engine into the hook's process. Only a lookup tells such a daemon from a
 * stale record, and only a lookup's caller stops or replaces it ({@link lookedUpEngine}).
 */
async function recordedDaemon(state: DaemonState, machine: DaemonMachine, wait: ProofWait): Promise<RecordedDaemon> {
  const recorded = await proofVerdict(state, machine, wait);
  return recorded === "no-proof" ? "gone" : recorded;
}

/**
 * {@link proofVerdict} for a lookup, the proof given `timeoutMs`: the start of the process an engine with no proof runs
 * in, read by `readDeadline`, the lookup's own limit, decides whether it is the daemon that wrote the record
 * ({@link olderDaemon}).
 */
async function lookedUpEngine(state: DaemonState, machine: DaemonMachine, timeoutMs: number, readDeadline: number): Promise<RecordedEngine> {
  const recorded = await proofVerdict(state, machine, { timeoutMs });
  return recorded === "no-proof" ? olderDaemon(state, machine, readDeadline) : recorded;
}

/**
 * {@link recordedDaemon} on Windows, for a live pid this user may signal or not (`signalable`). The state's own engine
 * pipe must exist, since a live daemon always holds it, and the engine on it must prove it holds the capability
 * (`engineCapabilityProof`), which never leaves this process. Windows soon reuses a pid, often for another of this
 * user's processes, and any local user can create the pipe's name, so a signalable pid and a listed pipe alone would
 * hand the capability to whoever holds the pipe and the port; and a pid this user may not signal (`EPERM`) can still be
 * this user's daemon, started from an elevated terminal or agent session. An engine silent for `wait.timeoutMs` is
 * `silent`: a daemon busy in an audit is, and dropping its state for that would lose a daemon that still runs. A state
 * without a capability is sent none, and the pid and pipe decide.
 *
 * An engine of Rerail 0.1.4 or earlier has no proof to give (`no-proof`), nor has a listener without the proof's path
 * on the pipe name of a daemon that is gone or shutting down. A lookup reads on for one only where the record was
 * written by a release before the proof ({@link predatesProof}) and this user may signal its pid, and so stop or
 * replace its daemon ({@link olderDaemon}); any other is `gone`, as before the proof's 404 was told apart: a daemon of
 * a release with the proof always gives it, and while one shuts down its pid lives on after its pipe and port are free.
 */
async function windowsRecordedDaemon(state: DaemonState, signalable: boolean, machine: DaemonMachine, wait: ProofWait): Promise<ProofVerdict> {
  if (!enginePipeExists(state, machine)) return "gone";
  if (state.accessToken === undefined) return signalable ? "alive" : "gone";
  const answer = await machine.capabilityProof(state.kernelSocketPath, state.accessToken, wait.timeoutMs, wait);
  if (answer === "proved") noteProof(machine, state.kernelSocketPath, state.accessToken);
  return answer === "no-proof" ? noProofVerdict(state, signalable) : PROOF_VERDICT[answer];
}

/** What an engine with no proof to give makes of `state`'s daemon: read on only for a signalable pid's pre-proof record. */
function noProofVerdict(state: DaemonState, signalable: boolean): ProofVerdict {
  return signalable && predatesProof(state.version) ? PROOF_VERDICT["no-proof"] : "gone";
}

/** The first release whose engine gives the proof. */
const PROOF_RELEASE = "0.1.5";

/** Whether `version`, as a state records it, is of a release before the engine's proof; one that does not parse is not. */
function predatesProof(version: string): boolean {
  return semver.valid(version) !== null && semver.lt(version, PROOF_RELEASE);
}

/**
 * What a lookup makes of the daemon a Windows state records whose engine has no proof to give and whose pid this user
 * may signal ({@link lookedUpEngine}), from the start of the process holding that pid, read by `deadline`
 * ({@link startVerdict}). A process keeps its pid for life, so one that started no later than the record's `startedAt`
 * is the daemon that wrote it: `older`, which is sent nothing ({@link OlderDaemon}). A process that took a gone
 * daemon's pid, one that started later than `startedAt` by more than {@link START_CLOCK_SLACK_MS}, is `gone`, and the
 * record is dropped. A start not read, or within the slack, proves neither, nor does a `startedAt` past the reader's
 * clock by more than the slack: `silent`, so the record is kept and nothing is sent, as for a busy daemon.
 */
function olderDaemon(state: DaemonState, machine: DaemonMachine, deadline: number): RecordedEngine {
  return START_VERDICT_ENGINE[startVerdict(state, machine, deadline)];
}

/** What {@link olderDaemon} makes of each {@link RecordedProcess}. */
const START_VERDICT_ENGINE: Readonly<Record<RecordedProcess, RecordedEngine>> = { ours: "older", "not-ours": "gone", unknown: "silent" };

/**
 * What may be made now of the daemon `state` records ({@link recordedDaemon}), for a reader that tells a busy daemon
 * (`silent`, sent nothing) from one that is gone. `timeoutMs` bounds the engine's proof on Windows; a caller that waits
 * out a busy daemon passes what is left of its own bound.
 */
export async function recordedDaemonStatus(
  state: DaemonState,
  machine: DaemonMachine = THIS_MACHINE,
  timeoutMs: number = PROBE_TIMEOUT_MS,
): Promise<RecordedDaemon> {
  return await recordedDaemon(state, machine, { timeoutMs });
}

/**
 * {@link recordedDaemonStatus} as a hook asks it before it sends its event. On Windows a pipe whose every instance is
 * taken has a daemon that lives and is busy, so the proof waits for a free instance as main's did, up to 30 s, and then
 * has its 2 s from the connect: the event reaches a daemon that frees one in that time. A command's read keeps its
 * 2 s from the ask. Elsewhere the two are one read.
 */
export async function recordedDaemonStatusForEvent(state: DaemonState, machine: DaemonMachine = THIS_MACHINE): Promise<RecordedDaemon> {
  return await recordedDaemon(state, machine, { timeoutMs: PROBE_TIMEOUT_MS, waitForInstance: true });
}

/**
 * Whether the daemon `state` records may be sent the capability now ({@link recordedDaemon}); one whose engine is
 * silent may not.
 */
export async function recordedDaemonAlive(state: DaemonState, machine: DaemonMachine = THIS_MACHINE): Promise<boolean> {
  return await recordedDaemonStatus(state, machine) === "alive";
}

/**
 * What the facts the OS keeps say of the live process daemon.json names, when it did not answer as the daemon that wrote
 * the record ({@link recordedProcess}):
 * - `ours`: it is that daemon: it holds the home's lock and is the holder recorded beside it, or it started no later
 *   than the record's `startedAt` (a process keeps its pid for life, so the one that ran with that pid then wrote it).
 * - `not-ours`: it cannot be: another process holds the home's lock as its recorded holder and is shown to have
 *   written that record ({@link holderWroteRecord}), or it started after `startedAt` by more than
 *   {@link START_CLOCK_SLACK_MS}, as a process that took a dead daemon's pid did.
 * - `unknown`: nothing decides: no lock held, no holder record, a lock that cannot be asked, a holder not shown to hold
 *   it, a start time that cannot be read or falls within the slack, and a `startedAt` past the reader's clock by more
 *   than the slack, which names no start ({@link startVerdict}); a daemon of a release before the home's lock
 *   holds none.
 */
type RecordedProcess = "ours" | "not-ours" | "unknown";

/**
 * {@link RecordedProcess} for the process daemon.json in `home` names (`state`), its start times read by `deadline`
 * (epoch ms), since each read costs a process on Windows. The home's lock as this pid's answers first
 * ({@link lockVerdict}); otherwise the pid's start decides, and a start no later than the record overrules a lock another
 * holder's record names: the record beside the lock can outlive the holder that wrote it, when it could neither be
 * replaced nor removed. That lock says not ours alone only while its holder is shown to have written its record.
 */
function recordedProcess(home: string, state: DaemonState, machine: DaemonMachine, deadline: number): RecordedProcess {
  const byLock = lockVerdict(home, state.pid, machine);
  if (byLock === "ours") return "ours";
  const byStart = startVerdict(state, machine, deadline);
  if (byStart !== "unknown" || byLock === "unknown") return byStart;
  return holderWroteRecord(home, machine, deadline) ? "not-ours" : "unknown";
}

/**
 * Whether the live process the holder record in `home` names wrote that record, read by `deadline`: it started no later
 * than the record was written. A daemon takes the home's lock before it records itself and holds it for life, so that
 * process holds the lock; a process that took a dead holder's pid since started after the record.
 */
function holderWroteRecord(home: string, machine: DaemonMachine, deadline: number): boolean {
  const holder = homeDaemonHolder(home);
  const written = homeDaemonHolderWrittenAt(home);
  if (holder === undefined || written === undefined) return false;
  const started = machine.processStartedAt(holder, deadline - Date.now());
  return started !== undefined && started <= written;
}

/**
 * {@link RecordedProcess} from the home's lock, which a daemon holds from before daemon.json names it until it ends and
 * the OS lets go of however it ends: it decides only while a live process recorded as its holder holds it. The lock is
 * asked only while a live process is recorded ({@link recordedHolderRuns}), and the record that decides is read once the
 * lock is found held: a daemon that takes a lock another let go of records itself beside it just after.
 */
function lockVerdict(home: string, pid: number, machine: DaemonMachine): RecordedProcess {
  if (!recordedHolderRuns(home, machine) || machine.homeDaemonLockHeld(home) !== true) return "unknown";
  const holder = homeDaemonHolder(home);
  if (holder === undefined || !processAlive(holder, machine.kill)) return "unknown";
  return holder === pid ? "ours" : "not-ours";
}

/** Whether the pid recorded beside the home's lock in `home` is a live process's. */
function recordedHolderRuns(home: string, machine: DaemonMachine): boolean {
  const holder = homeDaemonHolder(home);
  return holder !== undefined && processAlive(holder, machine.kill);
}

/**
 * {@link RecordedProcess} from when the process `state.pid` started, read by `deadline`, against the record's `startedAt`.
 * A daemon starts before it stamps `startedAt`, so its own start is never later; a process that took its pid started
 * after it died, later by more than {@link START_CLOCK_SLACK_MS}, which covers a clock set forward since the daemon
 * started. A `startedAt` further past the reader's clock than that slack, as a daemon stamps while the clock runs ahead
 * of a time it is set back to, names no start ({@link startNamesProcess}): `unknown`, and nothing is read.
 */
function startVerdict(state: DaemonState, machine: DaemonMachine, deadline: number): RecordedProcess {
  if (!startNamesProcess(state.startedAt)) return "unknown";
  const started = machine.processStartedAt(state.pid, deadline - Date.now());
  if (started === undefined) return "unknown";
  if (started <= state.startedAt) return "ours";
  return started > state.startedAt + START_CLOCK_SLACK_MS ? "not-ours" : "unknown";
}

/**
 * What the state file in `home` says, checked against the machine. `probeTimeoutMs` bounds the engine's proof and the
 * health probe together; a hook waiting out a busy daemon passes more (`patientLookup`). A live pid whose daemon did not
 * answer as itself is kept unless it is proven not to be the daemon that wrote the record ({@link recordedProcess}),
 * its start read with what is left of `readLimitMs` from the lookup's start (that limit, unless a stop passes its own
 * wait), which a read through Windows PowerShell keeps to and the engine's system call needs not: a busy daemon is
 * kept, and a process that took a dead daemon's pid is not. A start time not read proves nothing, and the record is
 * kept. The start of a daemon whose engine has no proof is read the same way ({@link olderDaemon}).
 */
export async function lookupDaemon(
  home: string,
  machine: DaemonMachine = THIS_MACHINE,
  probeTimeoutMs: number = PROBE_TIMEOUT_MS,
  readLimitMs: number = probeTimeoutMs,
): Promise<DaemonLookup> {
  const stateFile = join(home, DAEMON_STATE_FILE);
  const state = await readDaemonState(stateFile, machine.platform);
  if (state === undefined) return { status: "absent" };
  const deadline = Date.now() + readLimitMs;
  const found = await checkedLookup(stateFile, state, machine, probeTimeoutMs, deadline);
  if (foundRunning(found) || found.status === "absent" || recordedProcess(home, state, machine, deadline) !== "not-ours") return found;
  await rm(stateFile, { force: true });
  return { status: "absent", notDaemon: state.pid };
}

/**
 * What the pid, the engine's proof and the health probe make of the daemon `state` records, within `probeTimeoutMs`;
 * the start of a daemon whose engine has no proof is read by `readDeadline` (epoch ms), the lookup's own limit.
 */
async function checkedLookup(stateFile: string, state: DaemonState, machine: DaemonMachine, probeTimeoutMs: number, readDeadline: number): Promise<DaemonLookup> {
  const deadline = Date.now() + probeTimeoutMs;
  const recorded = await lookedUpEngine(state, machine, probeTimeoutMs, readDeadline);
  // Another user's process cannot be this user's daemon, and on Windows no daemon lives without its engine pipe or
  // fails its engine's proof: the state is stale, and its URL is never probed.
  if (recorded === "gone") {
    await rm(stateFile, { force: true });
    return { status: "absent" };
  }
  // A busy daemon is waited for, not forgotten; its URL is not probed with a capability its engine has not proved.
  if (recorded === "silent") return { status: "unresponsive", state };
  // A daemon of a release before the proof proved nothing, and its health route wants the capability: it is asked
  // nothing, and its state goes on without either capability, so no caller can send it one.
  if (recorded === "older") return { status: "older", state: withoutCapabilities(state) };
  return await probedLookup(stateFile, state, Math.max(0, deadline - Date.now()));
}

/** `state` without its access capability and native token. */
function withoutCapabilities(state: DaemonState): DaemonState {
  const { accessToken: _access, nativeToken: _native, ...rest } = state;
  return rest;
}

/** What the health probe of the daemon `state` records, which may be sent the capability, makes of it within `timeoutMs`. */
async function probedLookup(stateFile: string, state: DaemonState, timeoutMs: number): Promise<DaemonLookup> {
  const probe = await probeDaemon(state, { timeoutMs });
  if (probe.status === "refused") return { status: "refused", state };
  if (probe.status === "unreachable") return { status: "unresponsive", state };
  if (probe.health.pid !== state.pid) {
    // Something else answers on that URL; the recorded daemon is not the listener.
    await rm(stateFile, { force: true });
    return { status: "absent" };
  }
  return { status: "running", state, health: probe.health };
}

/** Whether `lookup` found a daemon running, one of a release before the proof included ({@link FoundRunning}). */
export function foundRunning(lookup: DaemonLookup): lookup is FoundRunning {
  return lookup.status === "running" || lookup.status === "older";
}

/** The daemon recorded in `home` that a lookup finds running ({@link FoundRunning}), or undefined when there is none. */
export async function findRunningDaemon(home: string, machine: DaemonMachine = THIS_MACHINE): Promise<FoundRunning | undefined> {
  const lookup = await lookupDaemon(home, machine);
  return foundRunning(lookup) ? lookup : undefined;
}

/**
 * Serialize the check-then-spawn window per home. Two sessions starting at
 * once would otherwise both see "absent" and each launch a daemon; the loser
 * would either die on the port or, with `--port 0`, run orphaned while both
 * fight over the state file. The lock is the OS lock `locks/start.lock`
 * (`withHomeLock`), free the moment a start holding it ends, however it ends.
 */
export async function withStartLock<T>(home: string, operation: () => Promise<T>): Promise<T> {
  return await withHomeLock(home, {
    name: START_LOCK,
    waitMs: START_TIMEOUT_MS,
    busy: holder => new Error(`Another Rerail start (${holder === undefined ? "starting" : `pid ${holder.pid}`}) is still in progress`),
  }, operation);
}

/** How much of the end of daemon.log a start that failed reads: far more than the lines it shows, never the whole log. */
const LOG_TAIL_BYTES = 64 * 1024;

/**
 * The last `lines` lines written to `path` from byte `offset` on: this launch's output, not earlier runs'. Only the
 * last {@link LOG_TAIL_BYTES} are read, since the log grows across runs and a launch can write much; a log that cannot
 * be read gives none.
 */
async function tailFile(path: string, offset: number, lines: number): Promise<string> {
  const text = await readTail(path, offset).catch(() => "");
  return text.trimEnd().split("\n").slice(-lines).join("\n");
}

/**
 * What `path` holds from byte `offset` on, or from {@link LOG_TAIL_BYTES} before its end when that is later. The read
 * then starts one byte early, so the line that edge cuts into is known and left out.
 */
async function readTail(path: string, offset: number): Promise<string> {
  const file = await open(path, "r");
  try {
    const { size } = await file.stat();
    const edge = size - LOG_TAIL_BYTES;
    const start = offset >= edge ? offset : edge - 1;
    const length = Math.max(0, size - start);
    const { buffer, bytesRead } = await file.read(Buffer.alloc(length), 0, length, start);
    const text = buffer.subarray(0, bytesRead).toString("utf8");
    return start === offset ? text : text.slice(text.indexOf("\n") + 1);
  } finally {
    await file.close();
  }
}

export interface DaemonControl {
  home: string;
  packageRoot: string;
  /** Repairs the SQLite binding below `packageRoot` before a daemon is spawned; tests substitute it. */
  ensureSqlite?: (packageRoot: string) => Promise<SqliteRepairResult>;
  /**
   * Told the pid of a daemon still in its start that another process launched, each time the start begins to wait for
   * it ({@link answeredOrExited}); never of the daemon this start launched, which it waits for without a word.
   */
  onWaiting?: (pid: number) => void;
}

/**
 * The daemon opens SQLite as soon as it starts; a binding built for another Node ABI (a Node upgrade behind a
 * stable Homebrew symlink is the usual cause) kills it before it can answer. Repairing here, in the launcher,
 * puts the rebuild and any unrepairable failure in front of whoever ran `rerail daemon start` rather than in the log.
 */
async function repairSqlite(control: DaemonControl, machine: DaemonMachine): Promise<void> {
  const repair = control.ensureSqlite ?? (async (packageRoot: string) => await ensureSqlite(packageRoot, {
    executable: machine.nodeExecutable(),
    log: line => process.stderr.write(`${line}\n`),
  }));
  const result = await repair(control.packageRoot);
  if (result.rebuilt) process.stderr.write(`Rebuilt better-sqlite3 in ${result.prefix}\n`);
}

/** Runs `start` under the start lock in `home`. */
async function startUnderLock<T>(home: string, start: () => Promise<T>): Promise<T> {
  makeRerailHome(home);
  return await withStartLock(home, start);
}

/**
 * Removes the autostart claim in `home` when it names this process. A launcher that hands its start to the active
 * version's CLI leaves the claim to end with its own exit (`claimHolds`), and a claim that cannot be read or removed
 * holds only while its launcher runs and renews it (`keepAutostartClaim`).
 */
async function endOwnClaim(home: string): Promise<void> {
  const claim = join(home, AUTOSTART_CLAIM_FILE);
  const named = await readFile(claim, "utf8").catch(() => "");
  if (named.trim() === String(process.pid)) await rm(claim, { force: true }).catch(() => undefined);
}

/**
 * Whether the holder `pid` recorded in `home` may still be the home's daemon, asked without loading anything: a live
 * process, and on Windows only while the engine pipe of the home's database is there, under its own name or its
 * fallback, which the daemon binds before daemon.json can name it and holds until it exits. Windows ends a daemon
 * without its exit handler at a logoff, a restart or a stop's kill, so its record outlives it and soon names a pid
 * Windows has given to another process. A daemon that has taken the lock and not yet bound its pipe is not counted:
 * a start then launches another, which exits naming it.
 */
function homeHolderMayRun(home: string, pid: number, machine: DaemonMachine): boolean {
  if (!processAlive(pid, machine.kill)) return false;
  return machine.platform !== "win32" || databasePipeListed(join(home, RERAIL_DATABASE_FILE), () => machine.pipes());
}

/**
 * Whether a daemon holds `home`, its daemon.json or not. As for the main instance ({@link mainInstanceHolderRuns}), a
 * rule that asks nothing of the lock answers first ({@link homeHolderMayRun}): the holder records its pid beside the lock
 * and removes the record when it lets go, so a home whose daemon stopped, was killed or never ran is answered from that
 * record. Only a holder that may run is asked about by its lock, which tells it from a process that took its pid since,
 * and loads the engine on macOS and Linux (`homeDaemonLockHeld`). A lock that cannot be asked (the engine does not
 * load, or the lock file does not open) counts as free, so no start or stop waits on it: the daemon a start launches
 * asks the lock itself and exits, naming the holder, when another daemon holds it.
 */
function homeDaemonRuns(home: string, machine: DaemonMachine): boolean {
  const pid = homeDaemonHolder(home);
  if (pid === undefined || !homeHolderMayRun(home, pid, machine)) return false;
  return machine.homeDaemonLockHeld(home) ?? false;
}

/**
 * The daemon a start or a stop acts on in `home`: the one daemon.json records, else the one that holds the home
 * ({@link homeDaemonRuns}) once it has recorded itself again. The daemon of a home writes daemon.json back within a
 * second of finding it gone or naming another process, so it is waited for, never started beside: a second daemon
 * would refuse to run beside it, as would every start after it, and `rerail daemon stop` would find none to stop. Each
 * lookup reads a start time within `readLimitMs` ({@link lookupDaemon}): a stop passes its own wait, so a record that
 * names another process is removed before the stop looks for the daemon that holds the home. One that has not recorded
 * itself by the end of the wait fails the lookup with what `unrecorded` says of the home. The wait lasts while
 * `holderWaits` says a holder has yet to record itself: a start ends it for a holder still in its start.
 */
async function lookupHomeDaemon(
  home: string,
  machine: DaemonMachine,
  readLimitMs: number = PROBE_TIMEOUT_MS,
  unrecorded: (home: string, platform: NodeJS.Platform) => string = unrecordedDaemonProblem,
  holderWaits: (home: string, machine: DaemonMachine) => boolean = homeDaemonRuns,
): Promise<DaemonLookup> {
  const deadline = Date.now() + START_TIMEOUT_MS;
  for (;;) {
    const lookup = await lookupDaemon(home, machine, PROBE_TIMEOUT_MS, readLimitMs);
    if (lookup.status !== "absent" || !holderWaits(home, machine)) return lookup;
    if (Date.now() >= deadline) throw new Error(unrecorded(home, machine.platform));
    await sleep(POLL_INTERVAL_MS);
  }
}

/**
 * What a start says when a daemon holds `home` and has not recorded itself within the start's wait: what a stop says
 * of it ({@link unrecordedDaemonProblem}), and that the start leaves that process running. The start gets here only
 * after its whole wait, with the holder recorded beside the lock at every look, so that daemon has held the home at
 * least that long, and a start that waited for the start lock first, as one begun while the start that launched the
 * daemon still waited, adds that start's wait.
 */
function holderNotRecorded(home: string, platform: NodeJS.Platform): string {
  return `${unrecordedDaemonProblem(home, platform)} This start leaves that process running.`;
}

/**
 * What to tell the person when a daemon holds `home` and never records itself. The pid the holder record names comes
 * with the command that shows its command line, to check before ending it: nothing Rerail reads on Windows shows which
 * process holds a lock, and the record can be one a holder left that another process's pid now matches, or one copied
 * with the home beside the daemon of the home it came from.
 */
function unrecordedDaemonProblem(home: string, platform: NodeJS.Platform): string {
  const pid = homeDaemonHolder(home);
  const unrecorded = `holds ${home} but has not written ${DAEMON_STATE_FILE} within ${START_TIMEOUT_MS / 1_000}s`;
  if (pid === undefined) return `A Rerail daemon ${unrecorded}; end that process, then run rerail daemon start.`;
  return `A Rerail daemon ${unrecorded}. ${homeDaemonHolderFile(home)} names pid ${pid}, which Rerail could `
    + `not show is that daemon: see its command line with ${commandLineCheck(pid, platform)}, and end it only if `
    + "it runs Rerail's daemon-entry.js for this home. Then run rerail daemon start.";
}

/**
 * A daemon still in its start ({@link daemonComingUp}) that a start waits for outside the start lock: `daemon`, the one
 * this start launched (`launched`), or one another process launched, whose exit is asked by its pid. `records` say where
 * it records itself and writes its output, and the wait ends at `deadline`, {@link START_TIMEOUT_MS} from the launch
 * or from finding it. `replaced` names the daemon of a release before the engine's proof this start ended before its
 * launch ({@link replacedOlderDaemon}), which the start's result names once that launch answers.
 */
interface ComingUp {
  status: "coming-up";
  pid: number;
  daemon: LaunchedDaemon;
  launched: boolean;
  records: LaunchRecords;
  deadline: number;
  replaced?: StartDaemonResult["replaced"];
}

/**
 * The pid of the daemon that holds `home` and is still in its start, with where its output starts in the home's log,
 * or undefined: the record beside the home's lock says it is starting (`homeDaemonStarting`), that pid runs, and the
 * lock is held, asked as `homeDaemonRuns` asks it. A daemon killed in its start leaves that record and lets go of the
 * lock, so it is not coming up. On Windows the engine pipe is not asked: a daemon binds it well into its start.
 */
function daemonComingUp(home: string, machine: DaemonMachine): { pid: number; logOffset: number } | undefined {
  const found = homeDaemonStarting(home);
  if (found === undefined || !processAlive(found.pid, machine.kill)) return undefined;
  return machine.homeDaemonLockHeld(home) === true ? found : undefined;
}

/** The pid of the daemon of `home` still in its start ({@link daemonComingUp}), for `rerail status`; undefined when none is. */
export function homeDaemonInStart(home: string, machine: DaemonMachine = THIS_MACHINE): number | undefined {
  return daemonComingUp(home, machine)?.pid;
}

/**
 * The daemon of `home` still in its start, found by a start that did not launch it: waited for as long as a start waits
 * for its own launch, from now, and gone once its pid is. One that daemon.json names and that answers is running,
 * whatever its record says: a record a daemon could not rewrite at the end of its start must not keep a start waiting.
 * What a start shows of it, when it has not answered by the deadline or exits in the start's last wait, is what it
 * wrote to the log from where its record says its output starts, the lines it wrote before this start found it
 * included, as for a daemon the start launched: what the log held before is earlier runs' and other starts'.
 */
async function foundStarting(home: string, machine: DaemonMachine): Promise<ComingUp | DaemonLookup | undefined> {
  const found = daemonComingUp(home, machine);
  if (found === undefined) return undefined;
  const { pid, logOffset } = found;
  const records = { home, stateFile: join(home, DAEMON_STATE_FILE), logFile: join(home, DAEMON_LOG_FILE), logOffset };
  const answering = await answeringDaemon(records.stateFile, pid, machine.platform);
  if (answering !== undefined) return { status: "running", ...answering };
  const exitReason = (): string | undefined => (processAlive(pid, machine.kill) ? undefined : `(pid ${pid}) exited`);
  return { status: "coming-up", pid, daemon: { pid, exitReason }, launched: false, records, deadline: Date.now() + START_TIMEOUT_MS };
}

/**
 * What a start finds in `home`, a daemon still in its start first: that one has not answered yet, recorded or not, and
 * is waited for, never reported as a daemon that does not answer nor started beside. One that takes the home while the
 * lookup runs is found after it: the lookup's wait for a holder to record itself ends once that holder says it is in
 * its start. A daemon the lookup found running is the answer even then, one of a release before the engine's proof
 * included: that one holds no lock, so the start ends it rather than leave it running beside the one in its start.
 */
async function startLookup(home: string, machine: DaemonMachine): Promise<DaemonLookup | ComingUp> {
  const before = await foundStarting(home, machine);
  if (before !== undefined) return before;
  const holderWaits = (at: string, on: DaemonMachine): boolean => homeDaemonRuns(at, on) && daemonComingUp(at, on) === undefined;
  const lookup = await lookupHomeDaemon(home, machine, PROBE_TIMEOUT_MS, holderNotRecorded, holderWaits);
  if (foundRunning(lookup)) return lookup;
  return (await foundStarting(home, machine)) ?? lookup;
}

/**
 * The `onWaiting` of a start in `home` that a person runs: it says on stderr that the start waits for the daemon `pid`,
 * still in its start. `rerail daemon start` and set-up both pass it, so they say the same.
 */
export function startingDaemonNoticeTo(home: string): (pid: number) => void {
  return pid => void process.stderr.write(`Rerail daemon (pid ${pid}) is still starting; this start waits for it to answer. See ${join(home, DAEMON_LOG_FILE)}\n`);
}

/**
 * The daemon `launched` once its state file names it and it answers its probe as itself; undefined until then. One that
 * has exited fails the start with what it wrote to the log.
 */
async function answeredLaunch(launched: LaunchedDaemon, records: LaunchRecords, platform: NodeJS.Platform): Promise<RunningDaemon | undefined> {
  const exit = launched.exitReason();
  if (exit !== undefined) throw await exitedLaunch(exit, records);
  return await answeringDaemon(records.stateFile, launched.pid, platform);
}

/**
 * Waits for the daemon `coming` names, still in its start, as a start waits for the daemon it launched (`readyDaemon`),
 * telling `control.onWaiting` first only when another process launched it: until it answers, or until it exits, which
 * this returns as how it exited. At `coming.deadline` it fails as {@link unansweredLaunch} says, and the daemon is left
 * running.
 */
async function answeredOrExited(control: DaemonControl, coming: ComingUp, machine: DaemonMachine): Promise<RunningDaemon | string> {
  if (!coming.launched) control.onWaiting?.(coming.pid);
  while (Date.now() < coming.deadline) {
    const exit = coming.daemon.exitReason();
    if (exit !== undefined) return exit;
    const daemon = await answeringDaemon(coming.records.stateFile, coming.daemon.pid, machine.platform);
    if (daemon !== undefined) return daemon;
    await sleep(POLL_INTERVAL_MS);
  }
  throw await unansweredLaunch(coming.daemon, coming.records);
}

/** Waits as {@link answeredOrExited} does; an exit fails the start with what the daemon wrote to the log. */
async function untilStarted(control: DaemonControl, coming: ComingUp, machine: DaemonMachine): Promise<RunningDaemon> {
  const ended = await answeredOrExited(control, coming, machine);
  if (typeof ended === "string") throw await exitedLaunch(ended, coming.records);
  return ended;
}

/**
 * Runs `locked` under the start lock in `control.home`. One that ends with a daemon still in its start lets go of the
 * lock and waits for that daemon ({@link answeredOrExited}), so another start finds it and waits beside, never behind
 * the lock. The daemon this start launched is reported as started once it answers, and its exit fails the start with
 * its log. One another process launched is looked up again under the lock once it answers, so a start replaces it as it
 * would a running one, or once it exits, so the start launches a daemon of its own, with its own package and options,
 * as in a home with none: a hook's start beside another package's daemon that fails in its start is one such case. A
 * second wait after that look is the last, and the exit of the daemon it waits for fails the start with that daemon's
 * log.
 *
 * Once the whole start is over, both looks and both waits, it ends the autostart claim (`claimAutostart`) if it names
 * this process, the launcher an autostart started, however the start ended: the launch it held other processes off for
 * is over, so the next process to find the daemon down launches again at once, after a start that failed too. While the
 * daemon cannot start, that is one launch after another, never one per process that finds it down. A claim naming
 * another process is that launcher's, still running its start: a start that gave up waiting for the start lock it
 * holds, or one a person ran beside it, leaves the claim to it.
 */
async function startBesideStarting(
  control: DaemonControl,
  machine: DaemonMachine,
  locked: () => Promise<StartDaemonResult | ComingUp>,
): Promise<StartDaemonResult> {
  try {
    let outcome = await startUnderLock(control.home, locked);
    if ("status" in outcome && !outcome.launched) {
      await answeredOrExited(control, outcome, machine);
      outcome = await startUnderLock(control.home, locked);
    }
    if (!("status" in outcome)) return outcome;
    const { launched, replaced } = outcome;
    const daemon = await untilStarted(control, outcome, machine);
    return replaced === undefined ? { started: launched, daemon } : { started: launched, daemon, replaced };
  } finally {
    await endOwnClaim(control.home);
  }
}

export async function startDaemon(
  control: DaemonControl,
  options: DaemonLaunchOptions,
  machine: DaemonMachine = THIS_MACHINE,
): Promise<StartDaemonResult> {
  return await startBesideStarting(control, machine, async () => {
    const lookup = await startLookup(control.home, machine);
    if (lookup.status === "coming-up") return lookup;
    if (lookup.status === "older") return await replacedOlderDaemon(control, options, lookup, machine);
    if (lookup.status === "running") return { started: false, daemon: { state: lookup.state, health: lookup.health } };
    if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
    return await spawnDaemon(control, options, machine);
  });
}

/**
 * Ends the daemon of a release before the engine's proof that a start found ({@link OlderDaemon}) through its process,
 * as a hook's start does, then starts this release's daemon in its place: a person who upgraded needs one command. A
 * launch still in its start carries the ended daemon to the wait outside the start lock ({@link ComingUp}).
 */
async function replacedOlderDaemon(
  control: DaemonControl,
  options: DaemonLaunchOptions,
  older: OlderDaemon,
  machine: DaemonMachine,
): Promise<StartDaemonResult | ComingUp> {
  const ended = await stopFound(control.home, older, machine);
  const started = await spawnDaemon(control, options, machine);
  // The start names only a daemon its end read as ended. A pid that passed to a later process since the lookup was let
  // alone; a daemon whose end could not be read and whose pid was free afterwards may have exited by itself or by that
  // end, so the start does not name it.
  if (ended !== "asked") return started;
  const { pid, version } = older.state;
  return { ...started, replaced: { pid, version } };
}

/**
 * What a caller that does not end it says of a daemon of a release before the engine's proof ({@link OlderDaemon}): it
 * is sent nothing, and `rerail daemon start` ends it and starts this release's daemon.
 */
export function olderDaemonMessage(state: Pick<DaemonState, "pid" | "version">): string {
  return `Rerail daemon pid ${state.pid} runs Rerail ${state.version}, a release before 0.1.5, which Rerail sends nothing; run rerail daemon start to end it and start this release's daemon.`;
}

/** What `rerail daemon start` says when it ended a daemon of a release before the engine's proof first. */
export function olderDaemonReplacedMessage(replaced: Pick<DaemonState, "pid" | "version">): string {
  return `Ended Rerail daemon pid ${replaced.pid}, which ran Rerail ${replaced.version}, a release before 0.1.5, and started this release's daemon in its place.`;
}

/**
 * Starts a daemon from `control.packageRoot`, replacing a healthy daemon from
 * another managed package. Hooks use this before sending engine service configuration:
 * an older daemon cannot safely accept fields introduced by the active hook.
 */
export async function startDaemonForPackage(
  control: DaemonControl,
  options: DaemonLaunchOptions,
  machine: DaemonMachine = THIS_MACHINE,
): Promise<StartDaemonResult> {
  return await startBesideStarting(control, machine, async () => {
    const lookup = await startLookup(control.home, machine);
    if (lookup.status === "coming-up") return lookup;
    if (foundRunning(lookup)) return await keptOrReplaced(control, options, lookup, machine);
    if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
    return await spawnDaemon(control, options, machine);
  });
}

/**
 * The daemon a hook's start found running (`found`), kept when it was launched from the hook's package, else ended and
 * replaced by one launched from it: always for a daemon of a release before the engine's proof ({@link OlderDaemon}).
 */
async function keptOrReplaced(
  control: DaemonControl,
  options: DaemonLaunchOptions,
  found: FoundRunning,
  machine: DaemonMachine,
): Promise<StartDaemonResult | ComingUp> {
  if (found.status === "running" && await daemonUsesPackageRoot(found.state, control.packageRoot)) {
    return { started: false, daemon: { state: found.state, health: found.health } };
  }
  await stopFoundDaemon(control.home, found, machine);
  return await spawnDaemon(control, options, machine);
}

/**
 * Whether a hook may send its settings and events to the healthy daemon it found (`found`): one launched from the hook's
 * own package (`packageRoot`), unless it runs a release before the engine's proof (`older`, known on Windows), which
 * predates the fields the hook sends whichever package it ran from.
 */
export async function daemonServesPackage(found: FoundRunning, packageRoot: string): Promise<boolean> {
  return found.status === "running" && await daemonUsesPackageRoot(found.state, packageRoot);
}

/** Whether a healthy daemon was launched from the package the caller is executing. */
export async function daemonUsesPackageRoot(daemon: Pick<DaemonState, "packageRoot">, packageRoot: string): Promise<boolean> {
  const canonical = async (path: string): Promise<string> => await realpath(path).catch(() => resolve(path));
  const [runningRoot, expectedRoot] = await Promise.all([canonical(daemon.packageRoot), canonical(packageRoot)]);
  return runningRoot === expectedRoot;
}

/**
 * `RERAIL_AUTOSTART=0` or `RERAIL_AUTOSTART=false`: the person runs the daemon themselves, and neither a hook, a
 * session's start nor set-up starts one. Every reader of the setting asks here, so each spelling means the same to all.
 */
export function autostartOff(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.RERAIL_AUTOSTART === "0" || env.RERAIL_AUTOSTART === "false";
}

/** No daemon is running, and `RERAIL_AUTOSTART` (`0` or `false`) kept the caller from starting one. */
export class AutostartOffError extends Error {
  override readonly name = "AutostartOffError";
}

/**
 * A daemon runs that the caller may not send events to (another package's, or a release before the engine's proof:
 * {@link daemonServesPackage}), and `RERAIL_AUTOSTART` (`0` or `false`) kept the caller from replacing it. Unlike a
 * daemon that is not running, this is no choice of the person's: hooks log each event it costs, and a session's start
 * says so (`answerAutostartOff`).
 */
export class OtherDaemonKeptError extends Error {
  override readonly name = "OtherDaemonKeptError";
}

/** What `rerail status` and `rerail daemon stop` say when daemon.json named a live process that is not the home's daemon. */
export const NOT_HOME_DAEMON_NOTE = "The process daemon.json named is not a Rerail daemon of this home, so Rerail ended nothing and removed that record.";

/**
 * What `rerail status`, and a session's start (`answerAutostartOff`), say when no daemon runs. Under `RERAIL_AUTOSTART=0`
 * it also says that nothing is recorded until the daemon runs, since hooks then start no daemon and log nothing about
 * the missing one: that is what the person asked for. It says nothing of later: the one lookup behind it cannot tell
 * whether a daemon comes up a moment after (a `rerail daemon start` in flight, the daemon's restart at its heap ceiling).
 */
export function daemonNotRunningMessage(env: NodeJS.ProcessEnv = process.env): string {
  return autostartOff(env)
    ? `The Rerail daemon is not running, and RERAIL_AUTOSTART=${env.RERAIL_AUTOSTART} keeps hooks from starting it, so nothing is recorded until it runs. Start it with rerail daemon start.`
    : "The Rerail daemon is not running. Start it with rerail daemon start.";
}

/**
 * What a session's start says (`answerAutostartOff`) when `RERAIL_AUTOSTART` kept its hook from replacing a daemon the
 * hook may not use ({@link OtherDaemonKeptError}): nothing is recorded until a daemon of the active package runs, and
 * the stop comes before the start, since a start beside a running daemon answers that one already runs.
 */
export function otherDaemonKeptMessage(env: NodeJS.ProcessEnv = process.env): string {
  return `The running Rerail daemon is from another Rerail package or an earlier release, and RERAIL_AUTOSTART=${env.RERAIL_AUTOSTART} keeps hooks from replacing it, so nothing is recorded until a daemon from the active package runs. Run rerail daemon stop, then rerail daemon start.`;
}

/**
 * The engine endpoint main host clients send events to; only the daemon evaluates them, since it alone opens
 * the machine-wide content store. `replaceOtherPackage` brings up this package's daemon in place of one from
 * another package or of a release before the engine's proof ({@link daemonServesPackage}), which is how a hook keeps
 * an older daemon from receiving fields it predates, and sends such a daemon nothing where it may not replace it; a
 * client that outlives an update passes false after its first event so it keeps using the newer daemon instead of
 * displacing it, though never a daemon of a release before the proof, which proved nothing. `RERAIL_AUTOSTART=0`
 * starts nothing and requires a running daemon, and throws `AutostartOffError` without one and `OtherDaemonKeptError`
 * beside one it may not use. A daemon that is alive but busy is waited for (`patientLookup`), until `deadline` (epoch
 * ms) when the caller names one, as a hook does. A daemon still in its start, launched by this start or found, is
 * waited for as every start waits for its own launch, {@link START_TIMEOUT_MS}, never cut at `deadline`: a hook waited
 * that long for the daemon it launched before a start told a daemon in its start apart, and at least as long for one it
 * found in its start.
 */
export async function daemonEngineEndpoint(
  replaceOtherPackage: boolean,
  deadline?: number,
  machine: DaemonMachine = THIS_MACHINE,
): Promise<EngineEndpoint> {
  const control = { home: defaultRerailHome(machine.env), packageRoot: PACKAGE_ROOT };
  const daemon = autostartOff(machine.env)
    ? await runningDaemon(control.home, deadline, machine)
    : await startedDaemon(control, replaceOtherPackage, deadline, machine);
  // Only the lookup an off autostart makes finds no daemon, so this error spells the setting as the person set it.
  if (daemon === undefined) {
    throw new AutostartOffError(`RERAIL_AUTOSTART=${machine.env.RERAIL_AUTOSTART} requires a running daemon; run rerail daemon start or set RERAIL_ENGINE_SOCKET`);
  }
  if (daemon.status === "older" || (replaceOtherPackage && !(await daemonServesPackage(daemon, control.packageRoot)))) {
    throw otherPackageDaemon(machine.env);
  }
  return engineEndpointOf(daemon.state);
}

/**
 * Why the caller sends no event to the daemon it found of another package or of a release before the engine's proof
 * ({@link daemonServesPackage}). An off autostart may not replace that daemon, so the error is an
 * {@link OtherDaemonKeptError}, which a session's start answers, and names the setting as the person set it, and the
 * stop before the start: a start beside a running daemon answers that one already runs. With autostart on, a start
 * replaces it unless it was busy when the start looked and answered only later, so the error names no setting.
 */
function otherPackageDaemon(env: NodeJS.ProcessEnv): Error {
  return autostartOff(env)
    ? new OtherDaemonKeptError(`RERAIL_AUTOSTART=${env.RERAIL_AUTOSTART} requires a daemon from the active Rerail package, and the running one is from another Rerail package or an earlier release; run rerail daemon stop, then rerail daemon start, or set RERAIL_ENGINE_SOCKET`)
    : new Error("the running Rerail daemon is from another Rerail package or an earlier release; run rerail daemon stop, then rerail daemon start");
}

/**
 * With `RERAIL_AUTOSTART=0`, the daemon running in `home`, if any. The lookup has already waited out a busy daemon
 * (`patientLookup`); one that still does not answer, or refuses the capability, throws `LiveDaemonError` and is not
 * waited for again.
 */
async function runningDaemon(home: string, deadline: number | undefined, machine: DaemonMachine): Promise<FoundRunning | undefined> {
  const lookup = await patientLookup(home, machine, deadline);
  if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
  return foundRunning(lookup) ? lookup : undefined;
}

/**
 * The daemon a start brings up or finds running: this package's in place of another package's when
 * `replaceOtherPackage` (`startDaemonForPackage`), else whichever runs (`startDaemon`). A start probes once and refuses
 * to replace a daemon that is alive but busy; that one is waited for (`patientLookup`), and throws the start's refusal
 * when it still does not answer.
 */
async function startedDaemon(
  control: DaemonControl,
  replaceOtherPackage: boolean,
  deadline: number | undefined,
  machine: DaemonMachine,
): Promise<FoundRunning> {
  const start = replaceOtherPackage ? startDaemonForPackage : startDaemon;
  try {
    return { status: "running", ...(await start(control, { dev: false }, machine)).daemon };
  } catch (error) {
    if (!(error instanceof LiveDaemonError) || error.lookup.status !== "unresponsive") throw error;
    const answered = await patientLookup(control.home, machine, deadline, error.lookup);
    if (!foundRunning(answered)) throw error;
    return answered;
  }
}

/**
 * The daemon a hook found after a session's start: its engine endpoint, or why no daemon takes the event. `absent`
 * means none runs; `unresponsive` and `refused` name a daemon that is alive, which a start would only refuse to replace.
 */
export type HookDaemon = { status: "running"; endpoint: EngineEndpoint } | Exclude<DaemonLookup, FoundRunning>;

/**
 * The engine endpoint for a hook after a session's start, or why there is none. A daemon that is down is not started:
 * each tool call then costs one failed probe instead of an autostart that can outlast the coding agent's hook timeout
 * (design N11). A busy one is waited for (`patientLookup`), until `deadline` when the hook names one. A healthy daemon
 * from another package (an update installed mid-session), or of a release before the engine's proof
 * ({@link daemonServesPackage}), is replaced as `daemonEngineEndpoint(true)` replaces it, so steering carries on with
 * this package's engine.
 */
export async function runningDaemonEngineEndpoint(deadline?: number, machine: DaemonMachine = THIS_MACHINE): Promise<HookDaemon> {
  const lookup = await patientLookup(defaultRerailHome(machine.env), machine, deadline);
  if (!foundRunning(lookup)) return lookup;
  const endpoint = await daemonServesPackage(lookup, PACKAGE_ROOT)
    ? engineEndpointOf(lookup.state)
    : await daemonEngineEndpoint(true, deadline, machine);
  return { status: "running", endpoint };
}

/**
 * The daemon the home records, when a hook already waited its whole wait for it and left an event it has not taken
 * since (`untakenFor`): its event loop has not run since then, so a hook asks it nothing and leaves its event behind
 * that one. Only an event in the folder the daemon watches counts (`DaemonState.untakenFolder`), and only while the
 * daemon may still run (`recordedDaemonMayRun`). Reads two files, and on Linux connects once; waits for nothing.
 */
export async function daemonLeftEventUntaken(machine: DaemonMachine = THIS_MACHINE): Promise<DaemonState | undefined> {
  const home = defaultRerailHome(machine.env);
  // A state file that cannot be read says nothing here: the hook asks the daemon, which records why it failed.
  const state = await readDaemonState(join(home, DAEMON_STATE_FILE), machine.platform).catch(() => undefined);
  if (state?.untakenFolder === undefined || !untakenFor(home, state.pid, state.untakenFolder)) return undefined;
  return await recordedDaemonMayRun(state, machine) ? state : undefined;
}

/**
 * Whether the daemon `state` records may still run, asked of the machine: its pid, then on Windows its own pipe, and on
 * Linux its engine socket, where a refused connect says no process listens there and the pid is another's now. A full
 * listen queue (a long silence's hooks) refuses a connect on macOS too, so there the pid alone tells.
 */
async function recordedDaemonMayRun(state: DaemonState, machine: DaemonMachine): Promise<boolean> {
  if (processState(state.pid, machine.kill) === "gone") return false;
  if (machine.platform === "win32") return enginePipeExists(state, machine);
  return machine.platform !== "linux" || await engineSocketListens(state.kernelSocketPath);
}

/**
 * Whether a process listens on the Unix socket at `socketPath`: false when a connect is refused or finds no file, true
 * when it connects or fails for any other reason (a full queue answers EAGAIN). A connect to a Unix socket ends at once,
 * so this waits for nothing.
 */
async function engineSocketListens(socketPath: string): Promise<boolean> {
  const probed = Promise.withResolvers<boolean>();
  const probe = connect(socketPath);
  probe.once("connect", () => {
    probe.destroy();
    probed.resolve(true);
  });
  probe.once("error", (error: NodeJS.ErrnoException) => probed.resolve(error.code !== "ECONNREFUSED" && error.code !== "ENOENT"));
  return await probed.promise;
}

/**
 * The daemon lookup a hook acts on. A daemon whose pid is alive but which missed the first probe's 2 s, as one does
 * while its one event loop runs an audit or a long garbage collection, is probed once more, with up to
 * `BUSY_DAEMON_WAIT_MS` or until `deadline` (epoch ms) when one is given; only an answer from the recorded pid makes it
 * running. Nothing, the capability included, goes to its engine socket before that answer: a pid Windows reused, or a
 * socket another process holds, is not the daemon.
 */
async function patientLookup(
  home: string,
  machine: DaemonMachine,
  deadline: number | undefined,
  first?: DaemonLookup,
): Promise<DaemonLookup> {
  const lookup = first ?? await lookupDaemon(home, machine);
  if (lookup.status !== "unresponsive") return lookup;
  const wait = deadline === undefined ? BUSY_DAEMON_WAIT_MS : deadline - Date.now();
  return wait > 0 ? await lookupDaemon(home, machine, wait) : lookup;
}

/**
 * Starts a daemon process with `args` (its entry script first) the way `rerail daemon start` does, in the Rerail home and
 * through the native spawn on Windows (`launchDaemonProcess`), and returns its pid without waiting for it: a daemon
 * restarting itself hands over this way.
 */
export function relaunchDaemon(args: string[], logFile: string, machine: DaemonMachine = THIS_MACHINE): number | undefined {
  const control = { home: defaultRerailHome(machine.env), packageRoot: PACKAGE_ROOT };
  return launchDaemonProcess(control, { args, env: machine.env, logFile }, machine).pid;
}

export interface LaunchedDaemon {
  pid: number | undefined;
  /** Why the launched process is gone, the same each time it is asked once it is; undefined while it runs. */
  exitReason: () => string | undefined;
}

function daemonArguments(control: DaemonControl, options: DaemonLaunchOptions, stateFile: string): string[] {
  const publicHosts = options.publicHosts?.length ? options.publicHosts.join(",") : undefined;
  return [
    join(control.packageRoot, ENTRY_POINTS.daemon),
    "--state-file", stateFile,
    "--workspace-root", resolve(options.cwd ?? process.cwd()),
    ...optionArguments("--dev", options.dev),
    ...optionArguments("--host", options.host),
    ...optionArguments("--allow-remote", options.allowRemote),
    ...optionArguments("--public-host", publicHosts),
    ...optionArguments("--tls-cert", options.tls?.certFile),
    ...optionArguments("--tls-key", options.tls?.keyFile),
    ...optionArguments("--insecure-http", options.insecureHttp),
    ...optionArguments("--port", options.port),
  ];
}

/** A daemon option as arguments: the flag alone for `true`, the flag and its value for a value, nothing when unset. */
function optionArguments(flag: string, value: string | number | boolean | undefined): string[] {
  if (value === undefined || value === false) return [];
  return value === true ? [flag] : [flag, String(value)];
}

/** A Node process to start detached, with NUL stdin and its stdout and stderr appended to `logFile`. */
export interface DetachedLaunch {
  args: readonly string[];
  /** The folder it runs in; undefined keeps this process's. */
  cwd: string | undefined;
  env: NodeJS.ProcessEnv;
  logFile: string;
}

/**
 * Why a process the native spawn started is gone, or undefined while it runs: its exit code, from the handle the native
 * module kept, which it closes once it has given the code, so a second read after the exit throws. A binary built before
 * it had no such handle, and there only a pid that no process holds says so.
 */
function detachedExitReason(launched: NativeProcess, kill: ProcessSignal): string | undefined {
  if (launched.exitCode === undefined) return processAlive(launched.pid, kill) ? undefined : "exited";
  const code = launched.exitCode();
  return code === null ? undefined : `exited with code ${code}`;
}

/**
 * Starts `launch` under {@link nodeExecutable}, detached from this process. On Windows it goes through the native spawn,
 * for two reasons. Node's `detached: true` there adds DETACHED_PROCESS, so the child has no console, and every console
 * program it starts (Git, node, npm) without windowsHide gets a console window of its own, which Windows 11 hands to
 * Windows Terminal; the native spawn uses CREATE_NO_WINDOW instead, a console with no window that those programs share.
 * And Node's spawn hands the child every inheritable handle in this process, not just its stdio: a Rust launcher such
 * as `just` leaves its own stdout inheritable, so a daemon started below it held the caller's output pipe open and a
 * captured `rerail setup | tee` never finished. The native spawn passes only NUL stdin and the log, and keeps the
 * child's handle, which says when it exits and with what code even after Windows gives its pid to another process.
 * The Node binary, both spawns and the pid checks are `machine`'s.
 */
export function launchDetached(launch: DetachedLaunch, machine: DaemonMachine = THIS_MACHINE): LaunchedDaemon {
  const program = machine.nodeExecutable();
  if (machine.platform === "win32") {
    const env = Object.fromEntries(Object.entries(launch.env).filter((entry): entry is [string, string] => entry[1] !== undefined));
    const launched = machine.spawnDetached({ program, args: [...launch.args], cwd: launch.cwd ?? null, env, logFile: launch.logFile });
    // The native module closes the handle once it has given the exit code, and a later read throws: keep the answer.
    let exited: string | undefined;
    return { pid: launched.pid, exitReason: () => (exited ??= detachedExitReason(launched, machine.kill)) };
  }
  const log = openSync(launch.logFile, "a", 0o600);
  let child: StartedChild;
  try {
    child = machine.spawn(program, [...launch.args], { detached: true, stdio: ["ignore", log, log], cwd: launch.cwd, env: launch.env });
  } finally {
    closeSync(log);
  }
  let exited: string | undefined;
  child.once("error", error => {
    exited = `could not start: ${error.message}`;
  });
  child.once("exit", code => {
    exited = `exited with code ${code ?? "signal"}`;
  });
  child.unref();
  return { pid: child.pid, exitReason: () => exited };
}

/**
 * Starts the daemon in the Rerail home, not the caller's folder: Windows holds a process's current folder open, so a
 * daemon started from a repository kept that checkout, and every folder above it, from being deleted or renamed for as
 * long as it ran. The repository goes to it as `--workspace-root` instead, and the daemon looks for Git from there
 * (`searchGitFrom`), so a `PATH` entry inside the repository is still skipped.
 */
function launchDaemonProcess(control: DaemonControl, launch: Omit<DetachedLaunch, "cwd">, machine: DaemonMachine): LaunchedDaemon {
  return launchDetached({ ...launch, cwd: control.home }, machine);
}

async function spawnDaemon(control: DaemonControl, options: DaemonLaunchOptions, machine: DaemonMachine): Promise<StartDaemonResult | ComingUp> {
  await repairSqlite(control, machine);
  await mkdir(control.home, { recursive: true, mode: 0o700 });
  // A Rerail home an earlier release made outside the profile folder kept its parent's ACL on Windows, which there lets
  // every local user read the capability and the database; a launch, rare beside hooks, gives it an owner-only one.
  keepFolderPrivate(control.home, { repair: true });
  const stateFile = join(control.home, DAEMON_STATE_FILE);
  const logFile = join(control.home, DAEMON_LOG_FILE);
  const env = { ...(options.env ?? machine.env), RERAIL_HOME: control.home };
  // The log is appended to across runs; a failure shows only what this launch wrote.
  const logOffset = await stat(logFile).then(info => info.size, () => 0);
  const args = daemonArguments(control, options, stateFile);
  const launched = launchDaemonProcess(control, { args, env, logFile }, machine);
  const ready = await readyDaemon(launched, { home: control.home, stateFile, logFile, logOffset }, machine);
  return "status" in ready ? ready : { started: true, daemon: ready };
}

/** The home a launched daemon holds, where it records itself and where it writes its output, from `logOffset` on. */
interface LaunchRecords {
  home: string;
  stateFile: string;
  logFile: string;
  logOffset: number;
}

/**
 * The daemon `launched` started, once its state file names it and it answers its probe as itself, or once it holds the
 * home and records itself still in its start ({@link daemonComingUp}), for the start to go on waiting for outside the
 * lock until the same deadline ({@link untilStarted}). A launch that exits first fails with what it wrote to the log;
 * one that has done neither within `START_TIMEOUT_MS` fails as {@link unansweredLaunch} says.
 */
async function readyDaemon(launched: LaunchedDaemon, records: LaunchRecords, machine: DaemonMachine): Promise<RunningDaemon | ComingUp> {
  const deadline = Date.now() + START_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const daemon = await answeredLaunch(launched, records, machine.platform);
    if (daemon !== undefined) return daemon;
    const { pid } = launched;
    if (pid !== undefined && daemonComingUp(records.home, machine)?.pid === pid) return { status: "coming-up", pid, daemon: launched, launched: true, records, deadline };
    await sleep(POLL_INTERVAL_MS);
  }
  throw await unansweredLaunch(launched, records);
}

/** A launch that exited (`exit` says how) before it answered, with what it wrote to the log from `logOffset` on. */
async function exitedLaunch(exit: string, { logFile, logOffset }: LaunchRecords): Promise<Error> {
  const output = await tailFile(logFile, logOffset, 20);
  return new Error(`Rerail daemon ${exit} before it was ready${output === "" ? "." : `:\n${output}`}`);
}

/**
 * What a start says of the daemon it launched when that daemon has not answered within `START_TIMEOUT_MS`: how it
 * exited, asked once more since it may have exited during the last poll; else that it may still be starting, by its
 * pid, and that this start leaves it running, with what it wrote so far. A daemon slow to come up, on a busy machine or
 * while another program holds a file it reads, goes on to serve: a start that said it failed would tell the person
 * nothing runs while one does.
 */
async function unansweredLaunch(launched: LaunchedDaemon, records: LaunchRecords): Promise<Error> {
  const exit = launched.exitReason();
  if (exit !== undefined) return await exitedLaunch(exit, records);
  const output = await tailFile(records.logFile, records.logOffset, 20);
  return new Error(`${mayStillStart(launched.pid, records.logFile)}${output === "" ? "" : `:\n${output}`}`);
}

/**
 * What a start says of the daemon `pid` it launched that has not answered within its wait: it may still be starting,
 * the start leaves it running, and `rerail status` shows it running once it answers. It names no second start, which
 * waits for a daemon still in its start only as long as this one did, and names one past its start that does not
 * answer as one to stop.
 */
function mayStillStart(pid: number | undefined, logFile: string): string {
  return `Rerail daemon${pid === undefined ? "" : ` (pid ${pid})`} has not answered within ${START_TIMEOUT_MS / 1_000}s `
    + "and may still be starting; this start leaves it running. Once it answers, rerail status shows it running. "
    + `See ${logFile}`;
}

/**
 * The daemon `pid` once the state file, read for `platform`, names it and it answers its probe as itself; undefined
 * until then.
 */
async function answeringDaemon(stateFile: string, pid: number | undefined, platform: NodeJS.Platform): Promise<RunningDaemon | undefined> {
  const state = await readDaemonState(stateFile, platform);
  if (state === undefined || state.pid !== pid) return undefined;
  const probe = await probeDaemon(state);
  return probe.status === "ok" && probe.health.pid === state.pid ? { state, health: probe.health } : undefined;
}

let autostartAttempted = false;

/** Creates the claim at `path`, naming this process, if nothing is there; whether this call made it. */
function createClaim(path: string): boolean {
  let claim: number;
  try {
    claim = openSync(path, "wx", 0o600);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EEXIST") return false;
    throw error;
  }
  try {
    writeSync(claim, String(process.pid));
  } finally {
    closeSync(claim);
  }
  return true;
}

/** The claim at `path` as one look finds it: when it was made or last renewed, and what it names; undefined once it is gone. */
function readClaim(path: string): { made: number; named: string } | undefined {
  try {
    return { made: statSync(path).mtimeMs, named: readFileSync(path, "utf8").trim() };
  } catch (error) {
    // A start ended it between this process's create and its look.
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
}

/**
 * The claim at `path` read again once the process it named, `named`, was found gone: the claim as handed over since
 * (`handOverClaim`), or undefined when it still names that process or is gone.
 */
function claimHandedOver(path: string, named: string): { made: number; named: string } | undefined {
  const again = readClaim(path);
  return again?.named === named ? undefined : again;
}

/**
 * Whether the claim at `path` still holds: it was made or renewed under `AUTOSTART_CLAIM_LEASE_MS` from now, either way,
 * so a claim made before the clock moved back expires too, and the process it names runs (`kill`'s signal 0). A claim
 * with no pid in it yet is being written by the process that created it, and holds. The autostart that made a claim
 * hands it to its launcher (`handOverClaim`) and then exits, so a claim that names a process found gone is read again:
 * it lapses only while it still names that process, and one handed over meanwhile holds while its launcher runs.
 */
function claimHolds(path: string, kill: ProcessSignal): boolean {
  let claim = readClaim(path);
  while (claim !== undefined && Math.abs(Date.now() - claim.made) < AUTOSTART_CLAIM_LEASE_MS) {
    if (claim.named === "") return true;
    if (!/^[1-9]\d*$/u.test(claim.named)) return false;
    if (processAlive(Number(claim.named), kill)) return true;
    claim = claimHandedOver(path, claim.named);
  }
  return false;
}

/**
 * Whether this process launches the autostart: it created the claim in `home`, or replaced one that no longer holds
 * (`claimHolds`), such as one whose launcher died before its start. Sessions and hooks each run in a process of their
 * own, and the ones that find the daemon gone arrive together; the claim is created exclusively, so one launches and the
 * rest leave the start to it. The launcher's start ends the claim as it ends (`startBesideStarting`). Two processes
 * that both replace a claim at once can both launch; the start lock still lets only one launcher start a daemon.
 */
function claimAutostart(home: string, kill: ProcessSignal): boolean {
  const claim = join(home, AUTOSTART_CLAIM_FILE);
  if (createClaim(claim)) return true;
  if (claimHolds(claim, kill)) return false;
  rmSync(claim, { force: true });
  return createClaim(claim);
}

/**
 * Keeps the autostart claim in `home` held while it names this process, the `rerail daemon start` an autostart launched:
 * every {@link AUTOSTART_CLAIM_RENEW_MS} it renews the claim's time, so the claim holds while this launcher runs and
 * lapses within {@link AUTOSTART_CLAIM_LEASE_MS} of its end however it ends, a kill included, whatever process gets its
 * pid. It renews for {@link AUTOSTART_CLAIM_MS} of this process's life (`startedAt`) at most. The claim may name the
 * autostart that launched this process for a moment, until the autostart hands it over (`handOverClaim`), so each turn
 * looks again. A claim that is gone or unreadable is left to lapse. Returns what stops the renewal; it never keeps the
 * process alive.
 */
export function keepAutostartClaim(home: string, startedAt = Date.now() - process.uptime() * 1_000): () => void {
  const claim = join(home, AUTOSTART_CLAIM_FILE);
  const renew = (): void => {
    if (Date.now() - startedAt >= AUTOSTART_CLAIM_MS) {
      clearInterval(renewal);
      return;
    }
    try {
      if (readFileSync(claim, "utf8").trim() !== String(process.pid)) return;
      const now = new Date();
      utimesSync(claim, now, now);
    } catch {
      // Gone, ended by this launcher's start, or unreadable: it lapses.
    }
  };
  const renewal = setInterval(renew, AUTOSTART_CLAIM_RENEW_MS);
  renewal.unref();
  // At once too, so the lease runs from this launcher's start, not a second after it.
  renew();
  return () => clearInterval(renewal);
}

/**
 * Hands the claim in `home` to the launcher this process started, so it holds while that launch runs; with no launcher
 * (the launch started no process), removes it, so the next process to find the daemon down launches.
 */
function handOverClaim(home: string, launcher: number | undefined): void {
  const claim = join(home, AUTOSTART_CLAIM_FILE);
  if (launcher === undefined) rmSync(claim, { force: true });
  else writeFileSync(claim, String(launcher), { mode: 0o600 });
}

/**
 * Bring the daemon up from inside a host session so the browser view is
 * live without anyone running `rerail daemon start`. One launch per process,
 * and one per Rerail home while its claim holds (`claimAutostart`): a process
 * that left the start to another asks again at its next call.
 * `RERAIL_AUTOSTART=0` disables it ({@link autostartOff}).
 *
 * Truly fire-and-forget: the work is handed to a detached `rerail daemon start`
 * process and nothing here is awaited. Running `startDaemon` in-process
 * would leave its probes and poll timers on the event loop, and a one-shot
 * hook would then sit through daemon startup, or the 15 s timeout, before
 * it could exit. The launcher's own output goes to `daemon.log`; a failure
 * is visible there and through `rerail status`, never in the host. The
 * launcher starts as the daemon does ({@link launchDetached}), so on Windows
 * the SQLite probe, npm and the managed-version dispatch it runs open no window.
 * The launcher runs in the Rerail home, as the daemon does ({@link launchDaemonProcess}): Windows holds a process's
 * current folder open, so one started in the session's folder kept that checkout from being deleted or renamed while
 * it ran. The session's folder goes to it as `--workspace-root`, which the daemon serves and looks for Git from.
 *
 * Every session records into the one database in the Rerail home, and the one
 * daemon serves every repository; sessions in any repository reuse it for their
 * engine service and status.
 */
export function autostartDaemon(machine: DaemonMachine = THIS_MACHINE): void {
  if (autostartAttempted || autostartOff(machine.env)) return;
  const home = defaultRerailHome(machine.env);
  try {
    makeRerailHome(home);
    if (!claimAutostart(home, machine.kill)) return;
    autostartAttempted = true;
    let launcher: number | undefined;
    try {
      launcher = launchDetached({
        args: [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "daemon", "start", "--workspace-root", process.cwd()],
        cwd: home,
        env: machine.env,
        logFile: join(home, DAEMON_LOG_FILE),
      }, machine).pid;
    } finally {
      handOverClaim(home, launcher);
    }
  } catch {
    // Autostart is a convenience; the session must never notice it failing.
  }
}

/**
 * What a stop did: `stopped`, it asked the daemon it found to close, or found it ending, and killed it past the stop's
 * wait only while still proven the home's daemon; `not-running`, it found no daemon, only no record or a stale one the
 * lookup cleared; `not-daemon`, the record named a live process proven not the home's daemon, at the lookup or just
 * before the SIGTERM, and it removed that record and ended nothing (said with {@link NOT_HOME_DAEMON_NOTE});
 * `record-changed`, just before the SIGTERM the pid was proven not the home's daemon while daemon.json named it no
 * longer (another pid, none, or unreadable), and it ended and removed nothing.
 */
export type StopDaemonResult = "stopped" | "not-running" | "not-daemon" | "record-changed";

/** What `rerail daemon stop` says of each {@link StopDaemonResult}. */
export const STOP_ANSWERS: Readonly<Record<StopDaemonResult, string>> = {
  stopped: "Rerail daemon stopped.\n",
  "not-running": "The Rerail daemon is not running.\n",
  "not-daemon": `${NOT_HOME_DAEMON_NOTE}\nThe Rerail daemon is not running.\n`,
  "record-changed": "daemon.json changed while Rerail was stopping the daemon it named, so Rerail ended nothing and left that "
    + "record as it is; run rerail daemon stop again to stop the daemon it names now.\n",
};

/**
 * What to tell the person when the daemon outlives the kill, with a check that runs where they are: Windows has no
 * `ps` in cmd, and PowerShell's `ps` refuses `-p`; `tasklist` works in both.
 */
export function stillRunningMessage(pid: number, platform: NodeJS.Platform = process.platform): string {
  const check = platform === "win32" ? `tasklist /FI "PID eq ${pid}"` : `ps -p ${pid}`;
  return `Rerail daemon pid ${pid} is still running ${KILL_TIMEOUT_MS / 1_000}s after SIGKILL; check it with ${check}, then run rerail daemon stop again.`;
}

/**
 * Sends `signal` to the daemon, saying who to ask when this user may not signal it (`EPERM`). A daemon that has ended
 * (`ESRCH`) is what the stop wants: its own shutdown can finish between the stop's last look and the signal, as when it
 * waits out a session's requests for about as long as the stop waits for it, and the stop then goes on as for a
 * daemon gone by itself.
 */
function signalDaemon(pid: number, signal: string, machine: DaemonMachine): void {
  try {
    machine.kill(pid, signal);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ESRCH") return;
    if (code !== "EPERM") throw error;
    throw new Error(`Rerail daemon pid ${pid} runs as another user or elevated, and this user may not stop it; stop it from there.`, { cause: error });
  }
}

/**
 * Asks the daemon `state` records to close itself over its HTTP API, with its capability and native token; whether it
 * took the request. On Windows `process.kill` ends a process at once, so this request is the only way the daemon's own
 * shutdown runs there: requests from other sessions finish, and it removes its state file and main-instance record.
 * Only a daemon that may be sent the capability ({@link recordedDaemonAlive}) is asked: a lookup finds a daemon whose
 * engine stayed silent to the proof unresponsive, without proving it is this user's.
 */
async function requestDaemonShutdown(state: DaemonState, machine: DaemonMachine): Promise<boolean> {
  if (state.nativeToken === undefined || !(await recordedDaemonAlive(state, machine))) return false;
  try {
    const endpoint = daemonEndpoint(state, "/native/shutdown");
    const response = await endpoint.fetch(endpoint.url, {
      method: "POST",
      headers: { ...endpoint.headers, authorization: `Bearer ${state.nativeToken}` },
      signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    });
    await response.body?.cancel();
    return response.status === 202;
  } catch {
    return false;
  }
}

/**
 * What daemon.json in `home` says of `pid` now: it still names it (`named`), names another (`other`) or is gone (`gone`);
 * undefined when it cannot be read or parsed, which tells neither.
 */
function recordNow(home: string, pid: number): "named" | "other" | "gone" | undefined {
  let text: string;
  try {
    text = readFileSync(join(home, DAEMON_STATE_FILE), "utf8");
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "ENOENT" ? "gone" : undefined;
  }
  try {
    return (JSON.parse(text) as { pid?: unknown } | null)?.pid === pid ? "named" : "other";
  } catch {
    return undefined;
  }
}

/**
 * Removes daemon.json in `home` while it names `pid`, and says whether it did. A record that names another process is
 * that process's, as when a start beside the stop launched a daemon that recorded itself; one that cannot be read tells
 * neither, and is left as it is.
 */
async function forgetRecordOf(home: string, pid: number): Promise<boolean> {
  if (recordNow(home, pid) !== "named") return false;
  await rm(join(home, DAEMON_STATE_FILE), { force: true });
  return true;
}

/**
 * Removes daemon.json in `home` once a stop of `pid` is done, as main does, unless it cannot be read or names the live
 * process that holds the home's lock as its recorded holder ({@link namesLockHolder}): a daemon a start beside the stop
 * launched, which took the lock and recorded itself. A record naming any other process belongs to no daemon of the home,
 * and a later stop would act on the process it names.
 */
async function forgetAfterStop(home: string, pid: number, machine: DaemonMachine): Promise<void> {
  const record = recordNow(home, pid);
  if (record === undefined || record === "gone" || (record === "other" && namesLockHolder(home, machine))) return;
  await rm(join(home, DAEMON_STATE_FILE), { force: true });
}

/** Whether daemon.json in `home` names the live process that holds the home's lock as its recorded holder ({@link lockVerdict}). */
function namesLockHolder(home: string, machine: DaemonMachine): boolean {
  const holder = homeDaemonHolder(home);
  return holder !== undefined && recordNow(home, holder) === "named" && lockVerdict(home, holder, machine) === "ours";
}

/**
 * What asking a daemon to stop came to ({@link askDaemonToStop}): it was asked, or let alone as already ending (`asked`);
 * the end of a daemon of a release before the engine's proof could not be read and its pid was free afterwards, so it
 * had exited, by itself or by that end ({@link endOlderDaemon}: `exited`); its pid was proven not the home's daemon and
 * the record naming it is removed (`not-daemon`); or it was proven not the home's daemon once daemon.json named it no
 * longer, and that record is left as it is (`record-changed`).
 */
type StopAsk = "asked" | "exited" | Extract<StopDaemonResult, "not-daemon" | "record-changed">;

/**
 * Asks the daemon `state` records in `home` to shut down: SIGTERM, or on Windows a shutdown request, SIGTERM only if it
 * is refused. The shutdown request carries its own proof, the engine's ({@link requestDaemonShutdown}), so `check`, which
 * decides the signal, is asked after it and just before the signal: a stop of a busy daemon reads its start once, with
 * no wait between that read and the signal for its pid to pass to another process.
 */
async function askDaemonToStop(state: DaemonState, check: () => SignalVerdict, home: string, machine: DaemonMachine): Promise<StopAsk> {
  if (machine.platform === "win32" && (await requestDaemonShutdown(state, machine))) return "asked";
  return await signalWhenProven(state, check(), home, machine);
}

/**
 * Sends SIGTERM to the pid `state` names when `verdict` proves it the home's daemon now ({@link provenNow}), whatever
 * daemon.json says meanwhile. One that is ending is let alone. One proven another process's is let alone, and the record
 * naming it is removed, or, when daemon.json names it no longer, left as it is. One nothing proves either way is let
 * alone, with the record, and the stop says so ({@link unprovenDaemonMessage}).
 */
async function signalWhenProven(state: DaemonState, verdict: SignalVerdict, home: string, machine: DaemonMachine): Promise<StopAsk> {
  if (verdict === "unknown") throw new Error(unprovenDaemonMessage(home, state.pid, machine.platform));
  if (verdict === "ours") signalDaemon(state.pid, "SIGTERM", machine);
  if (verdict !== "not-ours") return "asked";
  return (await forgetRecordOf(home, state.pid)) ? "not-daemon" : "record-changed";
}

/** Whether the process `alive` watches is gone within `ms`, polling while it runs. */
async function goneWithin(alive: () => boolean, ms: number): Promise<boolean> {
  const deadline = Date.now() + ms;
  while (alive() && Date.now() < deadline) await sleep(POLL_INTERVAL_MS);
  return !alive();
}

/**
 * Stops the daemon `home` records: SIGTERM on macOS and Linux, which runs its shutdown; on Windows, where no signal
 * reaches a handler, a shutdown request (`requestDaemonShutdown`) first. Either way a daemon still running after
 * {@link STOP_TIMEOUT_MS} is killed.
 */
export async function stopDaemon(home: string, machine: DaemonMachine = THIS_MACHINE): Promise<StopDaemonResult> {
  const lookup = await lookupHomeDaemon(home, machine, STOP_TIMEOUT_MS);
  if (lookup.status === "absent") return lookup.notDaemon === undefined ? "not-running" : "not-daemon";
  return await stopFoundDaemon(home, lookup, machine);
}

/** A daemon a lookup found alive: running, of a release before the proof, unresponsive or refusing the capability. */
export type FoundDaemon = Exclude<DaemonLookup, { status: "absent" }>;

/**
 * What a stop may do to the pid daemon.json named, asked just before each signal ({@link provenNow}): signal it as the
 * home's daemon (`ours`); let it alone as another process's (`not-ours`), or as a daemon that has exited, or whose pid a
 * process started since has taken (`ending`); or let it alone and say so, since it runs and no fact proves whose it is
 * (`unknown`).
 */
type SignalVerdict = RecordedProcess | "ending";

/**
 * What proved the daemon a stop found the home's ({@link provenNow}): its answer to the health probe as itself, given
 * while it held the home's lock (`lock`) or while it held none (`answer`); or, for one that did not answer, the home's
 * lock held as its recorded holder when the stop began (`holder`), else the facts {@link recordedProcess} reads
 * (`record`).
 */
type StopProof = "lock" | "answer" | "holder" | "record";

/** The proofs that rest on the home's lock held as the daemon's when the stop began. */
const LOCK_PROOFS: ReadonlySet<StopProof> = new Set(["lock", "holder"]);

/**
 * The {@link provenNow} a stop asks just before each signal to the daemon `found` names. Never one of a release before
 * the engine's proof, which proved nothing and is ended through its process ({@link endOlderDaemon}).
 */
function signalCheck(home: string, found: Exclude<FoundDaemon, OlderDaemon>, machine: DaemonMachine): () => SignalVerdict {
  const held = lockVerdict(home, found.state.pid, machine) === "ours";
  const proof: StopProof = found.status === "running" ? (held ? "lock" : "answer") : (held ? "holder" : "record");
  return () => provenNow(home, found.state, proof, machine);
}

/**
 * The one rule a stop of the daemon `state` names in `home` follows, asked again just before each signal: it acts on the
 * process it proved by `proof`, as the facts the OS keeps show it now ({@link provenBy}), never on what daemon.json says.
 * A process still proven is `ours`, to be ended as main does. One no longer proven gets nothing: `ending` once it has
 * exited; `not-ours` when the home's lock belongs to another live process and daemon.json names it no longer
 * ({@link lockOfAnother}), as when a start beside the stop launched a daemon; otherwise what {@link provenBy} said. A
 * daemon that held the lock when the stop began ({@link LOCK_PROOFS}), whether it answered or was too busy to, and that
 * neither the lock nor its start decides by now, is `ours` while it runs: its exit can run on after it let go of the
 * lock, and on macOS, or on Windows when PowerShell does not answer, no start is read.
 */
function provenNow(home: string, state: DaemonState, proof: StopProof, machine: DaemonMachine): SignalVerdict {
  const verdict = provenBy(home, state, proof, machine);
  if (verdict === "ours") return "ours";
  if (!processAlive(state.pid, machine.kill)) return "ending";
  if (lockOfAnother(home, state.pid, machine)) return "not-ours";
  return verdict === "unknown" && LOCK_PROOFS.has(proof) ? "ours" : verdict;
}

/**
 * {@link SignalVerdict} for the process `state` names, by the facts that proved it (`proof`). A daemon that did not
 * answer and held no lock when the stop began is as {@link recordedProcess} shows it: the home's lock held as its
 * recorded holder, or a start no later than its record. One that answered as itself while it held no lock stays proven
 * while it runs. One that held the lock when the stop began, whether it answered or was too busy to, stays proven while
 * it holds it or its start shows it the process that wrote its record, as when its exit runs on after it let go of the
 * lock. It is `ending` once its start reads later than its record by more than the clock's slack ({@link startVerdict}),
 * as a process that took its pid since does, and `unknown` while neither the lock nor its start decides.
 */
function provenBy(home: string, state: DaemonState, proof: StopProof, machine: DaemonMachine): SignalVerdict {
  if (proof === "record") return recordedProcess(home, state, machine, Date.now() + STOP_TIMEOUT_MS);
  if (proof === "answer" || lockVerdict(home, state.pid, machine) === "ours") return "ours";
  const byStart = startVerdict(state, machine, Date.now() + STOP_TIMEOUT_MS);
  return byStart === "not-ours" ? "ending" : byStart;
}

/**
 * Whether the home's lock in `home` belongs to a live process other than `pid` ({@link lockVerdict}) while daemon.json
 * no longer names `pid`: a start beside the stop removed that record and launched a daemon, which took the lock and
 * recorded itself beside it. A record that still names `pid` keeps the question open: the record beside the lock can
 * outlive the holder that wrote it ({@link recordedProcess}).
 */
function lockOfAnother(home: string, pid: number, machine: DaemonMachine): boolean {
  const record = recordNow(home, pid);
  return (record === "gone" || record === "other") && lockVerdict(home, pid, machine) === "not-ours";
}

/**
 * What to tell the person when a stop cannot prove that the live process daemon.json names is the home's daemon
 * ({@link RecordedProcess}), with what came of it (`outcome`): how to see what it is, and what to do either way.
 */
function unprovenDaemonMessage(home: string, pid: number, platform: NodeJS.Platform = process.platform, outcome = "so it ended nothing"): string {
  const record = join(home, DAEMON_STATE_FILE);
  return `Rerail could not prove that pid ${pid}, which ${record} names, is this home's daemon, ${outcome}. `
    + `See its command line with ${commandLineCheck(pid, platform)}: if it runs Rerail's daemon-entry.js for this home, `
    + `end it; if not, delete ${record}. Then run rerail daemon start.`;
}

/** The command that shows the command line of the process holding `pid` on `platform`. */
function commandLineCheck(pid: number, platform: NodeJS.Platform): string {
  return platform === "win32"
    ? `powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}').CommandLine"`
    : `ps -p ${pid} -o command=`;
}

/**
 * Whether the pid a stop signalled in `home` may be killed, now that it outlived the stop's wait, by `verdict`: only while
 * it is proven the home's daemon. One that is ending or another process's by now is left alone, and so is one that did
 * not hold the home's lock when the stop began ({@link provenNow}), recorded as its holder, whose lock file is there and
 * held by no one: a daemon lets go of it once it has closed, just before it exits. One that runs while nothing proves it either way is left alone too, with every record,
 * and the stop says so ({@link unprovenDaemonMessage}): the cleanup after a stop would forget a daemon that may still
 * run. A missing lock file is no lock let go ({@link homeDaemonLockHeld}).
 */
function killable(home: string, pid: number, verdict: SignalVerdict, machine: DaemonMachine): boolean {
  if (verdict !== "unknown") return verdict === "ours";
  if (homeDaemonHolder(home) === pid && machine.homeDaemonLockHeld(home) === false) return false;
  throw new Error(unprovenDaemonMessage(home, pid, machine.platform));
}

/**
 * Waits up to {@link STOP_TIMEOUT_MS} for the process `pid` to end, as its stop asked, then kills it while `check` still
 * proves it the home's daemon ({@link killable}), and waits {@link KILL_TIMEOUT_MS} more.
 */
async function endWhenStopped(home: string, pid: number, check: () => SignalVerdict, machine: DaemonMachine): Promise<void> {
  const alive = (): boolean => processAlive(pid, machine.kill);
  if ((await goneWithin(alive, STOP_TIMEOUT_MS)) || !killable(home, pid, check(), machine)) return;
  signalDaemon(pid, "SIGKILL", machine);
  // The state file stays, so a later stop can find the process again.
  if (!(await goneWithin(alive, KILL_TIMEOUT_MS))) throw new Error(stillRunningMessage(pid, machine.platform));
}

/**
 * Stops the daemon `found`, which a {@link lookupDaemon} of `home` returned, as {@link stopDaemon} does: `not-daemon`,
 * and its record removed, when the pid it names is proven not the home's daemon when the stop would signal it
 * ({@link signalCheck}); it throws {@link unprovenDaemonMessage}, ending nothing and keeping every record, when nothing
 * proves it either way while it runs.
 */
export async function stopFoundDaemon(home: string, found: FoundDaemon, machine: DaemonMachine = THIS_MACHINE): Promise<StopDaemonResult> {
  const asked = await stopFound(home, found, machine);
  return asked === "asked" || asked === "exited" ? "stopped" : asked;
}

/** {@link stopFoundDaemon}, with what its end came to: `exited` for a daemon of a release before the proof that had exited. */
async function stopFound(home: string, found: FoundDaemon, machine: DaemonMachine): Promise<StopAsk> {
  const pid = found.state.pid;
  const asked = await endFoundDaemon(home, found, machine);
  if (asked !== "asked" && asked !== "exited") return asked;
  // The daemon removes its own state file, main-instance record and home lock record on a clean shutdown; one killed, as
  // Windows always does, leaves them behind. A record naming a pid Windows reuses would keep the uninstall from the
  // cache, or have a later stop ask the home's lock.
  await forgetAfterStop(home, pid, machine);
  if (machine.platform === "win32") {
    machine.forgetMainInstanceHolder(pid);
    forgetHomeDaemonHolder(home, pid);
  }
  return asked;
}

/**
 * Ends the daemon `found` names as {@link stopFoundDaemon} does: a daemon of a release before the engine's proof through
 * its process ({@link endOlderDaemon}); any other is asked to stop, then killed while it is proven the home's daemon.
 */
async function endFoundDaemon(home: string, found: FoundDaemon, machine: DaemonMachine): Promise<StopAsk> {
  if (found.status === "older") return await endOlderDaemon(home, found.state, machine);
  const check = signalCheck(home, found, machine);
  const asked = await askDaemonToStop(found.state, check, home, machine);
  if (asked === "asked") await endWhenStopped(home, found.state.pid, check, machine);
  return asked;
}

/** What came of an end of a daemon of a release before the proof that answered `unread` while its pid runs. */
const UNREAD_END = "or that it ended: Rerail could not open the process, read its start, or wait for its exit once it asked Windows to end it";

/**
 * Ends a daemon of a release before the engine's proof ({@link OlderDaemon}), which is sent nothing, through its
 * process, never by its pid's number: one call opens the process, reads its start and ends it through that handle, so
 * the process ended is the one whose start shows it wrote the record, and waits up to {@link KILL_TIMEOUT_MS} on that
 * handle for it to exit. A process that took the pid since, started later, is let alone and the record naming it goes.
 * An end that is `unread` while its pid runs keeps every record, and the stop says the process may still run and may
 * have been ended. One whose pid is free by then had exited (`exited`).
 */
async function endOlderDaemon(home: string, state: DaemonState, machine: DaemonMachine): Promise<StopAsk> {
  const end = machine.endProcessStartedBy(state.pid, state.startedAt, KILL_TIMEOUT_MS);
  if (end === "running") throw new Error(stillRunningMessage(state.pid, machine.platform));
  if (end === "later") return (await forgetRecordOf(home, state.pid)) ? "not-daemon" : "record-changed";
  if (end === "ended") return "asked";
  // Not opened, its start not read, or its exit not waited on once the end was asked, so it may have been ended: a pid
  // that still runs is let alone, and a free one had exited, by itself or by that end.
  if (processAlive(state.pid, machine.kill)) throw new Error(unprovenDaemonMessage(home, state.pid, machine.platform, UNREAD_END));
  return "exited";
}

/**
 * The program that hands `url` to the desktop's default browser, its arguments, and the folder it runs in; anything
 * but macOS and Windows is assumed to be a freedesktop environment. Windows goes through `rundll32` rather than
 * `cmd /c start`: `start` parses its command line through cmd, so the `&` in a session URL's query string would split
 * it into separate commands. It runs by its System32 path from the home folder, because Windows looks for a bare name
 * in the current folder first, and Rerail opens pages from inside repositories.
 */
export function browserOpener(url: string, platform: NodeJS.Platform = process.platform, env: NodeJS.ProcessEnv = process.env): { command: string; args: string[]; cwd?: string } {
  if (platform === "darwin") return { command: "open", args: [url] };
  if (platform === "win32") return { command: windowsSystemProgram("rundll32.exe", env), args: ["url.dll,FileProtocolHandler", url], cwd: homedir() };
  return { command: "xdg-open", args: [url] };
}

/**
 * `BROWSER=none`: whoever runs Rerail asks that no browser open, as tools that open one commonly honour. Set-up
 * (`noBrowserReason`) and every page opener here read it.
 */
export function browserOff(env: NodeJS.ProcessEnv): boolean {
  return env.BROWSER === "none";
}

/** What a command prints when `BROWSER=none` kept it from opening a page; it shows the link itself. */
export const BROWSER_OFF_NOTE = "BROWSER is set to none, so no browser opened.";

/**
 * Hands the URL to the desktop's default browser, unless `BROWSER=none` ({@link browserOff}); whether it did. Returns
 * once the opener has been spawned.
 */
export function openInBrowser(url: string, machine: DaemonMachine = THIS_MACHINE): boolean {
  if (browserOff(machine.env)) return false;
  const { command, args, cwd } = browserOpener(url, machine.platform, machine.env);
  const child = machine.spawnShown(command, args, { detached: true, stdio: "ignore", ...(cwd === undefined ? {} : { cwd }) });
  child.once("error", () => {
    process.stderr.write(`rerail: could not open a browser; visit ${url}\n`);
  });
  child.unref();
  return true;
}

/**
 * Where a page ended up: a tab in the current Orca window, the desktop's default browser, or nowhere, under
 * `BROWSER=none` ({@link browserOff}), when the caller shows the link instead.
 */
export type PageOpening = "orca-tab" | "system-browser" | "not-opened";

/** How long Orca gets to acknowledge a tab before the desktop browser takes over. */
const ORCA_TAB_TIMEOUT_MS = 10_000;

/** Where an Orca install puts its CLI; PATH is never consulted so a shadowing `orca` cannot be run. */
const ORCA_EXECUTABLES: readonly string[] = [
  "/usr/local/bin/orca",
  "/opt/homebrew/bin/orca",
  "/Applications/Orca.app/Contents/Resources/bin/orca",
];

/** The first installed Orca CLI among `candidates`, or undefined when Orca is not installed (always on Windows). */
export function orcaExecutable(candidates: readonly string[] = ORCA_EXECUTABLES, platform: NodeJS.Platform = process.platform): string | undefined {
  if (platform === "win32") return undefined;
  return candidates.find(candidate => existsSync(candidate));
}

/** `url` as {@link dashboardLink} makes it for the daemon `home` records on `platform`; as it is when there is none. */
export async function dashboardAccessLink(url: string, home: string, platform: NodeJS.Platform = process.platform): Promise<string> {
  const state = await readDaemonState(join(home, DAEMON_STATE_FILE), platform).catch(() => undefined);
  return state === undefined ? url : dashboardLink(state, url);
}

/**
 * Opens a page of the home daemon's dashboard, with its access capability when it needs one, as a tab in the current
 * Orca window when running under Orca (`ORCA_TAB_ID` set and an Orca CLI installed at one of its fixed locations),
 * otherwise in the desktop browser. Under `BROWSER=none` it opens neither and answers `not-opened`, and the caller
 * shows the link. Never throws: an Orca failure falls back to the desktop browser.
 */
export async function openDashboard(
  page: string,
  machine: DaemonMachine = THIS_MACHINE,
  candidates: readonly string[] = ORCA_EXECUTABLES,
): Promise<PageOpening> {
  const { env, platform } = machine;
  if (browserOff(env)) return "not-opened";
  const url = await dashboardAccessLink(page, defaultRerailHome(env), platform);
  const orca = env.ORCA_TAB_ID === undefined || env.ORCA_TAB_ID === "" ? undefined : orcaExecutable(candidates, platform);
  if (orca === undefined) {
    openInBrowser(url, machine);
    return "system-browser";
  }
  const { promise, resolve } = Promise.withResolvers<boolean>();
  const child = machine.spawn(orca, ["tab", "create", "--url", url], { stdio: "ignore", timeout: ORCA_TAB_TIMEOUT_MS });
  // A spawn failure may or may not be followed by `exit`; the first verdict wins.
  child.once("error", () => resolve(false));
  child.once("exit", code => resolve(code === 0));
  if (await promise) return "orca-tab";
  openInBrowser(url, machine);
  return "system-browser";
}
