import type { SpawnOptions } from "node:child_process";
import { createHash, timingSafeEqual } from "node:crypto";
import { closeSync, existsSync, openSync, readFileSync, rmSync, statSync } from "node:fs";
import { mkdir, readFile, realpath, rm, stat } from "node:fs/promises";
import { Agent as HttpsAgent, request as httpsRequest, type RequestOptions as HttpsRequestOptions } from "node:https";
import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { Readable, type Duplex } from "node:stream";
import { connect as tlsConnect, type ConnectionOptions } from "node:tls";
import { setTimeout as sleep } from "node:timers/promises";
import { spawn, spawnShown } from "../engine/child-process.js";
import { programOnPath, windowsSystemProgram } from "../engine/system-programs.js";
import { loadNativeBinding } from "../engine/native/native.js";
import { forgetMainInstanceHolder, mainInstanceLockHeld } from "../engine/native/main-instance.js";
import { withHomeLock } from "../engine/native/file-lock.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../engine/package-root.js";
import { ensureSqlite, type SqliteRepairResult } from "./sqlite-repair.js";
import { engineCapabilityProof, type EngineProofAnswer } from "./engine-proof.js";
import { enginePipeListed, isNamedPipe, listPipes, pipeListed, pipeNameOf } from "./engine-pipe.js";
import { processState, type ProcessSignal } from "../engine/process-liveness.js";
import { DAEMON_ACCESS_TOKEN_FILE, DAEMON_LOG_FILE, DAEMON_STATE_FILE, defaultRerailHome } from "../engine/storage/rerail-home.js";
import { keepFolderPrivate, makeRerailHome } from "../engine/storage/private-files.js";
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
/** In the Rerail home: the claim of the one process that launched an autostart (`claimAutostart`). */
export const AUTOSTART_CLAIM_FILE = "autostart.claim";

const START_TIMEOUT_MS = 15_000;
/**
 * How long an autostart claim holds unless a start ends it: a launcher's whole life, its wait for the start lock and
 * then for the daemon.
 */
const AUTOSTART_CLAIM_MS = 2 * START_TIMEOUT_MS;
const STOP_TIMEOUT_MS = 10_000;
const KILL_TIMEOUT_MS = 5_000;
const POLL_INTERVAL_MS = 100;
const PROBE_TIMEOUT_MS = 2_000;
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
export function processAlive(pid: number, kill?: ProcessSignal): boolean {
  return processState(pid, kill) !== "gone";
}

const isString = (value: unknown): boolean => typeof value === "string";
const isOptionalString = (value: unknown): boolean => value === undefined || typeof value === "string";

