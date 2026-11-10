import { createHash, timingSafeEqual } from "node:crypto";
import { closeSync, existsSync, openSync, readFileSync } from "node:fs";
import { mkdir, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { Agent as HttpsAgent, request as httpsRequest, type RequestOptions as HttpsRequestOptions } from "node:https";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { Readable, type Duplex } from "node:stream";
import { connect as tlsConnect, type ConnectionOptions } from "node:tls";
import { setTimeout as sleep } from "node:timers/promises";
import { spawn, spawnShown } from "../engine/child-process.js";
import { renameWithRetry } from "../engine/file-replace.js";
import { windowsSystemProgram } from "../engine/system-programs.js";
import { loadNativeBinding, type NativeBinding } from "../engine/native/native.js";
import { forgetMainInstanceHolder } from "../engine/native/main-instance.js";
import { keepFolderToOwner } from "../engine/storage/windows-acl.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../engine/package-root.js";
import { resolveExecutable } from "./npm.js";
import { ensureSqlite, type SqliteRepairResult } from "./sqlite-repair.js";
import { engineHoldsCapability } from "./engine-proof.js";
import { enginePipeListed, isNamedPipe, listPipes, pipeListed, pipeNameOf } from "./engine-pipe.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { makeRerailHome } from "../engine/storage/private-files.js";
import type { ModelStatus } from "../engine/native/model-status.js";

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
}

/** Header a Rerail process, and the dashboard page, send the daemon's {@link DaemonState.accessToken} in. */
export const DAEMON_ACCESS_HEADER = "x-rerail-access";
/** The query parameter of the dashboard link; the page keeps its value in its origin's storage and sends the header. */
export const DAEMON_ACCESS_PARAM = "rerail-access";
/** In the Rerail home: the capability a loopback daemon keeps across restarts. */
export const DAEMON_ACCESS_TOKEN_FILE = "access-token";

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
}

function engineEndpointOf(state: Pick<DaemonState, "kernelSocketPath" | "accessToken">): EngineEndpoint {
  return state.accessToken === undefined ? { socketPath: state.kernelSocketPath } : { socketPath: state.kernelSocketPath, accessToken: state.accessToken };
}

/**
 * The endpoint for an engine socket named outright (`RERAIL_ENGINE_SOCKET`, or a test's path): with the capability of
 * the home's daemon when its state names that same socket, and none otherwise, which only a standalone engine service
 * (one no daemon started) accepts.
 */