/** The check each field of daemon.json passes; `kernelSocketPath` is checked for the platform (`isDaemonState`). */
const DAEMON_STATE_FIELDS: Readonly<Record<string, (value: unknown) => boolean>> = {
  pid: Number.isInteger,
  url: isString,
  host: isString,
  port: Number.isInteger,
  version: isString,
  ui: value => value === "bundled" || value === "dev",
  nativeToken: isOptionalString,
  accessToken: isOptionalString,
  tlsFingerprint256: isOptionalString,
  startedAt: value => typeof value === "number",
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
   * never sees (`engineCapabilityProof`).
   */
  capabilityProof(socketPath: string, accessToken: string, timeoutMs: number): Promise<EngineProofAnswer>;
  /** Removes the main-instance record when it names `pid` (`forgetMainInstanceHolder`). */
  forgetMainInstanceHolder(pid: number): void;
  /**
   * Whether a process holds the main-instance lock in `directory` (`mainInstanceLockHeld`); undefined when the native
   * binding does not load.
   */
  mainInstanceLockHeld(directory: string): boolean | undefined;
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
  capabilityProof: async (socketPath, accessToken, timeoutMs) => await engineCapabilityProof(socketPath, accessToken, timeoutMs),
  forgetMainInstanceHolder: pid => forgetMainInstanceHolder(pid),
  mainInstanceLockHeld: directory => mainInstanceLockHeld(directory),
  nodeExecutable: () => nodeExecutable(),
  spawnDetached: nativeSpawnDetached,
  spawn: (command, args, options) => spawn(command, args, options),
  spawnShown,
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
type RecordedDaemon = "alive" | "gone" | "silent";

/** What an engine's answer to the proof makes of the daemon a Windows state records ({@link windowsRecordedDaemon}). */
const PROOF_VERDICT: Readonly<Record<EngineProofAnswer, RecordedDaemon>> = { proved: "alive", refused: "gone", silent: "silent" };

/**
 * Whether the daemon `state` records may still run, as this user's: the one check every reader of a state file makes
 * before it sends the capability the file records to the URL it records. A state file outlives a daemon killed without
 * its cleanup, and the pid it names can since belong to another process, which would get the capability. The pid must
 * be live, and on macOS and Linux this user's to signal: another user's process cannot be this user's daemon. Windows
 * asks more ({@link windowsRecordedDaemon}).
 */
async function recordedDaemon(state: DaemonState, machine: DaemonMachine, timeoutMs: number): Promise<RecordedDaemon> {
  const daemon = processState(state.pid, machine.kill);
  if (daemon === "gone") return "gone";
  if (machine.platform === "win32") return await windowsRecordedDaemon(state, daemon === "running", machine, timeoutMs);
  return daemon === "running" ? "alive" : "gone";
}

/**
 * {@link recordedDaemon} on Windows, for a live pid this user may signal or not (`signalable`). The state's own engine
 * pipe must exist, since a live daemon always holds it, and the engine on it must prove it holds the capability
 * (`engineCapabilityProof`), which never leaves this process. Windows soon reuses a pid, often for another of this
 * user's processes, and any local user can create the pipe's name, so a signalable pid and a listed pipe alone would
 * hand the capability to whoever holds the pipe and the port; and a pid this user may not signal (`EPERM`) can still be
 * this user's daemon, started from an elevated terminal or agent session. An engine silent for `timeoutMs` is `silent`:
 * a daemon busy in an audit is, and dropping its state for that would lose a daemon that still runs. A state without a
 * capability is sent none, and the pid and pipe decide.
 */
async function windowsRecordedDaemon(state: DaemonState, signalable: boolean, machine: DaemonMachine, timeoutMs: number): Promise<RecordedDaemon> {
  if (!enginePipeExists(state, machine)) return "gone";
  if (state.accessToken === undefined) return signalable ? "alive" : "gone";
  return PROOF_VERDICT[await machine.capabilityProof(state.kernelSocketPath, state.accessToken, timeoutMs)];
}

/**
 * Whether the daemon `state` records may be sent the capability now ({@link recordedDaemon}); one whose engine is
 * silent may not.
 */
export async function recordedDaemonAlive(state: DaemonState, machine: DaemonMachine = THIS_MACHINE): Promise<boolean> {
  return await recordedDaemon(state, machine, PROBE_TIMEOUT_MS) === "alive";
}

/**
 * What the state file in `home` says, checked against the machine. `probeTimeoutMs` bounds the engine's proof and the
 * health probe together; a hook waiting out a busy daemon passes more (`patientLookup`).
 */
export async function lookupDaemon(
  home: string,
  machine: DaemonMachine = THIS_MACHINE,
  probeTimeoutMs: number = PROBE_TIMEOUT_MS,
): Promise<DaemonLookup> {
  const stateFile = join(home, DAEMON_STATE_FILE);
  const state = await readDaemonState(stateFile, machine.platform);
  if (state === undefined) return { status: "absent" };
  const deadline = Date.now() + probeTimeoutMs;
  const recorded = await recordedDaemon(state, machine, probeTimeoutMs);
  // Another user's process cannot be this user's daemon, and on Windows no daemon lives without its engine pipe or
  // fails its engine's proof: the state is stale, and its URL is never probed.
  if (recorded === "gone") {
    await rm(stateFile, { force: true });
    return { status: "absent" };
  }
  // A busy daemon is waited for, not forgotten; its URL is not probed with a capability its engine has not proved.
  if (recorded === "silent") return { status: "unresponsive", state };
  return await probedLookup(stateFile, state, Math.max(0, deadline - Date.now()));
}

/** What the health probe of the daemon `state` records, which may be sent the capability, makes of it within `timeoutMs`. */
async function probedLookup(stateFile: string, state: DaemonState, timeoutMs: number): Promise<DaemonLookup> {
  const probe = await probeDaemon(state, undefined, timeoutMs);
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
export async function findRunningDaemon(home: string, machine: DaemonMachine = THIS_MACHINE): Promise<RunningDaemon | undefined> {
  const lookup = await lookupDaemon(home, machine);
  return lookup.status === "running" ? { state: lookup.state, health: lookup.health } : undefined;
}

export interface StartLockOptions {
  /** Waits between tries while another start holds the lock. */
  sleep?: (ms: number) => Promise<void>;
}

/**
 * Serialize the check-then-spawn window per home. Two sessions starting at
 * once would otherwise both see "absent" and each launch a daemon; the loser
 * would either die on the port or, with `--port 0`, run orphaned while both
 * fight over the state file. The lock is the OS lock `locks/start.lock`
 * (`withHomeLock`), free the moment a start holding it ends, however it ends.
 */
export async function withStartLock<T>(home: string, operation: () => Promise<T>, options: StartLockOptions = {}): Promise<T> {
  return await withHomeLock(home, {
    name: START_LOCK,
    waitMs: START_TIMEOUT_MS,
    sleep: options.sleep,
    busy: holder => new Error(`Another Rerail start (${holder === undefined ? "starting" : `pid ${holder.pid}`}) is still in progress`),
  }, operation);
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
async function repairSqlite(control: DaemonControl, machine: DaemonMachine): Promise<void> {
  const repair = control.ensureSqlite ?? (async (packageRoot: string) => await ensureSqlite(packageRoot, {
    executable: machine.nodeExecutable(),
    log: line => process.stderr.write(`${line}\n`),
  }));
  const result = await repair(control.packageRoot);
  if (result.rebuilt) process.stderr.write(`Rebuilt better-sqlite3 in ${result.prefix}\n`);
}

/**
 * Runs `start` under the start lock in `home`. A start that leaves a daemon answering ends the autostart claim
 * (`claimAutostart`): the launch it held other processes off for is done, so the next process to find the daemon gone
 * starts it again at once. A start that fails keeps the claim, which holds off launches for the rest of its 30 s
 * instead of letting every process that finds the daemon down launch a start that fails too.
 */
async function startUnderLock(home: string, start: () => Promise<StartDaemonResult>): Promise<StartDaemonResult> {
  makeRerailHome(home);
  const result = await withStartLock(home, start);
  // A claim that cannot be removed holds only until it is 30 s old.
  await rm(join(home, AUTOSTART_CLAIM_FILE), { force: true }).catch(() => undefined);
  return result;
}

export async function startDaemon(
  control: DaemonControl,
  options: DaemonLaunchOptions,
  machine: DaemonMachine = THIS_MACHINE,
): Promise<StartDaemonResult> {
  return await startUnderLock(control.home, async () => {
    const lookup = await lookupDaemon(control.home, machine);
    if (lookup.status === "running") return { started: false, daemon: { state: lookup.state, health: lookup.health } };
    if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
    return await spawnDaemon(control, options, machine);
  });
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
  return await startUnderLock(control.home, async () => {
    const lookup = await lookupDaemon(control.home, machine);
    if (lookup.status === "running") {
      if (await daemonUsesPackageRoot(lookup.state, control.packageRoot)) {
        return { started: false, daemon: { state: lookup.state, health: lookup.health } };
      }
      await stopFoundDaemon(control.home, lookup, machine);
      return await spawnDaemon(control, options, machine);
    }
    if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
    return await spawnDaemon(control, options, machine);
  });
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

/** No daemon is running, and `RERAIL_AUTOSTART=0` kept the caller from starting one. */
export class AutostartOffError extends Error {
  override readonly name = "AutostartOffError";
}

/**
 * What `rerail status` says when no daemon runs. Under `RERAIL_AUTOSTART=0` it also says that nothing is recorded,
 * since hooks then start no daemon and log nothing about the missing one: that is what the person asked for.
 */
export function daemonNotRunningMessage(env: NodeJS.ProcessEnv = process.env): string {
  return autostartOff(env)
    ? `The Rerail daemon is not running, and RERAIL_AUTOSTART=${env.RERAIL_AUTOSTART} keeps hooks from starting it, so nothing is recorded. Start it with rerail daemon start.`
    : "The Rerail daemon is not running. Start it with rerail daemon start.";
}

/**
 * The engine endpoint main host clients send events to; only the daemon evaluates them, since it alone opens
 * the machine-wide content store. `replaceOtherPackage` brings up this package's daemon in place of one from
 * another package, which is how a hook keeps an older daemon from receiving fields it predates; a client that
 * outlives an update passes false after its first event so it keeps using the newer daemon instead of displacing
 * it. `RERAIL_AUTOSTART=0` starts nothing and requires a running daemon, and throws `AutostartOffError` without
 * one. A daemon that is alive but busy is waited for (`patientLookup`), until `deadline` (epoch ms) when the caller
 * names one, as a hook does.
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
  if (daemon === undefined) throw new AutostartOffError("RERAIL_AUTOSTART=0 requires a running daemon; run rerail daemon start or set RERAIL_ENGINE_SOCKET");
  if (replaceOtherPackage && !(await daemonUsesPackageRoot(daemon.state, control.packageRoot))) {
    throw new Error("RERAIL_AUTOSTART=0 requires a daemon from the active Rerail package; run rerail daemon start or set RERAIL_ENGINE_SOCKET");
  }
  return engineEndpointOf(daemon.state);
}

/**
 * With `RERAIL_AUTOSTART=0`, the daemon running in `home`, if any. The lookup has already waited out a busy daemon
 * (`patientLookup`); one that still does not answer, or refuses the capability, throws `LiveDaemonError` and is not
 * waited for again.
 */
async function runningDaemon(home: string, deadline: number | undefined, machine: DaemonMachine): Promise<RunningDaemon | undefined> {
  const lookup = await patientLookup(home, machine, deadline);
  if (lookup.status === "unresponsive" || lookup.status === "refused") throw new LiveDaemonError(lookup);
  return lookup.status === "running" ? { state: lookup.state, health: lookup.health } : undefined;
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
): Promise<RunningDaemon> {
  const start = replaceOtherPackage ? startDaemonForPackage : startDaemon;
  try {
    return (await start(control, { dev: false }, machine)).daemon;
  } catch (error) {
    if (!(error instanceof LiveDaemonError) || error.lookup.status !== "unresponsive") throw error;
    const answered = await patientLookup(control.home, machine, deadline, error.lookup);
    if (answered.status !== "running") throw error;
    return { state: answered.state, health: answered.health };
  }
}

/**
 * The daemon a hook found after a session's start: its engine endpoint, or why no daemon takes the event. `absent`
 * means none runs; `unresponsive` and `refused` name a daemon that is alive, which a start would only refuse to replace.
 */
export type HookDaemon = { status: "running"; endpoint: EngineEndpoint } | Exclude<DaemonLookup, { status: "running" }>;

/**
 * The engine endpoint for a hook after a session's start, or why there is none. A daemon that is down is not started:
 * each tool call then costs one failed probe instead of an autostart that can outlast the coding agent's hook timeout
 * (design N11). A busy one is waited for (`patientLookup`), until `deadline` when the hook names one. A healthy daemon
 * from another package (an update installed mid-session) is replaced as `daemonEngineEndpoint(true)` replaces it, so
 * steering carries on with this package's engine.
 */
export async function runningDaemonEngineEndpoint(deadline?: number, machine: DaemonMachine = THIS_MACHINE): Promise<HookDaemon> {
  const lookup = await patientLookup(defaultRerailHome(machine.env), machine, deadline);
  if (lookup.status !== "running") return lookup;
  const endpoint = await daemonUsesPackageRoot(lookup.state, PACKAGE_ROOT)
    ? engineEndpointOf(lookup.state)
    : await daemonEngineEndpoint(true, deadline, machine);
  return { status: "running", endpoint };
}

/** How long, in all, a hook's second health probe waits for a busy daemon to answer as itself. */
const BUSY_DAEMON_WAIT_MS = 10_000;
/** How long, in all, a hook waits for a busy daemon from the event's arrival: the first probe and the busy wait after it. */
export const HOOK_DAEMON_WAIT_MS = PROBE_TIMEOUT_MS + BUSY_DAEMON_WAIT_MS;

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
  /** Why the launched process is gone, or undefined while it runs. */
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
 * module kept. A binary built before it had no such handle, and there only a pid that no process holds says so.
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
    return { pid: launched.pid, exitReason: () => detachedExitReason(launched, machine.kill) };
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

async function spawnDaemon(control: DaemonControl, options: DaemonLaunchOptions, machine: DaemonMachine): Promise<StartDaemonResult> {
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
  const { pid, exitReason } = launchDaemonProcess(control, { args, env, logFile }, machine);
  return { started: true, daemon: await readyDaemon({ pid, exitReason }, { stateFile, logFile, logOffset, platform: machine.platform }) };
}

/**
 * The daemon `launched` started, once its state file names it and it answers its probe as itself. A launch that exits
 * first, or does not answer within `START_TIMEOUT_MS`, fails with what it wrote to the log from `logOffset` on.
 */
async function readyDaemon(
  { pid, exitReason }: LaunchedDaemon,
  { stateFile, logFile, logOffset, platform }: { stateFile: string; logFile: string; logOffset: number; platform: NodeJS.Platform },
): Promise<RunningDaemon> {
  const deadline = Date.now() + START_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const exit = exitReason();
    if (exit !== undefined) {
      const output = await tailFile(logFile, logOffset, 20);
      throw new Error(`Rerail daemon ${exit} before it was ready${output === "" ? "." : `:\n${output}`}`);
    }
    const daemon = await answeringDaemon(stateFile, pid, platform);
    if (daemon !== undefined) return daemon;
    await sleep(POLL_INTERVAL_MS);
  }
  throw new Error(`Rerail daemon did not become ready within ${START_TIMEOUT_MS / 1_000}s; see ${logFile}`);
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

/** Creates `path` if nothing is there; whether this call made it. */
function createOnce(path: string): boolean {
  try {
    closeSync(openSync(path, "wx", 0o600));
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EEXIST") return false;
    throw error;
  }
}

/**
 * Whether this process launches the autostart: it created the claim in `home`, or found one made more than
 * `AUTOSTART_CLAIM_MS` from now and replaced it. Sessions and hooks each run in a process of their own, and the ones
 * that find the daemon gone arrive together; the claim is created exclusively, so one launches and the rest leave the
 * start to it. A start that leaves a daemon answering removes the claim (`startUnderLock`). The distance counts both
 * ways, so a claim made before the clock moved back expires too. Two processes that both replace an expired claim can
 * both launch; the start lock still lets only one launcher start a daemon.
 */
function claimAutostart(home: string): boolean {
  const claim = join(home, AUTOSTART_CLAIM_FILE);
  if (createOnce(claim)) return true;
  const held = statSync(claim, { throwIfNoEntry: false });
  if (held !== undefined && Math.abs(Date.now() - held.mtimeMs) < AUTOSTART_CLAIM_MS) return false;
  rmSync(claim, { force: true });
  return createOnce(claim);
}

/**
 * Bring the daemon up from inside a host session so the browser view is
 * live without anyone running `rerail daemon start`. One attempt per process,
 * and one launch per Rerail home while its claim holds (`claimAutostart`);
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
  if (autostartAttempted) return;
  autostartAttempted = true;
  if (autostartOff(machine.env)) return;
  const home = defaultRerailHome(machine.env);
  try {
    makeRerailHome(home);
    if (!claimAutostart(home)) return;
    launchDetached({
      args: [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "daemon", "start", "--workspace-root", process.cwd()],
      cwd: home,
      env: machine.env,
      logFile: join(home, DAEMON_LOG_FILE),
    }, machine);
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

/** Asks the daemon `state` records to shut down: SIGTERM, or on Windows a shutdown request, SIGTERM only if it is refused. */
async function askDaemonToStop(state: DaemonState, machine: DaemonMachine): Promise<void> {
  if (machine.platform !== "win32" || !(await requestDaemonShutdown(state, machine))) signalDaemon(state.pid, "SIGTERM", machine);
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
  await askDaemonToStop(found.state, machine);
  if (!(await goneWithin(alive, STOP_TIMEOUT_MS))) {
    signalDaemon(pid, "SIGKILL", machine);
    // The state file stays, so a later stop can find the process again.
    if (!(await goneWithin(alive, KILL_TIMEOUT_MS))) throw new Error(stillRunningMessage(pid, machine.platform));
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