export async function engineEndpointAt(socketPath: string, home: string = defaultRerailHome()): Promise<EngineEndpoint> {
  const state = await readDaemonState(join(home, DAEMON_STATE_FILE)).catch(() => undefined);
  return state !== undefined && resolve(state.kernelSocketPath) === resolve(socketPath) ? engineEndpointOf(state) : { socketPath };
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
      const headers = new Headers();
      for (const [name, value] of Object.entries(response.headers)) {
        for (const item of Array.isArray(value) ? value : [value]) if (item !== undefined) headers.append(name, item);
      }
      const status = response.statusCode ?? 502;
      const empty = method === "HEAD" || NULL_BODY_STATUSES.has(status);
      if (empty) response.resume();
      answer(new Response(empty ? null : Readable.toWeb(response) as unknown as ReadableStream, {
        status, statusText: response.statusMessage ?? "", headers,
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
}

/**
 * Rerail CLI children require Node; a compiled Bun host's execPath may launch the host itself. There the name
 * is resolved through PATH once, so the SQLite probe, its rebuild and the daemon all run under one binary. On Windows
 * only a `node.exe` counts: a version manager's `node.cmd` shim cannot be spawned without a shell, and a bare `node`
 * would be looked for in the current folder first.
 */
export function nodeExecutable(platform: NodeJS.Platform = process.platform): string {
  if (process.versions.bun === undefined) return process.execPath;
  if (platform !== "win32") return resolveExecutable("node") ?? "node";
  const node = resolveExecutable("node", process.env, platform, { binaryOnly: true });
  if (node === undefined) throw new Error("Rerail runs its CLI under Node.js, and no node.exe is on PATH");
  return node;
}

export const DAEMON_STATE_FILE = "daemon.json";
export const DAEMON_LOG_FILE = "daemon.log";
/** In the Rerail home: the lock a daemon start holds. */
export const START_LOCK_DIR = "start.lock";

const START_TIMEOUT_MS = 15_000;
const STOP_TIMEOUT_MS = 10_000;
const KILL_TIMEOUT_MS = 5_000;
const POLL_INTERVAL_MS = 100;
const PROBE_TIMEOUT_MS = 2_000;
const START_LOCK_LEASE_MS = 60_000;
const MAX_MACOS_UNIX_SOCKET_PATH_BYTES = 103;

export interface RunningDaemon {
  state: DaemonState;
  health: DaemonHealth;
}

/**
 * What the state file says, checked against reality.
 *
 * - `running`: the recorded pid is alive and `/healthz` answers with that pid.
 * - `unresponsive`: the pid is alive but the probe failed. The state is kept:
 *   a transient timeout must not orphan a live daemon, and `stop` still needs
 *   the pid. `start` refuses rather than spawning a second instance.
 * - `refused`: the listener answered `401`: it does not accept the capability the state file records (edited, or
 *   written by another daemon). It is running; `start` refuses and says to restart it.
 * - `absent`: no state, or a stale file (pid dead, or a different process
 *   answering on that URL) which has been cleared.
 */
export type DaemonLookup =
  | { status: "running"; state: DaemonState; health: DaemonHealth }
  | { status: "unresponsive"; state: DaemonState }
  | { status: "refused"; state: DaemonState }
  | { status: "absent" };

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
}

/** Whether `pid` is a live process of any user: a process another user runs still holds its pid (`EPERM`). */
export function processAlive(pid: number, kill: ProcessSignal = process.kill.bind(process)): boolean {
  return processLiveness(pid, kill) !== "dead";
}

/**
 * Whether `pid` is a live process this OS user runs, the only kind that can be this user's daemon. A state file whose
 * pid belongs to another user is left over from a daemon that is gone, its pid since reused; probing the URL it records
 * would send the capability to whoever listens there now. On Windows `EPERM` answers alike for another user's
 * process, a SYSTEM service and this user's elevated one, so it counts as not this user's there too; `lookupDaemon`
 * tells this user's elevated daemon apart by its engine's proof.
 */
export function ownProcessAlive(pid: number, kill: ProcessSignal = process.kill.bind(process)): boolean {
  return processLiveness(pid, kill) === "own";
}

/** `process.kill`'s shape, which tests replace. */
export type ProcessSignal = (pid: number, signal?: string | number) => boolean;

function processLiveness(pid: number, kill: ProcessSignal = process.kill.bind(process)): "own" | "other-user" | "dead" {
  let owner: "own" | "other-user" = "own";
  try {
    kill(pid, 0);
  } catch (error) {
    // EPERM means the process exists but this user may not signal it.
    if ((error as NodeJS.ErrnoException).code !== "EPERM") return "dead";
    owner = "other-user";
  }
  return isZombie(pid) ? "dead" : owner;
}

/**
 * A process that has exited but was never reaped still answers signal 0. In a container without an init process
 * nothing reaps a detached daemon, so it stays a zombie for as long as the container runs: it holds no port and
 * no socket, and counts as stopped.
 */
function isZombie(pid: number): boolean {
  if (process.platform !== "linux") return false;
  let stat: string;
  try {
    stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  } catch {
    return false;
  }
  // `pid (comm) state …`: the command name may itself hold spaces and parentheses, so the state follows the last `)`.
  return /^\s*[ZX]/u.test(stat.slice(stat.lastIndexOf(")") + 1));
}

function hasDaemonStateFields(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return Number.isInteger(record.pid)
    && typeof record.url === "string"
    && typeof record.host === "string"
    && Number.isInteger(record.port)
    && typeof record.version === "string"
    && (record.ui === "bundled" || record.ui === "dev")
    && (record.nativeToken === undefined || typeof record.nativeToken === "string")
    && (record.accessToken === undefined || typeof record.accessToken === "string")
    && (record.tlsFingerprint256 === undefined || typeof record.tlsFingerprint256 === "string")
    && typeof record.startedAt === "number"
    && typeof record.packageRoot === "string"
    && (record.workspaceRoot === undefined || typeof record.workspaceRoot === "string");
}

export function isUsableEngineSocketPath(
  value: unknown,
  platform: NodeJS.Platform = process.platform,
): value is string {
  if (typeof value !== "string" || value.length <= 1 || value.includes("\0")) return false;
  if (platform === "win32") return isNamedPipe(value) && pipeNameOf(value).length > 0 && !pipeNameOf(value).includes("\\");
  return isAbsolute(value) && (platform !== "darwin" || Buffer.byteLength(value) <= MAX_MACOS_UNIX_SOCKET_PATH_BYTES);
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

/**
 * What the daemon `state` records answers on `/healthz`, with the capability it records, over its own transport
 * (`daemonFetch`) unless `fetchImpl` names another, as the MCP bridge's tests do. A `401` is a refusal, kept apart from
 * silence: the daemon is up, and "not answering" would send the person after the wrong cause.
 */
export async function probeDaemon(
  state: Pick<DaemonState, "url" | "accessToken" | "tlsFingerprint256">,
  fetchImpl?: DaemonFetch,
  timeoutMs: number = PROBE_TIMEOUT_MS,
): Promise<DaemonProbe> {
  try {
    const endpoint = daemonEndpoint(state, "/healthz");
    const response = await (fetchImpl ?? endpoint.fetch)(endpoint.url, { headers: endpoint.headers, signal: AbortSignal.timeout(timeoutMs) });
    if (response.status === 401) return { status: "refused" };
    if (!response.ok) return { status: "unreachable" };
    const body = await response.json() as DaemonHealth;
    return body.status === "ok" ? { status: "ok", health: body } : { status: "unreachable" };
  } catch {
    return { status: "unreachable" };
  }
}

/** What a daemon lookup asks of the machine; tests pass their own. */
export interface DaemonMachine {
  platform: NodeJS.Platform;
  kill: ProcessSignal;
  /** The named pipes that exist on Windows, read from `\\.\pipe\` without connecting to any. */
  pipes(): readonly string[];
  /** Whether the engine on `socketPath` proves it holds `accessToken`, which it never sees (`engineHoldsCapability`). */
  holdsCapability(socketPath: string, accessToken: string): Promise<boolean>;
  /** Removes the main-instance record when it names `pid` (`forgetMainInstanceHolder`). */
  forgetMainInstanceHolder(pid: number): void;
}

const THIS_MACHINE: DaemonMachine = {
  platform: process.platform,
  kill: (pid, signal) => process.kill(pid, signal),
  pipes: listPipes,
  holdsCapability: async (socketPath, accessToken) => await engineHoldsCapability(socketPath, accessToken),
  forgetMainInstanceHolder: pid => forgetMainInstanceHolder(pid),
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
 * logoff or reboot, so a record it wrote can name a pid Windows has given to another process since.
 */
export function daemonMayHold(pid: number, machine: DaemonMachine = THIS_MACHINE): boolean {
  if (!processAlive(pid, machine.kill)) return false;
  return machine.platform !== "win32" || enginePipeListed(() => machine.pipes());
}

/**
 * Whether the daemon `state` records may be this user's: a live pid, on Windows one holding its engine pipe, and this
 * user's process. On Windows a pid this user may not signal (`EPERM`) can still be this user's daemon, started from an
 * elevated terminal or an elevated agent session; it counts when its engine proves it holds the capability `state`
 * records, which a process on a reused pid cannot. Nothing is sent to its URL before that.
 */
async function recordedDaemonMayBeOwn(state: DaemonState, machine: DaemonMachine): Promise<boolean> {
  const liveness = processLiveness(state.pid, machine.kill);
  if (liveness === "dead") return false;
  if (machine.platform !== "win32") return liveness === "own";
  if (!enginePipeExists(state, machine)) return false;
  return liveness === "own" || (state.accessToken !== undefined && await machine.holdsCapability(state.kernelSocketPath, state.accessToken));
}

/**
 * What the state file in `home` says, checked against the machine. `probeTimeoutMs` bounds the health probe; a hook
 * waiting out a busy daemon passes more (`patientLookup`).
 */
export async function lookupDaemon(
  home: string,
  machine: DaemonMachine = THIS_MACHINE,
  probeTimeoutMs: number = PROBE_TIMEOUT_MS,
): Promise<DaemonLookup> {
  const stateFile = join(home, DAEMON_STATE_FILE);
  const state = await readDaemonState(stateFile, machine.platform);
  if (state === undefined) return { status: "absent" };
  // Another user's process cannot be this user's daemon, and on Windows no daemon lives without its engine pipe: the
  // state is stale, and its URL is never probed.
  if (!(await recordedDaemonMayBeOwn(state, machine))) {
    await rm(stateFile, { force: true });
    return { status: "absent" };
  }
  const probe = await probeDaemon(state, undefined, probeTimeoutMs);
  if (probe.status === "refused") return { status: "refused", state };
  if (probe.status === "unreachable") return { status: "unresponsive", state };
  if (probe.health.pid !== state.pid) {
    // Something else answers on that URL; the recorded daemon is not the listener.
    await rm(stateFile, { force: true });
    return { status: "absent" };
  }
  return { status: "running", state, health: probe.health };
}

/** The healthy daemon recorded in `home`, or undefined when there is none to reuse. */
export async function findRunningDaemon(home: string): Promise<RunningDaemon | undefined> {
  const lookup = await lookupDaemon(home);
  return lookup.status === "running" ? { state: lookup.state, health: lookup.health } : undefined;
}

interface StartLockLease {
  pid: number;
  expiresAt: number;
}

/**
 * Serialize the check-then-spawn window per home. Two sessions starting at
 * once would otherwise both see "absent" and each launch a daemon; the loser
 * would either die on the port or, with `--port 0`, run orphaned while both
 * fight over the state file. A lease held by a dead pid or past its expiry
 * is reclaimed.
 */
export interface StartLockTiming {
  leaseMs: number;
  renewMs: number;
}

const DEFAULT_START_LOCK_TIMING: StartLockTiming = { leaseMs: START_LOCK_LEASE_MS, renewMs: START_LOCK_LEASE_MS / 3 };

async function readStartLockLease(leasePath: string): Promise<StartLockLease | undefined> {
  return await readFile(leasePath, "utf8").then(text => JSON.parse(text) as StartLockLease).catch(() => undefined);
}

export async function withStartLock<T>(
  home: string,
  operation: () => Promise<T>,
  timing: StartLockTiming = DEFAULT_START_LOCK_TIMING,
): Promise<T> {
  const lockPath = join(home, START_LOCK_DIR);
  const leasePath = join(lockPath, "lease.json");
  const deadline = Date.now() + START_TIMEOUT_MS;
  for (;;) {
    try {
      await mkdir(lockPath, { mode: 0o700 });
      break;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      const lease = await readStartLockLease(leasePath);
      const stale = lease === undefined || !processAlive(lease.pid) || lease.expiresAt < Date.now();
      if (stale) {
        await rm(lockPath, { recursive: true, force: true });
        continue;
      }
      if (Date.now() > deadline) throw new Error(`Another Rerail start (pid ${lease.pid}) is still in progress`);
      await sleep(POLL_INTERVAL_MS);
    }
  }
  // Written beside the lease and renamed over it: a rewrite in place truncates first, and a starter that read the
  // empty file between would take the lease for stale and remove the lock this start still holds.
  const writeLease = async (): Promise<void> => {
    const lease: StartLockLease = { pid: process.pid, expiresAt: Date.now() + timing.leaseMs };
    const written = `${leasePath}.${process.pid}.tmp`;
    await writeFile(written, JSON.stringify(lease), { mode: 0o600 });
    await renameWithRetry(written, leasePath);
  };
  const ownsLease = async (): Promise<boolean> => (await readStartLockLease(leasePath))?.pid === process.pid;
  await writeLease();
  // The operation may rebuild a native module, which outlives a single lease; renew while it runs, but never over
  // a lease another starter has since taken because ours looked stale.
  let renewal: Promise<void> = Promise.resolve();
  const renew = setInterval(() => {
    renewal = renewal.then(async () => {
      if (await ownsLease()) await writeLease();
    }).catch(() => undefined);
  }, timing.renewMs);
  renew.unref();
  try {
    return await operation();
  } finally {
    clearInterval(renew);
    await renewal;
    if (await ownsLease()) await rm(lockPath, { recursive: true, force: true });
  }
}

/** The last `lines` lines written to `path` from byte `offset` on: this launch's output, not earlier runs'. */
async function tailFile(path: string, offset: number, lines: number): Promise<string> {
  const text = await readFile(path).then(bytes => bytes.subarray(offset).toString("utf8"), () => "");
  return text.trimEnd().split("\n").slice(-lines).join("\n");
}

export interface DaemonControl {
  home: string;
  packageRoot: string;
  /** Repairs the SQLite binding below `packageRoot` before a daemon is spawned; tests substitute it. */
  ensureSqlite?: (packageRoot: string) => Promise<SqliteRepairResult>;
}

/**
 * The daemon opens SQLite as soon as it starts; a binding built for another Node ABI (a Node upgrade behind a
 * stable Homebrew symlink is the usual cause) kills it before it can answer. Repairing here, in the launcher,
 * puts the rebuild and any unrepairable failure in front of whoever ran `rerail daemon start` rather than in the log.
 */
async function repairSqlite(control: DaemonControl): Promise<void> {
  const repair = control.ensureSqlite ?? (async (packageRoot: string) => await ensureSqlite(packageRoot, {
    executable: nodeExecutable(),
    log: line => process.stderr.write(`${line}\n`),
  }));
  const result = await repair(control.packageRoot);
  if (result.rebuilt) process.stderr.write(`Rebuilt better-sqlite3 in ${result.prefix}\n`);
}

export async function startDaemon(control: DaemonControl, options: DaemonLaunchOptions): Promise<StartDaemonResult> {
  makeRerailHome(control.home);
  return await withStartLock(control.home, async () => {
    const lookup = await lookupDaemon(control.home);
    if (lookup.status === "running") return { started: false, daemon: { state: lookup.state, health: lookup.health } };
    if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
    return await spawnDaemon(control, options);
  });
}

/**
 * Starts a daemon from `control.packageRoot`, replacing a healthy daemon from
 * another managed package. Hooks use this before sending engine service configuration:
 * an older daemon cannot safely accept fields introduced by the active hook.
 */
export async function startDaemonForPackage(control: DaemonControl, options: DaemonLaunchOptions): Promise<StartDaemonResult> {
  makeRerailHome(control.home);
  return await withStartLock(control.home, async () => {
    const lookup = await lookupDaemon(control.home);
    if (lookup.status === "running") {
      if (await daemonUsesPackageRoot(lookup.state, control.packageRoot)) {
        return { started: false, daemon: { state: lookup.state, health: lookup.health } };
      }
      await stopFoundDaemon(control.home, lookup);
      return await spawnDaemon(control, options);
    }
    if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
    return await spawnDaemon(control, options);
  });
}

/** Whether a healthy daemon was launched from the package the caller is executing. */
export async function daemonUsesPackageRoot(daemon: Pick<DaemonState, "packageRoot">, packageRoot: string): Promise<boolean> {
  const canonical = async (path: string): Promise<string> => await realpath(path).catch(() => resolve(path));
  const [runningRoot, expectedRoot] = await Promise.all([canonical(daemon.packageRoot), canonical(packageRoot)]);
  return runningRoot === expectedRoot;
}

/**
 * The engine endpoint main host clients send events to; only the daemon evaluates them, since it alone opens
 * the machine-wide content store. `replaceOtherPackage` brings up this package's daemon in place of one from
 * another package, which is how a hook keeps an older daemon from receiving fields it predates; a client that
 * outlives an update passes false after its first event so it keeps using the newer daemon instead of displacing
 * it. `RERAIL_AUTOSTART=0` starts nothing and requires a running daemon.
 */
export async function daemonEngineEndpoint(replaceOtherPackage: boolean): Promise<EngineEndpoint> {
  const home = defaultRerailHome();
  const packageRoot = PACKAGE_ROOT;
  const control = { home, packageRoot };
  const options = { dev: false };
  let daemon: RunningDaemon | undefined;
  try {
    if (process.env.RERAIL_AUTOSTART === "0") {
      const lookup = await patientLookup(home);
      if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
      daemon = lookup.status === "running" ? { state: lookup.state, health: lookup.health } : undefined;
    } else {
      daemon = (replaceOtherPackage ? await startDaemonForPackage(control, options) : await startDaemon(control, options)).daemon;
    }
  } catch (error) {
    if (!(error instanceof LiveDaemonError) || error.lookup.status !== "unresponsive") throw error;
    const answered = await patientLookup(home, error.lookup);
    if (answered.status !== "running") throw error;
    daemon = { state: answered.state, health: answered.health };
  }
  if (daemon === undefined) throw new Error("RERAIL_AUTOSTART=0 requires a running daemon; run rerail daemon start or set RERAIL_ENGINE_SOCKET");
  if (replaceOtherPackage && !(await daemonUsesPackageRoot(daemon.state, packageRoot))) {
    throw new Error("RERAIL_AUTOSTART=0 requires a daemon from the active Rerail package; run rerail daemon start or set RERAIL_ENGINE_SOCKET");
  }
  return engineEndpointOf(daemon.state);
}

/**
 * The engine endpoint for a hook after a session's start, or undefined when no daemon is alive to take the event. A
 * daemon that is down is not started: each tool call then costs one failed probe instead of an autostart that can
 * outlast the coding agent's hook timeout (design N11). A busy one is waited for (`patientLookup`). A healthy daemon
 * from another package (an update installed mid-session) is replaced as `daemonEngineEndpoint(true)` replaces it, so
 * steering carries on with this package's engine.
 */
export async function runningDaemonEngineEndpoint(): Promise<EngineEndpoint | undefined> {
  const lookup = await patientLookup(defaultRerailHome());
  if (lookup.status !== "running") return undefined;
  return await daemonUsesPackageRoot(lookup.state, PACKAGE_ROOT) ? engineEndpointOf(lookup.state) : await daemonEngineEndpoint(true);
}

/** How long, in all, a hook's second health probe waits for a busy daemon to answer as itself. */
const BUSY_DAEMON_WAIT_MS = 10_000;

/**
 * The daemon lookup a hook acts on. A daemon whose pid is alive but which missed the first probe's 2 s, as one does
 * while its one event loop runs an audit or a long garbage collection, is probed once more with up to
 * `BUSY_DAEMON_WAIT_MS`; only an answer from the recorded pid makes it running. Nothing, the capability included, goes
 * to its engine socket before that answer: a pid Windows reused, or a socket another process holds, is not the daemon.
 */
async function patientLookup(home: string, first?: DaemonLookup): Promise<DaemonLookup> {
  const lookup = first ?? await lookupDaemon(home);
  return lookup.status === "unresponsive" ? await lookupDaemon(home, THIS_MACHINE, BUSY_DAEMON_WAIT_MS) : lookup;
}

/**
 * Starts a daemon process with `args` (its entry script first) the way `rerail daemon start` does, in the Rerail home and
 * through the native spawn on Windows (`launchDaemonProcess`), and returns its pid without waiting for it: a daemon
 * restarting itself hands over this way.
 */
export function relaunchDaemon(args: string[], logFile: string): number | undefined {
  return launchDaemonProcess({ home: defaultRerailHome(), packageRoot: PACKAGE_ROOT }, args, process.env, logFile).pid;
}

export interface LaunchedDaemon {
  pid: number | undefined;
  /** Why the launched process is gone, or undefined while it runs. */
  exitReason: () => string | undefined;
}

function daemonArguments(control: DaemonControl, options: DaemonLaunchOptions, stateFile: string): string[] {
  const args = [
    join(control.packageRoot, ENTRY_POINTS.daemon),
    "--state-file", stateFile,
    "--workspace-root", resolve(options.cwd ?? process.cwd()),
  ];
  if (options.dev) args.push("--dev");
  if (options.host !== undefined) args.push("--host", options.host);
  if (options.allowRemote) args.push("--allow-remote");
  if (options.publicHosts !== undefined && options.publicHosts.length > 0) args.push("--public-host", options.publicHosts.join(","));
  if (options.tls !== undefined) args.push("--tls-cert", options.tls.certFile, "--tls-key", options.tls.keyFile);
  if (options.insecureHttp === true) args.push("--insecure-http");
  if (options.port !== undefined) args.push("--port", String(options.port));
  return args;
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
 * module kept. A binary built before it had no such handle, and there only a pid that no process holds says so.
 */
function detachedExitReason(native: Pick<NativeBinding, "detachedExitCode">, pid: number): string | undefined {
  if (native.detachedExitCode === undefined) return processAlive(pid) ? undefined : "exited";
  const code = native.detachedExitCode(pid);
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
 */
export function launchDetached(launch: DetachedLaunch, platform: NodeJS.Platform = process.platform): LaunchedDaemon {
  if (platform === "win32") {
    const definedEnv = Object.fromEntries(Object.entries(launch.env).filter((entry): entry is [string, string] => entry[1] !== undefined));
    const native = loadNativeBinding();
    const pid = native.spawnDetached(nodeExecutable(), [...launch.args], launch.cwd ?? null, definedEnv, launch.logFile);
    return { pid, exitReason: () => detachedExitReason(native, pid) };
  }
  const log = openSync(launch.logFile, "a", 0o600);
  let child;
  try {
    child = spawn(nodeExecutable(), [...launch.args], { detached: true, stdio: ["ignore", log, log], cwd: launch.cwd, env: launch.env });
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
function launchDaemonProcess(control: DaemonControl, args: string[], env: NodeJS.ProcessEnv, logFile: string): LaunchedDaemon {
  return launchDetached({ args, cwd: control.home, env, logFile });
}

async function spawnDaemon(control: DaemonControl, options: DaemonLaunchOptions): Promise<StartDaemonResult> {
  await repairSqlite(control);
  await mkdir(control.home, { recursive: true, mode: 0o700 });
  // A Rerail home an earlier release made outside the profile folder kept its parent's ACL on Windows, which there lets
  // every local user read the capability and the database; a launch, rare beside hooks, gives it an owner-only one.
  keepFolderToOwner({ folder: control.home });
  const stateFile = join(control.home, DAEMON_STATE_FILE);
  const logFile = join(control.home, DAEMON_LOG_FILE);
  const env = { ...(options.env ?? process.env), RERAIL_HOME: control.home };
  // The log is appended to across runs; a failure shows only what this launch wrote.
  const logOffset = await stat(logFile).then(info => info.size, () => 0);
  const { pid, exitReason } = launchDaemonProcess(control, daemonArguments(control, options, stateFile), env, logFile);

  const deadline = Date.now() + START_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const exit = exitReason();
    if (exit !== undefined) {
      const output = await tailFile(logFile, logOffset, 20);
      throw new Error(`Rerail daemon ${exit} before it was ready${output === "" ? "." : `:\n${output}`}`);
    }
    const state = await readDaemonState(stateFile);
    if (state !== undefined && state.pid === pid) {
      const probe = await probeDaemon(state);
      if (probe.status === "ok" && probe.health.pid === state.pid) return { started: true, daemon: { state, health: probe.health } };
    }
    await sleep(POLL_INTERVAL_MS);
  }
  throw new Error(`Rerail daemon did not become ready within ${START_TIMEOUT_MS / 1_000}s; see ${logFile}`);
}

let autostartAttempted = false;

/**
 * Bring the daemon up from inside a host session so the browser view is
 * live without anyone running `rerail daemon start`. One attempt per process;
 * `RERAIL_AUTOSTART=0` disables it.
 *
 * Truly fire-and-forget: the work is handed to a detached `rerail daemon start`
 * process and nothing here is awaited. Running `startDaemon` in-process
 * would leave its probes and poll timers on the event loop, and a one-shot
 * hook would then sit through daemon startup, or the 15 s timeout, before
 * it could exit. The launcher's own output goes to `daemon.log`; a failure
 * is visible there and through `rerail status`, never in the host. The
 * launcher starts as the daemon does ({@link launchDetached}), so on Windows
 * the SQLite probe, npm and the managed-version dispatch it runs open no window.
 *
 * Every session records into the one database in the Rerail home, and the one
 * daemon serves every repository; sessions in any repository reuse it for their
 * engine service and status.
 */
export function autostartDaemon(platform: NodeJS.Platform = process.platform): void {
  if (autostartAttempted) return;
  autostartAttempted = true;
  const setting = process.env.RERAIL_AUTOSTART;
  if (setting === "0" || setting === "false") return;
  const home = defaultRerailHome();
  try {
    makeRerailHome(home);
    launchDetached({
      args: [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "daemon", "start"],
      cwd: undefined,
      env: process.env,
      logFile: join(home, DAEMON_LOG_FILE),
    }, platform);
  } catch {
    // Autostart is a convenience; the session must never notice it failing.
  }
}

export type StopDaemonResult = "stopped" | "not-running";

/**
 * What to tell the person when the daemon outlives the kill, with a check that runs where they are: Windows has no
 * `ps` in cmd, and PowerShell's `ps` refuses `-p`; `tasklist` works in both.
 */
export function stillRunningMessage(pid: number, platform: NodeJS.Platform = process.platform): string {
  const check = platform === "win32" ? `tasklist /FI "PID eq ${pid}"` : `ps -p ${pid}`;
  return `Rerail daemon pid ${pid} is still running ${KILL_TIMEOUT_MS / 1_000}s after SIGKILL; check it with ${check}, then run rerail daemon stop again.`;
}

/** Sends `signal` to the daemon, saying who to ask when this user may not signal it (`EPERM`). */
function signalDaemon(pid: number, signal: string, machine: DaemonMachine): void {
  try {
    machine.kill(pid, signal);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EPERM") throw error;
    throw new Error(`Rerail daemon pid ${pid} runs as another user or elevated, and this user may not stop it; stop it from there.`, { cause: error });
  }
}

/**
 * Asks the daemon `state` records to close itself over its HTTP API, with its capability and native token; whether it
 * took the request. On Windows `process.kill` ends a process at once, so this request is the only way the daemon's own
 * shutdown runs there: requests from other sessions finish, and it removes its state file and main-instance record.
 */
async function requestDaemonShutdown(state: DaemonState): Promise<boolean> {
  if (state.nativeToken === undefined) return false;
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
 * Stops the daemon `home` records: SIGTERM on macOS and Linux, which runs its shutdown; on Windows, where no signal
 * reaches a handler, a shutdown request (`requestDaemonShutdown`) first. Either way a daemon still running after
 * {@link STOP_TIMEOUT_MS} is killed.
 */
export async function stopDaemon(home: string, machine: DaemonMachine = THIS_MACHINE): Promise<StopDaemonResult> {
  const lookup = await lookupDaemon(home, machine);
  if (lookup.status === "absent") return "not-running";
  await stopFoundDaemon(home, lookup, machine);
  return "stopped";
}

/** A daemon a lookup found alive: running, unresponsive or refusing the capability. */
export type FoundDaemon = Exclude<DaemonLookup, { status: "absent" }>;

/** Stops the daemon `found`, which a {@link lookupDaemon} of `home` returned, as {@link stopDaemon} does. */
export async function stopFoundDaemon(home: string, found: FoundDaemon, machine: DaemonMachine = THIS_MACHINE): Promise<void> {
  const pid = found.state.pid;
  const alive = (): boolean => processAlive(pid, machine.kill);
  if (machine.platform !== "win32" || !(await requestDaemonShutdown(found.state))) signalDaemon(pid, "SIGTERM", machine);
  const deadline = Date.now() + STOP_TIMEOUT_MS;
  while (alive() && Date.now() < deadline) await sleep(POLL_INTERVAL_MS);
  if (alive()) {
    signalDaemon(pid, "SIGKILL", machine);
    const killDeadline = Date.now() + KILL_TIMEOUT_MS;
    while (alive() && Date.now() < killDeadline) await sleep(POLL_INTERVAL_MS);
    // The state file stays, so a later stop can find the process again.
    if (alive()) throw new Error(stillRunningMessage(pid, machine.platform));
  }
  // The daemon removes its own state file and main-instance record on a clean shutdown; one killed, as Windows
  // always does, leaves both behind, and a record naming a pid Windows reuses would keep the uninstall from the cache.
  await rm(join(home, DAEMON_STATE_FILE), { force: true });
  if (machine.platform === "win32") machine.forgetMainInstanceHolder(pid);
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

/** Hands the URL to the desktop's default browser. Returns once the opener has been spawned. */
export function openInBrowser(url: string): void {
  const { command, args, cwd } = browserOpener(url);
  const child = spawnShown(command, args, { detached: true, stdio: "ignore", ...(cwd === undefined ? {} : { cwd }) });
  child.once("error", () => {
    process.stderr.write(`rerail: could not open a browser; visit ${url}\n`);
  });
  child.unref();
}

/** Where a page ended up: a tab in the current Orca window, or the desktop's default browser. */
export type PageOpening = "orca-tab" | "system-browser";

/** How long Orca gets to acknowledge a tab before the desktop browser takes over. */
const ORCA_TAB_TIMEOUT_MS = 10_000;

/** Where an Orca install puts its CLI; PATH is never consulted so a shadowing `orca` cannot be run. */
const ORCA_EXECUTABLES: readonly string[] = [
  "/usr/local/bin/orca",
  "/opt/homebrew/bin/orca",
  "/Applications/Orca.app/Contents/Resources/bin/orca",
];

/** The first installed Orca CLI among `candidates`, or undefined when Orca is not installed (always on Windows). */
export function orcaExecutable(candidates: readonly string[] = ORCA_EXECUTABLES): string | undefined {
  if (process.platform === "win32") return undefined;
  return candidates.find(candidate => existsSync(candidate));
}

/** `url` as {@link dashboardLink} makes it for the daemon `home` records; as it is when there is none. */
export async function dashboardAccessLink(url: string, home: string): Promise<string> {
  const state = await readDaemonState(join(home, DAEMON_STATE_FILE)).catch(() => undefined);
  return state === undefined ? url : dashboardLink(state, url);
}

/**
 * Opens a page of the home daemon's dashboard, with its access capability when it needs one, as a tab in the current
 * Orca window when running under Orca (`ORCA_TAB_ID` set and an Orca CLI installed at one of its fixed locations),
 * otherwise in the desktop browser. Never throws: an Orca failure falls back to the desktop browser.
 */
export async function openDashboard(
  page: string,
  env: NodeJS.ProcessEnv = process.env,
  candidates: readonly string[] = ORCA_EXECUTABLES,
): Promise<PageOpening> {
  const url = await dashboardAccessLink(page, defaultRerailHome(env));
  const orca = env.ORCA_TAB_ID === undefined || env.ORCA_TAB_ID === "" ? undefined : orcaExecutable(candidates);
  if (orca === undefined) {
    openInBrowser(url);
    return "system-browser";
  }
  const { promise, resolve } = Promise.withResolvers<boolean>();
  const child = spawn(orca, ["tab", "create", "--url", url], { stdio: "ignore", timeout: ORCA_TAB_TIMEOUT_MS });
  // A spawn failure may or may not be followed by `exit`; the first verdict wins.
  child.once("error", () => resolve(false));
  child.once("exit", code => resolve(code === 0));
  if (await promise) return "orca-tab";
  openInBrowser(url);
  return "system-browser";
}
