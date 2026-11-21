import { createPrivateKey, randomBytes, X509Certificate } from "node:crypto";
import { constants, createReadStream, existsSync, realpathSync } from "node:fs";
import { open, readFile, stat, rm, type FileHandle } from "node:fs/promises";
import { createServer, type RequestListener, type Server as HttpServer } from "node:http";
import { createServer as createHttpsServer } from "node:https";
import { createSecureContext } from "node:tls";
import type { AddressInfo, Server as NetServer } from "node:net";
import { basename, dirname, extname, join, normalize, resolve, sep } from "node:path";
import { tmpdir, userInfo } from "node:os";
import { Readable } from "node:stream";
import { fileURLToPath } from "node:url";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { spawn } from "../engine/child-process.js";
import { replaceFile } from "../engine/file-replace.js";
import { prepareDaemonGit } from "../engine/git.js";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import { DAEMON_ACCESS_TOKEN_FILE, DAEMON_LOG_FILE, relaunchDaemon, sameSecret, type DaemonHealth, type DaemonLaunchOptions, type DaemonState, type DaemonTlsFiles, type NativeStatus } from "../runtime/daemon-control.js";
import { daemonExit } from "./daemon-exit.js";
import { watchHeap } from "./heap-ceiling.js";
import { startRemoteSink, type RemoteSinkLoop } from "../runtime/remote-outbox.js";
import { chooseEngineSocketPath, engineWorktreeWalks, startEngineService, type EngineService } from "../runtime/engine-service.js";
import { engineIdentity, enginePipeName, fallbackPipe } from "../runtime/daemon/engine-pipe.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import { createMergeAssistService, type MergeAssistRuntime } from "../runtime/merge-assist/service.js";
import { NativeSessionServer } from "./native-session-server.js";
import { startCheckoutPruning, type CheckoutPruning } from "./checkout-pruning.js";
import type { RepositoryWorktreeWalks } from "../engine/code-rule/worktree-walks.js";
import { getRequestListener } from "@hono/node-server";
import { Hono } from "hono";
import { HTTPException } from "hono/http-exception";
import { accessCapability, createRerailApi, hostAllowlist, publicHostSet, requestBodyLimit } from "../api/http.js";
import { createRerailMcpHandler } from "../api/mcp.js";
import type { RerailApiStores } from "../api/operation.js";
import { operations } from "../api/operations.js";
import { DatabaseApiStores } from "../api/stores.js";
import { nativeHealth as loadNativeHealth, type NativeHealth } from "../engine/native/native.js";
import { modelStatus } from "../engine/native/model-status.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { defaultRerailDatabasePath, type RerailDatabase } from "../engine/storage/database.js";
import { moveRuleProvenancesInWorker } from "../engine/storage/rule-provenance-move.js";
import { enabledExperiments, mergeAssistExperimentEnabled, remoteSinkEnabled } from "../engine/experimental-features.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { openToOthers, type AccessReader } from "../engine/storage/private-files.js";
import { holdHomeDaemonLock } from "../engine/native/home-daemon-lock.js";
import { acquireMainInstance } from "../engine/native/main-instance.js";
import { currentRerailBuild, isMainRerailBuild } from "../engine/native/rerail-build.js";

export const DEFAULT_DAEMON_HOST = "127.0.0.1";
export const DEFAULT_DAEMON_PORT = 4737;
/** In the Rerail home: the port the daemon last listened on, which the next start without `--port` prefers. */
export const DAEMON_PORT_FILE = "daemon-port.json";
/** How often the daemon of a home checks that daemon.json still holds its record (`RerailDaemonOptions.recordCheck`). */
const RECORD_CHECK_MS = 1_000;

/**
 * The largest body `/api` and `/mcp` take. Their largest legitimate request is a rule document (a library rule
 * upsert, or the MCP tool call carrying one): conversation rule text is capped at 32 KiB a field and a code rule is a
 * query, not source, so real documents are tens of KiB. 4 MiB leaves room for any of them.
 */
const API_BODY_LIMIT_BYTES = 4 * 1024 * 1024;
/**
 * The largest body `/native/session` takes. Its `initialize`, `semantic` and `cosine` requests carry the host
 * session's whole branch (every entry, tool output included), which reaches tens of MiB in a long session; 256 MiB
 * stays below the size V8 can hold as one string to parse.
 */
const NATIVE_BODY_LIMIT_BYTES = 256 * 1024 * 1024;
/**
 * How long a client has to send its headers, then its whole request. Both bound receiving the request only, never how
 * long a handler takes to answer, and `/mcp` answers each POST with one JSON body (no SSE stream: `mcp.ts`), so no
 * response is cut short. Dashboard polls are small GETs; between them a kept-alive connection is closed by Node's
 * `keepAliveTimeout`, not these. Loopback carries the largest native body in well under 60 seconds.
 */
const HTTP_TIMEOUTS = { headersTimeout: 10_000, requestTimeout: 60_000 } as const;
const INTERNAL_ERROR_MESSAGE = "The Rerail daemon could not answer this request; daemon.log in the Rerail home has the cause.";

export interface RerailDaemonOptions {
  version: string;
  /** Internal testing: false keeps the remote sink's sender off; tests may shorten its interval. */
  remoteSink?: false | { intervalMs: number };
  /** False keeps the hourly forgetting of removed checkouts off ({@link startCheckoutPruning}); tests may shorten its interval. */
  checkoutPruning?: false | { intervalMs: number };
  /** Defaults to `$RERAIL_HOME/rerail.sqlite`; embedding and tests pass their own. */
  databasePath?: string;
  /** Checkout native sessions address; defaults to the current directory. The API serves every repository. */
  workspaceRoot?: string;
  host?: string;
  /** Hostnames besides loopback that requests may be addressed to; anything else is answered 421. */
  publicHosts?: readonly string[];
  /** `0` asks the OS for a free port; omitted, the remembered port, else the default one, unless another process holds it. */
  port?: number;
  /** Where the port the daemon listens on is remembered, so dashboard links outlive a restart. */
  portFile?: string;
  /**
   * Where a loopback daemon keeps its access capability across restarts. Omitted, or for a daemon reachable from other
   * machines (bound off loopback, or answering a `--public-host`), every start mints a new one.
   */
  accessTokenFile?: string;
  /** The access capability to require, in place of a recorded or minted one; embedding and tests pass their own. */
  accessToken?: string;
  /** Directory holding the page. Omit to serve `/api` and `/healthz` alone. */
  uiRoot?: string;
  /** `uiRoot` is the committed `ui/` source rather than the packaged copy; reported as `ui: "dev"`. */
  dev?: boolean;
  /** Written once the server is listening and removed on close, while it still holds what this daemon wrote. */
  stateFile?: string;
  /**
   * Every `everyMs` the daemon checks that `stateFile` still holds what it wrote, and writes it again when it is gone or
   * differs, while `holdsHome` says it still holds the Rerail home at `home`. The daemon of a home passes it
   * (`runHomeDaemon`), since it alone holds the home's lock; omitted, the state file is written once.
   */
  recordCheck?: { readonly everyMs: number; readonly home: string; readonly holdsHome: () => boolean };
  nativeHealth?: () => NativeHealth;
  /** Local IPC endpoint for the engine service; mainly useful for isolated embedding and tests. */
  kernelSocketPath?: string;
  /**
   * The platform whose default engine endpoint the daemon takes when `kernelSocketPath` names none
   * (`defaultEngineSocketPath`): a named pipe on Windows, a Unix socket elsewhere. Tests take Windows' on any OS.
   */
  enginePlatform?: NodeJS.Platform;
  /** The coding agent check `GET /api/coding-agents` reports; the daemon's entry point passes set-up's. */
  codingAgents?: () => Promise<CodingAgentState[]>;
  /**
   * Keeps what a coding agent's status line shows drawn ahead of time, from the daemon's `stores`, until the daemon
   * closes and stops it. The daemon's entry point passes presentation's (`keepClaudeStatusFiles`), a layer above the
   * daemon's; omitted, nothing is drawn ahead.
   */
  statusLines?: (stores: RerailApiStores) => { stop(): Promise<void> };
  /**
   * The certificate (a PEM chain, leaf first) and private key to serve https with; omitted, the daemon serves http.
   * The state file records the leaf's SHA-256 fingerprint, which Rerail's own clients pin.
   */
  tls?: DaemonTlsMaterial;
  /**
   * Why a daemon started with a TLS key serves this computer alone, over plain http on loopback: other accounts can read
   * the key (`serveRerailDaemon`). Its arguments asked for https, and may have asked to be reachable from other
   * machines. The state file records it, for `rerail status` to say. It also names a key that is empty or no key.
   */
  remoteRefused?: string;
  /**
   * Called when a Rerail process asks the daemon to stop (`POST /native/shutdown`, which `rerail daemon stop` sends on
   * Windows, where no signal reaches a handler). Omitted, the daemon serves no such request.
   */
  onShutdownRequest?: () => void;
}

/** What `--tls-cert` and `--tls-key` name, read: PEM text, which `loadDaemonTls` parses before it hands it on. */
export interface DaemonTlsMaterial {
  cert: string;
  key: string;
}

const CONTENT_TYPES: Readonly<Record<string, string>> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8",
};

function readNativeStatus(probe: () => NativeHealth): NativeStatus {
  try {
    const health = probe();
    return health.ok
      ? { ok: true, kernelVersion: health.kernelVersion }
      : { ok: false, error: `Native kernel ${health.kernelVersion} reported unhealthy` };
  } catch (error) {
    return { ok: false, error: (error as Error).message };
  }
}

/** The built page, `<package>/dist/ui`; the UI is not present until the package is built. */
export function bundledUiRoot(): string {
  return join(PACKAGE_ROOT, "dist", "ui");
}

/** The committed page itself, `<package>/ui`: editing a file and reloading is the whole dev loop. */
export function sourceUiRoot(): string {
  return join(PACKAGE_ROOT, "ui");
}

/**
 * A short, database-derived local IPC endpoint isolates each Rerail home's engine service. On POSIX it lives in this
 * user's own directory in the temp directory, `rerail-<uid>`, which the engine service makes 0700 and checks before
 * listening; the daemon falls back to a `run` folder beside the database when that one is refused. Names stay short:
 * a socket path must fit 104 bytes on macOS and 108 on Linux.
 */
export function defaultEngineSocketPath(
  databasePath: string,
  platform: NodeJS.Platform = process.platform,
): string {
  return platform === "win32"
    ? enginePipeName(databasePath)
    : join(tmpdir(), `rerail-${process.getuid!()}`, `${engineIdentity(databasePath)}.sock`);
}

/** The listener the daemon serves on: `node:http`'s, or `node:https`'s when it has a certificate. */
type DaemonServer = NetServer & Pick<HttpServer, "closeIdleConnections" | "closeAllConnections">;

async function listen(server: DaemonServer, port: number, host: string): Promise<void> {
  const listening = Promise.withResolvers<void>();
  server.once("error", listening.reject);
  server.listen(port, host, () => {
    server.off("error", listening.reject);
    listening.resolve();
  });
  await listening.promise;
}

/**
 * An explicit port is bound or fails. Without one the daemon prefers the port it last listened on, then the default
 * port, and when those cannot be had, takes a free port instead; clients find it through the state file, never by the
 * port number. Held (EADDRINUSE) or refused (EACCES: Windows reserves port ranges for Hyper-V and moves them across
 * restarts) both move on; the remembered port moves on whatever its error, since only a deleted file would undo it.
 */
async function listenPreferred(server: DaemonServer, port: number | undefined, remembered: number | undefined, host: string): Promise<void> {
  if (port !== undefined) return await listen(server, port, host);
  for (const preferred of new Set([remembered ?? DEFAULT_DAEMON_PORT, DEFAULT_DAEMON_PORT])) {
    try {
      return await listen(server, preferred, host);
    } catch (error) {
      const code = error instanceof Error && "code" in error ? error.code : undefined;
      if (preferred !== remembered && code !== "EADDRINUSE" && code !== "EACCES") throw error;
    }
  }
  await listen(server, 0, host);
}

async function readRememberedPort(portFile: string): Promise<number | undefined> {
  try {
    const value: unknown = JSON.parse(await readFile(portFile, "utf8"));
    const port = typeof value === "object" && value !== null && "port" in value ? value.port : undefined;
    return typeof port === "number" && Number.isInteger(port) && port > 0 && port <= 65_535 ? port : undefined;
  } catch {
    return undefined;
  }
}

const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/**
 * What the record check found in place of the daemon's own record, for its log line. It names the pid another record
 * names, the process a person may have to end, and never the rest of the file, which holds the capabilities.
 */
function foundInstead(text: string | undefined): string {
  if (text === undefined) return "was gone";
  try {
    const { pid } = JSON.parse(text) as { pid?: unknown };
    if (typeof pid === "number") return `named pid ${pid}`;
  } catch {
    // Not JSON, or JSON null: a record no reader takes, named as such below.
  }
  return "held another record";
}

/**
 * The whole error for the daemon log, as a failed request's is written there: its stack, which says where it was
 * thrown, else its message. An error a worker thread threw arrives with the stack it had in that thread.
 */
const errorStack = (error: unknown): string => (error instanceof Error ? error.stack ?? error.message : String(error));

/** Remembering the port is a convenience: a failed write leaves the daemon running and is only reported. */
async function rememberPort(portFile: string, port: number): Promise<void> {
  try {
    await replaceFile(portFile, `${JSON.stringify({ port })}\n`, { mode: 0o600 });
  } catch (error) {
    process.stderr.write(`rerail daemon: could not remember port ${port} in ${portFile}: ${errorText(error)}\n`);
  }
}

const ACCESS_TOKEN_PATTERN = /^[A-Za-z0-9_-]{43}$/u;

/** A fresh 256-bit capability, URL-safe. */
function newAccessToken(): string {
  return randomBytes(32).toString("base64url");
}

/**
 * The capability recorded in `file`, minted and written (mode 0600, the file's whole content) when there is none or it
 * is unreadable or malformed; a replaced one is reported, since every browser that held it must open a new link. A
 * write that fails leaves a per-launch capability: the daemon still runs, every request still needs it, and the
 * browser asks for a new link after the next restart.
 */
async function persistentAccessToken(file: string): Promise<string> {
  try {
    const recorded = (await readFile(file, "utf8")).trim();
    if (ACCESS_TOKEN_PATTERN.test(recorded)) return recorded;
    process.stderr.write(`rerail daemon: ${file} does not hold an access capability; replacing it, so open the dashboard link again\n`);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") process.stderr.write(`rerail daemon: could not read ${file}: ${errorText(error)}\n`);
  }
  const token = newAccessToken();
  try {
    await replaceFile(file, `${token}\n`, { mode: 0o600 });
  } catch (error) {
    process.stderr.write(`rerail daemon: could not keep the access capability in ${file}: ${errorText(error)}\n`);
  }
  return token;
}

async function closeHttpServer(server: DaemonServer): Promise<void> {
  server.closeIdleConnections();
  const closed = Promise.withResolvers<void>();
  server.close((error) => (error === undefined ? closed.resolve() : closed.reject(error)));
  server.closeAllConnections();
  await closed.promise;
}

/** The hosts the daemon binds and answers to, and whether it is reachable off loopback. */
function daemonHosts(options: RerailDaemonOptions): { bindHost: string; publicHosts: ReturnType<typeof publicHostSet>; remote: boolean } {
  const bindHost = options.host ?? DEFAULT_DAEMON_HOST;
  const loopbackBind = LOOPBACK_BIND_HOSTS.includes(bindHost.toLowerCase());
  // A daemon bound off loopback (`--allow-remote`) is dialled by its own literal address, e.g. a
  // `http://0.0.0.0:4737/` state URL. An IP literal cannot arrive by DNS rebinding —
  // that attack's `Host` is a DNS name — so the bound address is as safe to answer as loopback.
  const publicHosts = publicHostSet([
    ...(options.publicHosts ?? []),
    ...(loopbackBind ? [] : [bindHost.includes(":") ? `[${bindHost}]` : bindHost]),
  ]);
  return { bindHost, publicHosts, remote: !loopbackBind || (options.publicHosts ?? []).length > 0 };
}

/**
 * Every request carries the daemon's access capability: `Host`/`Origin` are only what a client claims, and on
 * loopback another user of the machine, or a sandboxed process allowed localhost, reaches the socket as the person
 * would. A loopback daemon keeps its capability in the home across restarts, so a browser that holds it keeps
 * working; one reachable off loopback (`--allow-remote`, or a `--public-host` a reverse proxy forwards) mints a new
 * one on every start.
 */
async function daemonAccessToken(options: RerailDaemonOptions, remote: boolean): Promise<string> {
  return options.accessToken
    ?? (remote || options.accessTokenFile === undefined ? newAccessToken() : await persistentAccessToken(options.accessTokenFile));
}

function daemonUi(options: RerailDaemonOptions): { uiRoot: string | undefined; ui: DaemonState["ui"] } {
  const uiRoot = options.uiRoot === undefined ? undefined : realpathSync(options.uiRoot);
  return { uiRoot, ui: uiRoot === undefined || options.dev === true ? "dev" : "bundled" };
}

/** What the HTTP app serves. `health` asks the daemon the app ends up behind. */
function daemonApp({ stores, options, publicHosts, accessToken, nativeToken, sessions, workspaceRoot, uiRoot, health }: {
  stores: DatabaseApiStores; options: RerailDaemonOptions; publicHosts: ReturnType<typeof publicHostSet>; accessToken: string; nativeToken: string;
  sessions: NativeSessionServer; workspaceRoot: string; uiRoot: string | undefined; health: () => ReturnType<RerailDaemon["health"]>;
}): Hono {
  const mcp = createRerailMcpHandler(stores, operations, { version: options.version, publicHosts });
  const app = new Hono();
  app.use("*", hostAllowlist(publicHosts));
  app.use("*", accessCapability(accessToken));
  app.use("/api/*", requestBodyLimit(API_BODY_LIMIT_BYTES));
  app.use("/mcp", requestBodyLimit(API_BODY_LIMIT_BYTES));
  app.use("/native/*", requestBodyLimit(NATIVE_BODY_LIMIT_BYTES));
  app.get("/healthz", context => context.json(health()));
  const requestShutdown = options.onShutdownRequest;
  if (requestShutdown !== undefined) {
    // The stop Windows has no signal for. The native token is in daemon.json alone, which only this user reads, so a
    // page holding the dashboard's capability cannot stop the daemon; a browser always sends Origin.
    app.post("/native/shutdown", (context) => {
      if (context.req.header("origin") !== undefined) return context.json({ error: "native-origin-forbidden" }, 403);
      if (!sameSecret(context.req.header("authorization"), `Bearer ${nativeToken}`)) {
        return context.json({ error: "native-authorization-invalid" }, 403);
      }
      // After the answer is on its way: closing the server ends this connection too.
      setImmediate(requestShutdown);
      return context.json({ stopping: true }, 202);
    });
  }

  app.post("/native/session", async (context) => {
    // Browsers always send Origin, and Rerail processes always send JSON with the private capability.
    if (context.req.header("origin") !== undefined) return context.json({ error: "native-origin-forbidden" }, 403);
    if (context.req.header("content-type") !== "application/json") return context.json({ error: "native-content-type-required" }, 415);
    if (!sameSecret(context.req.header("authorization"), `Bearer ${nativeToken}`)) {
      return context.json({ error: "native-authorization-invalid" }, 403);
    }
    let body: unknown;
    try {
      body = await context.req.json();
    } catch {
      return context.json({ error: "native-request-invalid" }, 400);
    }
    if (body === null || typeof body !== "object" || Array.isArray(body)
      || !("workspaceRoot" in body) || typeof body.workspaceRoot !== "string") {
      return context.json({ error: "native-request-invalid" }, 400);
    }
    if (resolve(body.workspaceRoot) !== workspaceRoot) return context.json({ error: "native-workspace-mismatch" }, 403);
    try {
      return context.json(await sessions.handle(body));
    } catch (error) {
      return context.json({
        error: "native-request-invalid",
        message: error instanceof Error ? error.message : String(error),
      }, 400);
    }
  });
  app.route("/", createRerailApi(stores, operations, { version: options.version, publicHosts }));
  app.all("/mcp", (context) => mcp(context.req.raw));
  app.all("/api/*", (context) => context.json({ error: "not-found" }, 404));
  app.all("*", (context) => {
    if (context.req.method !== "GET" && context.req.method !== "HEAD") {
      return context.json({ error: "method-not-allowed" }, 405);
    }
    if (uiRoot === undefined) return context.json({ error: "not-found" }, 404);
    return serveStatic(context.req.path, context.req.method === "HEAD", uiRoot, context.req.header("if-none-match"));
  });
  // A client hears that something failed, never why: an error's text can name paths, rule text or SQL. The daemon
  // log (`daemon.log` in the Rerail home, which only this user can read) keeps the whole error. An `HTTPException`
  // is the framework's own answer to a malformed request (a body that is not JSON) and goes out as it is.
  app.onError((error, context) => {
    if (error instanceof HTTPException) return error.getResponse();
    process.stderr.write(`rerail daemon: ${context.req.method} ${context.req.path} failed: ${error.stack ?? error.message}\n`);
    return context.json({ error: "internal", message: INTERNAL_ERROR_MESSAGE }, 500);
  });
  return app;
}

function createDaemonServer(options: RerailDaemonOptions, app: Hono): DaemonServer {
  const listener = getRequestListener(app.fetch);
  const handle: RequestListener = (request, response) => void listener(request, response);
  return options.tls === undefined
    ? createServer(HTTP_TIMEOUTS, handle)
    : createHttpsServer({ ...HTTP_TIMEOUTS, cert: options.tls.cert, key: options.tls.key }, handle);
}

/** What a start has running so far, for a failed one to close. */
interface StartedServices {
  engineService?: EngineService;
  nativeServer?: NativeSessionServer;
  server?: DaemonServer;
  worktreeWalks?: RepositoryWorktreeWalks;
}

/**
 * Every one of them, whatever another does: a socket left bound makes every later start fail (EADDRINUSE); the
 * database last, once nothing uses it. The start's own error is the one thrown; a close that failed too is written to
 * stderr, so a later EADDRINUSE has its cause on record.
 */
async function closeAfterFailedStart({ engineService, nativeServer, server, worktreeWalks }: StartedServices, stores: DatabaseApiStores): Promise<void> {
  const failed = (what: string, reason: unknown): void => {
    process.stderr.write(`rerail daemon: could not close ${what} after a failed start: ${reason instanceof Error ? reason.message : String(reason)}\n`);
  };
  const closing: [string, () => unknown][] = [
    ["the HTTP server", () => (server?.listening === true ? closeHttpServer(server) : undefined)],
    ["the native session server", () => nativeServer?.close()],
    ["the engine service", () => engineService?.close()],
  ];
  const settled = await Promise.allSettled(closing.map(async ([, close]) => await close()));
  settled.forEach((outcome, index) => {
    if (outcome.status === "rejected") failed(closing[index]![0], outcome.reason);
  });
  // After both servers: their sessions have let go of the worktrees.
  worktreeWalks?.close();
  try {
    stores.close();
  } catch (reason) {
    failed("the database", reason);
  }
}

/**
 * The Merge Assist a daemon shares between its API and sessions, over the database `database` opens; none while the
 * experiment is off ({@link mergeAssistExperimentEnabled}), so nothing of it runs in this daemon.
 */
function daemonMergeAssist(database: () => RerailDatabase): { mergeAssist?: MergeAssistRuntime } {
  return mergeAssistExperimentEnabled() ? { mergeAssist: createMergeAssistService({ database }) } : {};
}

export class RerailDaemon {
  readonly host: string;
  readonly port: number;
  readonly url: string;
  readonly startedAt: number;
  readonly #server: DaemonServer;
  readonly #engineService: EngineService;
  readonly #stores: DatabaseApiStores;
  readonly #ui: DaemonState["ui"];
  readonly #native: NativeStatus;
  readonly #options: RerailDaemonOptions;
  readonly #databasePath: string;
  readonly #workspaceRoot: string;
  readonly kernelSocketPath: string;
  readonly #nativeServer: NativeSessionServer;
  /** The walks of the repositories' other worktrees, shared by the sessions of both servers; closed after both. */
  readonly #worktreeWalks: RepositoryWorktreeWalks;
  readonly #nativeToken: string;
  readonly #accessToken: string;
  readonly #tlsFingerprint256: string | undefined;
  #closed = false;
  /** The Merge Assist the API and every session share, while the experiment is on: stopped before the database closes. */
  readonly #mergeAssist: MergeAssistRuntime | undefined;
  #remoteSink: RemoteSinkLoop | undefined;
  #checkoutPruning: CheckoutPruning | undefined;
  #recordCheck: NodeJS.Timeout | undefined;
  /** The check of the state file in flight, which a close waits for before it removes the file. */
  #recordChecked: Promise<void> = Promise.resolve();
  /** What this daemon wrote to the state file, which its close removes only while the file still holds it. */
  #recordText: string | undefined;
  #provenanceMove: { readonly stop: AbortController; readonly done: Promise<void> } | undefined;
  /** What `RerailDaemonOptions.statusLines` started: stopped first on close, while the stores it reads are open. */
  #statusLines: { stop(): Promise<void> } | undefined;

  private constructor({
    server, engineService, kernelSocketPath, stores, options, databasePath, native, ui, workspaceRoot, nativeServer, nativeToken, accessToken, mergeAssist,
    worktreeWalks, tlsFingerprint256,
  }: {
    server: DaemonServer;
    engineService: EngineService;
    kernelSocketPath: string;
    stores: DatabaseApiStores;
    options: RerailDaemonOptions;
    databasePath: string;
    native: NativeStatus;
    ui: DaemonState["ui"];
    workspaceRoot: string;
    nativeServer: NativeSessionServer;
    worktreeWalks: RepositoryWorktreeWalks;
    nativeToken: string;
    accessToken: string;
    mergeAssist?: MergeAssistRuntime;
    tlsFingerprint256: string | undefined;
  }) {
    const address = server.address() as AddressInfo;
    this.host = address.address;
    this.port = address.port;
    this.url = `${tlsFingerprint256 === undefined ? "http" : "https"}://${this.host}:${this.port}/`;
    this.startedAt = Date.now();
    this.#server = server;
    this.#engineService = engineService;
    this.kernelSocketPath = kernelSocketPath;
    this.#stores = stores;
    this.#options = options;
    this.#databasePath = databasePath;
    this.#native = native;
    this.#ui = ui;
    this.#workspaceRoot = workspaceRoot;
    this.#nativeServer = nativeServer;
    this.#worktreeWalks = worktreeWalks;
    this.#nativeToken = nativeToken;
    this.#accessToken = accessToken;
    this.#mergeAssist = mergeAssist;
    this.#tlsFingerprint256 = tlsFingerprint256;
  }

  static async start(options: RerailDaemonOptions): Promise<RerailDaemon> {
    // Before anything is opened or bound: a file that holds no certificate is the start's whole failure.
    const tlsFingerprint256 = options.tls === undefined ? undefined : certificateFingerprint(options.tls.cert);
    const { bindHost, publicHosts, remote } = daemonHosts(options);
    const accessToken = await daemonAccessToken(options, remote);
    const { uiRoot, ui } = daemonUi(options);
    const native = readNativeStatus(options.nativeHealth ?? loadNativeHealth);
    const databasePath = resolve(options.databasePath ?? defaultRerailDatabasePath());
    // Anyone on the machine can make `rerail-<uid>` in a shared temp directory first, so a refused one falls back to a
    // `run` folder beside the database, checked the same way. Clients find the socket through the state file, never by
    // computing it. A socket path the caller named is used or refused, never replaced.
    const defaultSocketPath = defaultEngineSocketPath(databasePath, options.enginePlatform);
    const kernelSocketPath = options.kernelSocketPath
      ?? await chooseEngineSocketPath([defaultSocketPath, join(dirname(databasePath), "run", basename(defaultSocketPath))]);
    const workspaceRoot = resolve(options.workspaceRoot ?? process.cwd());
    // The paths of a session whose host sends none.
    const { storageRoot, modelPath } = resolveSessionPaths(workspaceRoot);
    let daemon: RerailDaemon | undefined;
    const started: StartedServices = {};
    // One Merge Assist for the API and every session this daemon runs, so a scan runs once per checkout and gh's
    // readiness is read once; its database opens with the stores', on first use.
    const sharedMergeAssist = daemonMergeAssist(() => stores.database());
    // Open the database before binding anything: a refused open (a file this build cannot read, say) must leave no
    // engine socket behind, or every later start fails with EADDRINUSE.
    const stores = new DatabaseApiStores(databasePath, {
      health: () => daemon!.health(),
      origin: () => new URL(daemon!.url).origin,
      ...(options.codingAgents === undefined ? {} : { codingAgents: options.codingAgents }),
      // The server listens only once the engine service runs, so a scan request always finds it. The engine service
      // reports a failure itself, and its close waits for the open.
      resumeSession: sessionId => void started.engineService?.resumeSession(sessionId),
      ...sharedMergeAssist,
    });
    try {
      // One walk of each repository's other worktrees for the sessions of both servers: hook sessions and native OMP and
      // Pi sessions in worktrees of one repository read them once, through the engine's one watch.
      const worktreeWalks = engineWorktreeWalks();
      started.worktreeWalks = worktreeWalks;
      const engineService = await startEngineService({
        socketPath: kernelSocketPath,
        storageRoot,
        modelPath,
        databasePath,
        workspaceRoot,
        accessToken,
        worktreeWalks,
        ...sharedMergeAssist,
        // On Windows any local user can create the default pipe first; the engine then listens on a random name.
        ...(options.kernelSocketPath === undefined
          ? { pipeFallback: () => fallbackPipe(kernelSocketPath) }
          : {}),
      });
      started.engineService = engineService;
      const nativeToken = randomBytes(32).toString("hex");
      const sessions = new NativeSessionServer({ workspaceRoot, modelPath, databasePath, worktreeWalks, ...sharedMergeAssist });
      started.nativeServer = sessions;
      const app = daemonApp({ stores, options, publicHosts, accessToken, nativeToken, sessions, workspaceRoot, uiRoot, health: () => daemon!.health() });
      const server = createDaemonServer(options, app);
      started.server = server;
      const remembered = options.portFile === undefined ? undefined : await readRememberedPort(options.portFile);
      await listenPreferred(server, options.port, remembered, bindHost);
      daemon = new RerailDaemon({
        server, engineService, kernelSocketPath: engineService.socketPath, stores, options, databasePath, native, ui, workspaceRoot,
        nativeServer: sessions, nativeToken, accessToken, tlsFingerprint256, worktreeWalks, ...sharedMergeAssist,
      });
      await daemon.#announce(remembered);
      daemon.#statusLines = options.statusLines?.(stores);
      daemon.#startCheckoutPruning();
      daemon.#startProvenanceMove();
      return daemon;
    } catch (error) {
      await closeAfterFailedStart(started, stores);
      throw error;
    }
  }

  /** What a started daemon leaves behind for others: its state file, its remote sink and the port links point at. */
  async #announce(remembered: number | undefined): Promise<void> {
    const options = this.#options;
    if (options.stateFile !== undefined) await this.#record(options.stateFile);
    this.#startRemoteSink();
    // A start that asked for any free port (`0`) takes a throwaway one; it does not replace the port links point at.
    if (options.portFile !== undefined && options.port !== 0 && this.port !== remembered) await rememberPort(options.portFile, this.port);
  }

  /** Once nothing can fail the start: checkouts whose folder is gone stop costing every read of their repository. */
  #startCheckoutPruning(): void {
    const pruning = this.#options.checkoutPruning;
    if (pruning !== false) this.#checkoutPruning = startCheckoutPruning(this.#databasePath, pruning ?? {});
  }

  /**
   * Internal testing: the remote sink sends nothing until a remote is added (`rerail remote add`), and nothing at all
   * while it is off in the daemon.
   */
  #startRemoteSink(): void {
    const sink = this.#options.remoteSink;
    if (sink !== false && remoteSinkEnabled()) this.#remoteSink = startRemoteSink({ home: dirname(this.#databasePath), ...(sink ?? {}) });
  }

  /**
   * Writes `stateFile`, the record hooks, starts and stops find this daemon by, and with `recordCheck` keeps it: every
   * `everyMs` it reads the file and writes it again when it is gone or holds anything else. The file can go
   * while the daemon runs: a lookup that takes a live daemon for a dead one removes it, as does the close of a daemon
   * from an earlier build that shared the home. Nothing finds a daemon no file names, and every start beside it refuses
   * to run; one that names this daemon with a capability it does not accept gets every hook refused. Only this daemon
   * writes the file, so anything but what it wrote is wrong. The check reads one small file and writes only when it
   * differs. It ends for good once the daemon no longer holds the home (`holdsHome`): a home moved or removed while it
   * runs, and maybe made again at its path, is no longer its own, and a record it wrote there would name it to every
   * start and hook in that home while it records into a database that home does not hold.
   */
  async #record(stateFile: string): Promise<void> {
    const text = `${JSON.stringify(this.state(), null, 2)}\n`;
    await replaceFile(stateFile, text, { mode: 0o600 });
    this.#recordText = text;
    const check = this.#options.recordCheck;
    if (check === undefined) return;
    this.#recordCheck = setInterval(() => {
      this.#recordChecked = this.#recordChecked.then(async () => await this.#restoreRecord(stateFile, text, check));
    }, check.everyMs);
    this.#recordCheck.unref();
  }

  /**
   * Whether this daemon no longer holds its home (`check.holdsHome`); if so, ends the record check for good and says why
   * in daemon.log. A check that throws cannot tell, and the home counts as held: the record is kept, as before the check.
   */
  #lostHome(stateFile: string, check: NonNullable<RerailDaemonOptions["recordCheck"]>): boolean {
    try {
      if (check.holdsHome()) return false;
    } catch {
      return false;
    }
    clearInterval(this.#recordCheck);
    this.#recordCheck = undefined;
    process.stderr.write(`rerail daemon: this daemon (pid ${process.pid}) no longer holds the Rerail home at ${check.home}: it was moved `
      + `or removed. The daemon stops writing ${stateFile}; end it, then run rerail daemon start.\n`);
    return true;
  }

  /** Writes `text` to `stateFile` again unless the file holds it, while this daemon holds its home (`check.holdsHome`). */
  async #restoreRecord(stateFile: string, text: string, check: NonNullable<RerailDaemonOptions["recordCheck"]>): Promise<void> {
    if (this.#closed || this.#recordCheck === undefined || this.#lostHome(stateFile, check)) return;
    const found = await readFile(stateFile, "utf8").catch(() => undefined);
    if (found === text) return;
    try {
      await replaceFile(stateFile, text, { mode: 0o600 });
      process.stderr.write(`rerail daemon: ${stateFile} ${foundInstead(found)}; it holds this daemon's (pid ${process.pid}) again\n`);
    } catch (error) {
      process.stderr.write(`rerail daemon: could not write ${stateFile} again: ${errorText(error)}\n`);
    }
  }

  /**
   * Moves the rule provenance that audits recorded before the upgrade hold inline, a batch per short transaction while
   * the daemon serves (`rule-provenance-move.ts`), so no open holds the write lock for the whole history. The move runs
   * on a worker thread, so this thread keeps answering through its batches. It finds nothing to move on a file it has
   * already moved, and starts no thread then. A failure goes to the daemon log with its stack, and the next start tries
   * again; a stop is no failure and logs nothing. The audits read the same either way.
   */
  #startProvenanceMove(): void {
    const stop = new AbortController();
    const done = moveRuleProvenancesInWorker(this.#databasePath, stop.signal).then(() => undefined, (error: unknown) => {
      process.stderr.write(`rerail daemon: moving audit rule provenance failed: ${errorStack(error)}\n`);
    });
    this.#provenanceMove = { stop, done };
  }

  state(): DaemonState {
    return {
      pid: process.pid,
      host: this.host,
      port: this.port,
      url: this.url,
      version: this.#options.version,
      databasePath: this.#databasePath,
      kernelSocketPath: this.kernelSocketPath,
      ui: this.#ui,
      startedAt: this.startedAt,
      packageRoot: PACKAGE_ROOT,
      workspaceRoot: this.#workspaceRoot,
      nativeToken: this.#nativeToken,
      accessToken: this.#accessToken,
      readsSpelling: true,
      ...(this.#tlsFingerprint256 === undefined ? {} : { tlsFingerprint256: this.#tlsFingerprint256 }),
      ...(this.#options.remoteRefused === undefined ? {} : { remoteRefused: this.#options.remoteRefused }),
    };
  }

  health(): DaemonHealth {
    const model = modelStatus();
    return {
      status: "ok",
      version: this.#options.version,
      pid: process.pid,
      uptimeMs: Date.now() - this.startedAt,
      database: {
        path: this.#databasePath,
        exists: existsSync(this.#databasePath),
      },
      native: this.#native,
      model,
      healthy: this.#native.ok && model.ok,
      ui: this.#ui,
      workspaceRoot: this.#workspaceRoot,
      installedAt: this.#stores.installedAt(),
      experiments: enabledExperiments(),
    };
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    clearInterval(this.#recordCheck);
    // A write in flight lands before the state file is removed below, never after.
    await this.#recordChecked;
    try {
      await this.#statusLines?.stop();
      this.#provenanceMove?.stop.abort();
      await this.#provenanceMove?.done;
      // Pruning stops first: a remote sink that fails to stop must not leave a pass free to commit after close begins.
      await this.#checkoutPruning?.stop();
      await this.#remoteSink?.stop();
      await closeHttpServer(this.#server);
    } finally {
      try {
        await this.#nativeServer.close();
        await this.#engineService.close();
      } finally {
        this.#worktreeWalks.close();
        await this.#mergeAssist?.stop();
        this.#stores.close();
        if (this.#options.stateFile !== undefined) await this.#removeOwnRecord(this.#options.stateFile);
      }
    }
  }

  /**
   * Removes `stateFile` while it holds what this daemon wrote, and leaves anything else: a home moved away and made again
   * at its path while this daemon ran can have a daemon of its own, whose record names it.
   */
  async #removeOwnRecord(stateFile: string): Promise<void> {
    if (this.#recordText === undefined) return;
    const found = await readFile(stateFile, "utf8").catch(() => undefined);
    if (found === this.#recordText) await rm(stateFile, { force: true });
  }
}

/**
 * Page files with an SPA fallback: a path that escapes the UI root names no file there, so one with an extension is
 * 404 and one without gets the shell like any client route. Every URL the page uses is unversioned (`/app.js`,
 * `/render/*.js`), so nothing may be cached past a revalidation: a browser that kept `app.js` for a year would run the
 * previous release's page, or in `--dev` miss every edit. The validator is a stat, so the common answer is a bodyless
 * 304.
 */
async function serveStatic(pathname: string, headOnly: boolean, uiRoot: string, ifNoneMatch: string | undefined): Promise<Response> {
  const notFound = () => Response.json({ error: "not-found" }, { status: 404 });
  const relative = normalize(decodeURIComponent(pathname)).replace(/^([/\\])+/, "");
  const candidate = resolve(uiRoot, relative);
  const inside = candidate === uiRoot || candidate.startsWith(uiRoot + sep);
  let target = inside ? candidate : undefined;
  if (target !== undefined) {
    const info = await stat(target).catch(() => undefined);
    if (info === undefined || !info.isFile()) target = undefined;
  }
  // Client-side routes have no file; a request that names no extension gets the SPA shell.
  if (target === undefined) {
    if (extname(relative) !== "") return notFound();
    target = join(uiRoot, "index.html");
  }
  const info = await stat(target).catch(() => undefined);
  if (info === undefined) return notFound();
  const etag = `W/"${info.size.toString(16)}-${Math.trunc(info.mtimeMs).toString(16)}"`;
  const contentType = CONTENT_TYPES[extname(target)] ?? "application/octet-stream";
  // The page holds a write-capable API origin: no framing (clickjacking) and no MIME sniffing.
  const headers: Record<string, string> = {
    "content-type": contentType,
    "cache-control": "no-cache",
    "x-content-type-options": "nosniff",
    etag,
    ...(contentType.startsWith("text/html")
      ? { "content-security-policy": "frame-ancestors 'none'", "x-frame-options": "DENY" }
      : {}),
  };
  if (ifNoneMatch?.split(",").some(candidate => candidate.trim() === etag)) return new Response(null, { status: 304, headers });
  const full = { ...headers, "content-length": String(info.size) };
  if (headOnly) return new Response(null, { status: 200, headers: full });
  return new Response(Readable.toWeb(createReadStream(target)) as unknown as ReadableStream, { status: 200, headers: full });
}

/** Parsed `--flag` / `--option value` arguments; every name is validated against the accepted sets. */
export interface ParsedArguments {
  flags: ReadonlySet<string>;
  options: ReadonlyMap<string, string>;
}

export function parseArguments(
  argv: readonly string[],
  accepted: { flags: readonly string[]; options: readonly string[] },
  unknownMessage: (argument: string) => string,
): ParsedArguments {
  const flags = new Set<string>();
  const options = new Map<string, string>();
  let index = 0;
  while (index < argv.length) {
    const argument = argv[index] ?? "";
    if (accepted.flags.includes(argument)) {
      flags.add(argument);
      index += 1;
      continue;
    }
    if (!accepted.options.includes(argument)) throw new Error(unknownMessage(argument));
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) throw new Error(`${argument} requires a value`);
    options.set(argument, value);
    index += 2;
  }
  return { flags, options };
}

interface DaemonArguments {
  host: string;
  publicHosts: readonly string[];
  port?: number;
  dev: boolean;
  stateFile?: string;
  /** The checkout native sessions address (`--workspace-root`); the folder it runs in when absent. */
  workspaceRoot?: string;
  tls?: DaemonTlsFiles;
}

const LOOPBACK_BIND_HOSTS: readonly string[] = ["127.0.0.1", "localhost", "::1"];

/**
 * Binding off loopback hands the write API to everything on the network, so it is an explicit
 * opt-in (`--allow-remote`), not a side effect of `--host`. Shared by `rerail start` and the daemon
 * itself, so the check holds for whichever launcher ran first.
 */
export function assertHostBindable(host: string, allowRemote: boolean): void {
  if (allowRemote || LOOPBACK_BIND_HOSTS.includes(host.toLowerCase())) return;
  throw new Error(
    `--host ${host} is not a loopback address; every host on the network could then read and write this daemon. `
    + "Pass --allow-remote to bind it anyway, and --public-host <name> for each hostname it should answer to.",
  );
}

/** `--public-host a,b`: each entry is a bare hostname, checked case-insensitively against `Host`. */
export function parsePublicHosts(text: string | undefined): readonly string[] {
  return text === undefined ? [] : text.split(",").map(host => host.trim()).filter(host => host !== "");
}

/** The flags and options `parseDaemonTransport` reads; every launcher of the daemon accepts them. */
export const DAEMON_TRANSPORT_FLAGS: readonly string[] = ["--insecure-http"];
export const DAEMON_TRANSPORT_OPTIONS: readonly string[] = ["--tls-cert", "--tls-key"];

/**
 * Whether a daemon on `host` answering `publicHosts` serves https (`--tls-cert` and `--tls-key`, resolved to absolute
 * paths) or plain http. One reachable from other machines (bound off loopback, or answering a `--public-host`) must
 * not serve plain http unless `--insecure-http` says so: every request carries the access capability, which would
 * cross the network in clear. Shared by `rerail daemon start` and the daemon itself, like `assertHostBindable`.
 */
export function parseDaemonTransport(
  arguments_: ParsedArguments,
  host: string,
  publicHosts: readonly string[],
  platform: NodeJS.Platform = process.platform,
): Pick<DaemonLaunchOptions, "tls" | "insecureHttp"> {
  const certFile = arguments_.options.get("--tls-cert");
  const keyFile = arguments_.options.get("--tls-key");
  if ((certFile === undefined) !== (keyFile === undefined)) {
    throw new Error("--tls-cert and --tls-key go together: pass the certificate and its private key, both PEM files.");
  }
  const insecureHttp = arguments_.flags.has("--insecure-http");
  if (certFile !== undefined && keyFile !== undefined) {
    if (insecureHttp) throw new Error("--insecure-http contradicts --tls-cert and --tls-key; pass one or the other.");
    return { tls: { certFile: resolve(certFile), keyFile: resolve(keyFile) } };
  }
  if (insecureHttp) return { insecureHttp: true };
  const offLoopback = !LOOPBACK_BIND_HOSTS.includes(host.toLowerCase());
  if (!offLoopback && publicHosts.length === 0) return {};
  throw new Error(
    `${offLoopback ? `--host ${host}` : `--public-host ${publicHosts.join(",")}`} makes this daemon reachable from other machines, `
    + "and over plain http its access capability, sent with every request, would cross the network in clear. "
    + "Pass --tls-cert <cert.pem> --tls-key <key.pem> to serve https; a self-signed certificate works "
    + "(openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 365 -subj /CN=rerail "
    + `-keyout key.pem -out cert.pem, then ${privateKeyHint(platform)}). Pass --insecure-http instead only when the network in `
    + "between is already trusted, such as a container port published on the host's loopback.",
  );
}

/**
 * How to keep a new private key its owner's alone: mode bits elsewhere, and on Windows, which has none and no `chmod`,
 * an ACL that grants only the person's account, named as it is so the command runs in cmd and PowerShell alike.
 */
function privateKeyHint(platform: NodeJS.Platform): string {
  if (platform !== "win32") return "chmod 600 key.pem";
  let account = "<your account>";
  try {
    account = userInfo().username;
  } catch {
    // No user database entry: the placeholder stays.
  }
  return `icacls key.pem /inheritance:r /grant:r "${account}:R"`;
}

/**
 * A TLS private key file this account reads, which parses as a key, that the daemon may not serve with because other
 * accounts can read it too. Its message names the key and the command that makes it its owner's alone.
 */
export class TlsKeyExposedError extends Error {
  override readonly name = "TlsKeyExposedError";
}

/**
 * A TLS private key file that is empty or does not parse as a private key (`unparsedLine`), whether only its owner
 * reads it or others can too (`loadDaemonTls`). The daemon serves this computer alone on it, as on a key others can
 * read, so hooks keep recording; its message names the file and what it must hold, and then, when others can read it,
 * says so as `TlsKeyExposedError`'s does. Node's https server reads an empty key as no key: it would listen, say https,
 * and fail every handshake.
 */
export class TlsKeyUnparsedError extends Error {
  override readonly name = "TlsKeyUnparsedError";
}

/**
 * The PEM files `tls` names, read once at start. The private key must be its owner's alone (`openToOthers`): a key
 * other users can read lets any of them impersonate the daemon, so a key with any group or other permission is refused,
 * as ssh refuses one, and on Windows, which has no mode bits, a key whose ACL lets any account beyond the person,
 * SYSTEM and Administrators read it, as OpenSSH for Windows refuses one. A key outside the profile folder, under
 * `C:\certs` say, inherits an ACL that lets every local user read it. A key file this account reads, which parses as a
 * private key, and others can read too is a `TlsKeyExposedError`, on which the daemon serves this computer alone
 * (`serveRerailDaemon`), when the certificate would serve with it (`certificateServes`). A key file this account reads
 * that is empty or does not parse, such as one that holds the certificate, is a `TlsKeyUnparsedError`, on which the
 * daemon serves this computer alone too: when only its owner reads it, and when others can read it too beside a
 * certificate the start would take, its line then saying both. A key others can read that is no file this account can
 * read (a folder, a FIFO, a file this account may not read, one another program holds with a lease or an oplock, one
 * larger than any key) or that the certificate would not serve with, a key whose readers cannot be read (a missing or
 * misspelled one), and a key only its owner reads that cannot be read (a folder, a file this account may not read), is
 * a mistake in the command and stops the start within seconds. So does a certificate that is empty or does not parse
 * (`refuseUnparsed`), in a line that names the file. `access` is for tests.
 */
export async function loadDaemonTls(tls: DaemonTlsFiles, access: AccessReader = {}): Promise<DaemonTlsMaterial> {
  let others: string | undefined;
  try {
    others = await openToOthers(tls.keyFile, access);
  } catch (error) {
    // Unread is refused, as a key others can read is: the daemon would otherwise serve with it unchecked.
    throw new Error(`Could not read who may read ${tls.keyFile}: ${errorText(error)}`, { cause: error });
  }
  if (others !== undefined) {
    // A function, so a `$` in the key's path is never read as a replacement pattern.
    const hint = privateKeyHint(access.platform ?? process.platform).replace("key.pem", () => `"${tls.keyFile}"`);
    throw await refusedKey(tls, `${tls.keyFile} ${others}; a TLS private key must be readable by its owner only: ${hint}`);
  }
  const [cert, key] = await Promise.all([readFile(tls.certFile, "utf8"), readFile(tls.keyFile, "utf8")]);
  // The certificate first: one the start would refuse beside a good key is refused now, not once the key is fixed.
  refuseUnparsed(CERTIFICATE_FILE, tls.certFile, cert);
  refuseUnparsed(PRIVATE_KEY_FILE, tls.keyFile, key);
  return { cert, key };
}

/**
 * What a TLS file is called, what it must hold, the parse that stands for the one its text meets at start, and what a
 * file that fails it throws.
 */
interface TlsFileKind {
  readonly name: string;
  readonly holds: string;
  readonly parse: (text: string) => unknown;
  readonly refused: (message: string) => Error;
}

// The key is parsed as the https server parses it, so every key a server took before is taken still; the certificate
// as `RerailDaemon.start` parses it for its fingerprint, which refused any other before.
const PRIVATE_KEY_FILE: TlsFileKind = {
  name: "TLS private key", holds: "a PEM private key", parse: key => createSecureContext({ key }), refused: message => new TlsKeyUnparsedError(message),
};
const CERTIFICATE_FILE: TlsFileKind = {
  name: "TLS certificate", holds: "a PEM X.509 certificate", parse: pem => new X509Certificate(pem), refused: message => new Error(message),
};

/**
 * One line that names `file` and says what is wrong when `text`, read from it, is empty or does not parse as `kind`;
 * undefined when it parses. The line says the file could not be read as what it must hold, which stays true of a key
 * that needs a passphrase.
 */
function unparsedLine(kind: TlsFileKind, file: string, text: string): string | undefined {
  if (text === "") return `The ${kind.name} ${file} is empty: it must hold ${kind.holds}`;
  try {
    kind.parse(text);
    return undefined;
  } catch (error) {
    return `The ${kind.name} ${file} could not be read as ${kind.holds}: ${errorText(error)}`;
  }
}

/** Throws `kind.refused` with `unparsedLine`'s line, when it has one. */
function refuseUnparsed(kind: TlsFileKind, file: string, text: string): void {
  const line = unparsedLine(kind, file, text);
  if (line !== undefined) throw kind.refused(line);
}

/**
 * What a start whose key others can read throws, when the key is a file this account reads (`readKeyFile`) beside a
 * certificate that would serve with it (`certificateServes`): `TlsKeyExposedError` for a private key and
 * `TlsKeyUnparsedError` for a file that holds none (`servedAlone`), on each of which the daemon serves this computer
 * alone; else `refusal`, main's message for this command line. On Windows a Node child opens the two files first
 * (`openedInChild`). When its open was still waiting at its bound, the start refuses; when it never got to open them,
 * the refusal says why on a line of its own, since the person would otherwise expect the daemon on loopback.
 */
async function refusedKey(tls: DaemonTlsFiles, refusal: string): Promise<Error> {
  const child = process.platform === "win32" ? await openedInChild([tls.keyFile, tls.certFile]) : OPENED;
  if (child.kind !== "opened") return new Error(child.kind === "held" ? refusal : `${refusal}\n${child.why}`);
  const key = await readKeyFile(tls.keyFile);
  return key !== undefined && (await certificateServes(tls.certFile, key)) ? servedAlone(key, refusal) : new Error(refusal);
}

/** A key file others can read, as this account read it: a private key, or the line saying it is empty or holds none. */
type KeyFile = { readonly kind: "key"; readonly key: Buffer } | { readonly kind: "unparsed"; readonly line: string };

/**
 * The error on which the daemon serves this computer alone for `key`, a file others can read as `refusal` says. For a
 * file that holds no key, its one line first names the file and says so, then gives `refusal`.
 */
function servedAlone(key: KeyFile, refusal: string): Error {
  return key.kind === "key" ? new TlsKeyExposedError(refusal) : new TlsKeyUnparsedError(`${key.line}; ${refusal}`);
}

/**
 * What `path` holds, read once (`readWithoutWaiting`), when it is a regular file this account reads: the key, when it
 * parses as a private key; else the line `unparsedLine` gives it, empty or no key (text, the certificate, a key that
 * needs a passphrase). Undefined for a folder, a FIFO, a file this account may not read or another program holds with a
 * lease, one larger than any key, and one the https server would read though `createPrivateKey` does not.
 */
async function readKeyFile(path: string): Promise<KeyFile | undefined> {
  const text = await readWithoutWaiting(path).catch(() => undefined);
  if (text === undefined) return undefined;
  try {
    createPrivateKey(text);
    return { kind: "key", key: text };
  } catch {
    const line = unparsedLine(PRIVATE_KEY_FILE, path, text.toString("utf8"));
    return line === undefined ? undefined : { kind: "unparsed", line };
  }
}

/**
 * Whether the certificate at `certFile` would serve https with `key`, checked as an https start checks the two (the read
 * in `loadDaemonTls`, then `RerailDaemon.start`'s parse and the server's pairing), for a start whose key others can read,
 * which serves with neither. A key file that holds no key pairs with nothing: its certificate need only be one the
 * start would take, as beside such a key only its owner reads. A certificate that cannot be read, holds no certificate
 * or is not `key`'s throws what that start throws for it beside a private key, naming a folder, whose read error names
 * no file. A file `readWithoutWaiting` does not read (a FIFO, one another program holds with a lease, one larger than
 * any certificate) serves nothing.
 */
async function certificateServes(certFile: string, key: KeyFile): Promise<boolean> {
  const cert = (await readWithoutWaiting(certFile))?.toString("utf8");
  if (cert === undefined) return false;
  refuseUnparsed(CERTIFICATE_FILE, certFile, cert);
  if (key.kind === "key") createSecureContext({ cert, key: key.key });
  return true;
}

/**
 * The most a refused start reads of its key or certificate. A PEM private key, or a certificate with its chain, takes a
 * few kilobytes, so a larger file is no key or certificate a person serves with: it gets the key's refusal, unread past
 * this bound, so a file of gigabytes is refused as fast as a missing one.
 */
const REFUSED_TLS_FILE_MAX_BYTES = 1024 * 1024;

/**
 * Opens for reading without waiting (`O_NONBLOCK`). Where another program holds a lease on the file, as Samba's kernel
 * oplocks and an NFS server's delegations do on Linux, an open that waited would wait for the lease to break, 45 s by
 * default, past `rerail daemon start`'s 15 s wait; this one fails at once (`EAGAIN`). A FIFO opens without waiting for
 * a writer. Windows has no `O_NONBLOCK`: undefined there, it adds nothing, and `openedInChild` waits in its place.
 */
const OPEN_WITHOUT_WAITING = constants.O_RDONLY | (constants.O_NONBLOCK ?? 0);

/**
 * What a refused start's Node child did with the key and the certificate on Windows (`openedInChild`): opened them, or
 * failed to, in time; began but was still in an open at its bound; or never got to open them, with the line a refusal
 * then adds.
 */
type ChildOpen = { readonly kind: "opened" } | { readonly kind: "held" } | { readonly kind: "unstarted"; readonly why: string };

const OPENED: ChildOpen = { kind: "opened" };
const HELD: ChildOpen = { kind: "held" };

/** A child that never got to open the files: why the daemon does not serve this computer meanwhile, as a sentence. */
function unstarted(why: string): ChildOpen {
  return { kind: "unstarted", why: `This daemon would serve this computer alone meanwhile, but the Node process that opens the key and the certificate for it first ${why}.` };
}

/**
 * How long a refused start waits on Windows for its Node child to begin, from the spawn (`openedInChild`). A Node start
 * takes about 40 ms on a quiet machine, and its 95th percentile reached 3.5 s on a laptop busy with coding agents. A
 * child that has not begun within 10 s leaves too little of `rerail daemon start`'s 15 s wait for a daemon on loopback.
 */
const CHILD_START_WAIT_MS = 10_000;

/** How long the child's opens of the key and the certificate may wait, from the moment it begins them (`openedInChild`). */
const CHILD_OPEN_WAIT_MS = 2_000;

/** How long a refused start waits for its Node child to exit once it has ended it: milliseconds, even in an open. */
const CHILD_END_WAIT_MS = 1_000;

/**
 * The child's own pipe, past its stdin, stdout and stderr, which go nowhere: where it says it began and that it is
 * done (`CHILD_OPEN`). A preload in the person's NODE_OPTIONS runs in the child before its script and may print on
 * stdout or stderr; what it prints never reaches this pipe, so it never speaks for the child.
 */
const CHILD_PIPE_FD = 3;

/**
 * The child's work: say it began, open each file named after its script to read it and close it, then say it is done;
 * the daemon reports what an open throws. The writes are synchronous, so the first leaves before an open that waits.
 */
const CHILD_OPEN = `const fs = require('node:fs'); fs.writeSync(${CHILD_PIPE_FD}, 'b'); for (const file of process.argv.slice(1)) { try { fs.closeSync(fs.openSync(file, 'r')); } catch {} } fs.writeSync(${CHILD_PIPE_FD}, 'd');`;

/**
 * Windows: what one Node child did when asked to open `files` to read them. Windows has no open that skips another
 * program's oplock: one that caches writes, as Windows' SMB server holds for a client caching the file, holds every
 * open that reads it until its holder acknowledges the break, and a holder that never does holds it until it closes the
 * file. The child's open waits in the daemon's place: an open waiting in the daemon itself would keep its process from
 * exiting until the holder lets go, long after `rerail daemon start` stopped waiting. The opens get
 * `CHILD_OPEN_WAIT_MS` from the moment the child says on its own pipe (`CHILD_PIPE_FD`) that it began, so a busy
 * machine's slow Node start does not eat into them, and the start gets `CHILD_START_WAIT_MS`; a child past either bound
 * is ended. Only what the child says there counts, never what a preload prints. A child that cannot be spawned,
 * or ends before it began, never leads to an open in the daemon. Once the child's opens are through, any break is
 * acknowledged, and the daemon's own open does not wait.
 */
async function openedInChild(files: readonly string[]): Promise<ChildOpen> {
  let child: ReturnType<typeof spawn>;
  try {
    // Its stdin, stdout and stderr go nowhere; the pipe at index `CHILD_PIPE_FD` is what it says.
    child = spawn(process.execPath, ["-e", CHILD_OPEN, ...files], { stdio: ["ignore", "ignore", "ignore", "pipe"] });
  } catch (error) {
    return unstarted(`could not start: ${errorText(error)}`);
  }
  const exited = new Promise(resolve => {
    child.once("exit", resolve);
    child.once("error", resolve);
  });
  try {
    return await childOpen(child);
  } finally {
    // One that said it is done ends by itself; one still starting, or in an open, is ended here, and the start goes on
    // once it has exited, so nothing it started outlives it, or after `CHILD_END_WAIT_MS` at most.
    child.kill();
    await Promise.race([exited, new Promise(resolve => setTimeout(resolve, CHILD_END_WAIT_MS).unref())]);
  }
}

/** What `child` did (`openedInChild`): said it was done, ran out of time in an open after it began, or never began. */
async function childOpen(child: ReturnType<typeof spawn>): Promise<ChildOpen> {
  let timer: NodeJS.Timeout | undefined;
  try {
    return await new Promise<ChildOpen>(resolve => {
      timer = setTimeout(resolve, CHILD_START_WAIT_MS, unstarted(`did not start within ${CHILD_START_WAIT_MS / 1_000} s`));
      child.on("error", error => resolve(unstarted(`could not start: ${error.message}`)));
      // Only after all its output, so a child that said it is done counts as done however soon it ends.
      child.on("close", code => resolve(unstarted(`ended (exit code ${code ?? "none"}) before it opened them`)));
      // Unset when the spawn failed for want of file handles (EMFILE), which `error` answers.
      const pipe = child.stdio?.[CHILD_PIPE_FD];
      if (!(pipe instanceof Readable)) return;
      // A failed read only ends its output, which `close` or a bound then answers; unheard, it would throw here.
      pipe.on("error", () => undefined);
      // Only the child's own script writes to its pipe: "b" as it begins its opens, then "d" once they are through.
      let said = "";
      pipe.on("data", (chunk: Buffer) => {
        said += chunk.toString();
        if (said === "bd") resolve(OPENED);
        else if (said === "b") {
          clearTimeout(timer);
          timer = setTimeout(resolve, CHILD_OPEN_WAIT_MS, HELD);
        }
      });
    });
  } finally {
    clearTimeout(timer);
  }
}

/**
 * What the file at `path` holds: opened once without waiting (`OPEN_WITHOUT_WAITING`), then checked and read through
 * that handle, so the file checked is the file read. Undefined for a file another program holds under a lease, for a
 * path that is no regular file or folder (a FIFO, a device), which is never read, and for a file larger than
 * `REFUSED_TLS_FILE_MAX_BYTES`, of which no more than that is read. Throws what opening the path throws, and what
 * reading it throws (a folder's `EISDIR`) with the path added, as an error of the open names it.
 */
async function readWithoutWaiting(path: string): Promise<Buffer | undefined> {
  const handle = await open(path, OPEN_WITHOUT_WAITING).catch((error: unknown) => {
    if ((error as NodeJS.ErrnoException).code === "EAGAIN") return undefined;
    throw error;
  });
  if (handle === undefined) return undefined;
  try {
    const found = await handle.stat();
    return found.isFile() || found.isDirectory() ? await readAtMost(handle, REFUSED_TLS_FILE_MAX_BYTES) : undefined;
  } catch (error) {
    throw new Error(`${errorText(error)} '${path}'`, { cause: error });
  } finally {
    await handle.close();
  }
}

/** What `handle` holds from its start, reading at most `limit` bytes and one: undefined past `limit`. */
async function readAtMost(handle: FileHandle, limit: number): Promise<Buffer | undefined> {
  const buffer = Buffer.alloc(limit + 1);
  let filled = 0;
  let bytesRead = -1;
  while (bytesRead !== 0 && filled < buffer.length) {
    ({ bytesRead } = await handle.read(buffer, filled, buffer.length - filled, filled));
    filled += bytesRead;
  }
  return filled > limit ? undefined : buffer.subarray(0, filled);
}

/** The SHA-256 fingerprint of the first certificate in `pem`, the one a TLS server presents as its own. */
function certificateFingerprint(pem: string): string {
  try {
    return new X509Certificate(pem).fingerprint256;
  } catch (error) {
    throw new Error(`The TLS certificate is not a PEM X.509 certificate: ${errorText(error)}`);
  }
}

function parseDaemonArguments(argv: readonly string[]): DaemonArguments {
  const { flags, options } = parseArguments(
    argv,
    { flags: ["--dev", "--allow-remote", ...DAEMON_TRANSPORT_FLAGS], options: ["--host", "--port", "--state-file", "--workspace-root", "--public-host", ...DAEMON_TRANSPORT_OPTIONS] },
    (argument) => `Unknown daemon argument: ${argument}`,
  );
  const host = options.get("--host") ?? DEFAULT_DAEMON_HOST;
  assertHostBindable(host, flags.has("--allow-remote"));
  const publicHosts = parsePublicHosts(options.get("--public-host"));
  const { tls } = parseDaemonTransport({ flags, options }, host, publicHosts);
  const port = options.get("--port");
  const stateFile = options.get("--state-file");
  const workspaceRoot = options.get("--workspace-root");
  return {
    host,
    publicHosts,
    ...(port === undefined ? {} : { port: parsePort(port) }),
    dev: flags.has("--dev"),
    ...(stateFile === undefined ? {} : { stateFile: resolve(stateFile) }),
    ...(workspaceRoot === undefined ? {} : { workspaceRoot: resolve(workspaceRoot) }),
    ...(tls === undefined ? {} : { tls }),
  };
}

export function parsePort(text: string): number {
  const port = Number(text);
  if (!Number.isInteger(port) || port < 0 || port > 65_535) throw new Error(`Invalid port: ${text}`);
  return port;
}

async function packageVersion(): Promise<string> {
  const manifestPath = join(PACKAGE_ROOT, "package.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8")) as { version?: unknown };
  return typeof manifest.version === "string" ? manifest.version : "0.0.0";
}

/**
 * What starts the daemon with besides its arguments: what the process entry point (`src/cli/daemon-entry.ts`) supplies
 * from the layers above the daemon, the record check of the home's one daemon, which only `runHomeDaemon` sets, and why
 * it serves this computer alone, which only `serveRerailDaemon`'s loopback restart sets.
 */
type RerailDaemonServices = Pick<RerailDaemonOptions, "codingAgents" | "statusLines" | "recordCheck" | "remoteRefused">;

/**
 * Runs the daemon (`serveRerailDaemon`) as the one daemon of its Rerail home, whatever the build. It takes the home's
 * daemon lock before anything opens a store or binds a pipe, a socket or a port, so a second daemon stops here naming
 * the first: two starts at once, or a start whose wait for a slow daemon ran out, could otherwise leave two, one holding
 * the port and the other the one daemon.json names. Only the holder keeps daemon.json (`recordCheck`): two daemons
 * that each did would write over each other. The lock goes with the process however it ends; an exit also removes the
 * holder's record beside it. A loopback restart for a TLS key it may not serve with runs in this process and keeps the
 * lock. A restart at the heap ceiling launches the next daemon just before this one exits, and that one keeps trying
 * for the lock for a second. From the lock to the end of its start, the record beside the lock says it is starting
 * (`homeDaemonStarting`), so a start that finds it then waits for it rather than taking it for a daemon that does not
 * answer or launching a second one.
 */
export async function runHomeDaemon(argv: readonly string[], services: Omit<RerailDaemonServices, "remoteRefused"> = {}): Promise<void> {
  const home = defaultRerailHome();
  const homeLock = await holdHomeDaemonLock(home);
  if (homeLock === undefined) return await serveRerailDaemon(argv, services);
  process.once("exit", () => homeLock.release());
  await serveRerailDaemon(argv, { ...services, recordCheck: { everyMs: RECORD_CHECK_MS, home, holdsHome: () => homeLock.holdsHome() } });
  // Its start is over: it has recorded itself, said where it listens, and its one thread is free to answer. Until now
  // its record beside the lock said it was starting, and a start that found it waited for this.
  homeLock.started();
}

/**
 * The daemon flags and options (each followed by its value) that make it reachable from other machines: where it binds
 * and answers, and every transport one (`DAEMON_TRANSPORT_FLAGS`, `DAEMON_TRANSPORT_OPTIONS`), so one added there is
 * left out of a loopback daemon's arguments too.
 */
const REMOTE_FLAGS: readonly string[] = ["--allow-remote", ...DAEMON_TRANSPORT_FLAGS];
const REMOTE_OPTIONS: readonly string[] = ["--host", "--public-host", ...DAEMON_TRANSPORT_OPTIONS];

/** `argv` without the flags and options that make a daemon reachable from other machines: a loopback daemon's. */
export function loopbackArguments(argv: readonly string[]): string[] {
  const kept: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index] ?? "";
    if (REMOTE_OPTIONS.includes(argument)) index += 1;
    else if (!REMOTE_FLAGS.includes(argument)) kept.push(argument);
  }
  return kept;
}

/** What daemon.log, `rerail daemon start` and `rerail status` say of a daemon that serves this computer alone. */
export function loopbackOnlyNotice(reason: string): string {
  return `This daemon serves this computer alone, over plain http on loopback, until its TLS key is fixed: ${reason}`;
}

/** The daemon argument that names why a daemon serves this computer alone, as its loopback restart leaves it. */
const REMOTE_REFUSED = "--remote-refused";

/** The arguments of a daemon that serves this computer alone for `reason`: `argv`'s loopback ones, then why. */
export function loopbackCommandLine(argv: readonly string[], reason: string): string[] {
  return [...loopbackArguments(argv), REMOTE_REFUSED, reason];
}

/** Why a daemon started with `argv` serves this computer alone (`--remote-refused <why>`), and its other arguments. */
export function remoteRefusal(argv: readonly string[]): { argv: string[]; reason: string } | undefined {
  const at = argv.indexOf(REMOTE_REFUSED);
  const reason = at === -1 ? undefined : argv[at + 1];
  return reason === undefined ? undefined : { argv: [...argv.slice(0, at), ...argv.slice(at + 2)], reason };
}

/**
 * The daemon process: `runRerailDaemon`, except that a TLS key it may not serve with (`TlsKeyExposedError`), which keeps
 * it from serving https and from other machines, no longer keeps it from this one. The daemon starts again without the
 * options that reach other machines (`serveLoopback`): on loopback, over http, as a daemon started with no options is,
 * so hooks keep recording and steering, and the state file, daemon.log, `rerail daemon start` and `rerail status` say
 * why (`loopbackOnlyNotice`). The refusal comes before anything is opened, so the second start begins from nothing. A
 * daemon launched with `--remote-refused`, as a restart of such a daemon is, serves this computer alone from the start.
 * Its caller supplies only the services of the layers above: why the daemon serves this computer alone is this
 * function's alone to say, so no caller can record a daemon other machines reach as one they cannot. A key that is
 * empty or no key (`TlsKeyUnparsedError`) is served on in the same way.
 */
export async function serveRerailDaemon(argv: readonly string[], services: Omit<RerailDaemonServices, "remoteRefused"> = {}): Promise<void> {
  const relaunched = remoteRefusal(argv);
  if (relaunched !== undefined) return await serveLoopback(relaunched.argv, relaunched.reason, services);
  try {
    await runRerailDaemon(argv, services);
  } catch (error) {
    if (!(error instanceof TlsKeyExposedError) && !(error instanceof TlsKeyUnparsedError)) throw error;
    await serveLoopback(argv, error.message, services);
  }
}

/**
 * Runs the daemon for this computer alone, for `reason`: with `argv` less what reaches other machines
 * (`loopbackArguments`), saying why on stderr, which is daemon.log, once for each start. The process takes the arguments
 * of such a daemon (`loopbackCommandLine`) as if it had been started with them, as the hook launcher runs a hook: a
 * restart at the heap ceiling relaunches the daemon with this process's arguments (`runRerailDaemon`), so the next
 * daemon serves this computer alone too, saying why, even once the key is fixed, until someone starts it again.
 */
async function serveLoopback(argv: readonly string[], reason: string, services: Omit<RerailDaemonServices, "remoteRefused">): Promise<void> {
  process.stderr.write(`rerail daemon: ${loopbackOnlyNotice(reason)}\n`);
  process.argv.splice(2, process.argv.length - 2, ...loopbackCommandLine(argv, reason));
  await runRerailDaemon(loopbackArguments(argv), { ...services, remoteRefused: reason });
}

/** The page the daemon serves: `--dev` serves the committed page straight from `ui/`, so an edit needs only a reload. */
function daemonPage(arguments_: DaemonArguments): { dev: boolean; uiRoot: string } {
  const dev = arguments_.dev || process.env.RERAIL_DEV === "1";
  const uiRoot = dev ? sourceUiRoot() : bundledUiRoot();
  if (!existsSync(join(uiRoot, "index.html"))) {
    throw new Error(`Rerail page is missing at ${uiRoot}; run npm run build:ui.`);
  }
  return { dev, uiRoot };
}

/** The daemon options its arguments set only when given: its port, its state file and its TLS material. */
function optionalDaemonOptions(arguments_: DaemonArguments, tls: DaemonTlsMaterial | undefined): Pick<RerailDaemonOptions, "port" | "stateFile" | "tls"> {
  return {
    ...(arguments_.port === undefined ? {} : { port: arguments_.port }),
    ...(arguments_.stateFile === undefined ? {} : { stateFile: arguments_.stateFile }),
    ...(tls === undefined ? {} : { tls }),
  };
}

/** What a started daemon says on stdout, which is daemon.log: where it listens, and what it runs with. */
function reportStart(daemon: RerailDaemon): void {
  const health = daemon.health();
  process.stdout.write(`rerail daemon ${health.version} listening at ${daemon.url} (pid ${process.pid})\n`);
  const { tlsFingerprint256 } = daemon.state();
  // What a browser shows for a self-signed certificate, to compare before accepting it.
  if (tlsFingerprint256 !== undefined) process.stdout.write(`TLS certificate SHA-256: ${tlsFingerprint256}\n`);
  process.stdout.write(`database: ${health.database.path}\n`);
  process.stdout.write(
    health.native.ok
      ? `native kernel: ok (version ${health.native.kernelVersion})\n`
      : `native kernel: unavailable: ${health.native.error}\n`,
  );
  process.stdout.write(`model: ${health.model.message}\n`);
  const catalog = loadRuleCatalog();
  const packs = [...catalog.packs.values()].map(pack => `${pack.id}@${pack.version ?? "unversioned"}`);
  process.stdout.write(`rule packs: ${packs.length === 0 ? "none installed" : packs.join(", ")}\n`);
  // A pack left out is otherwise only visible to someone who asks (`rerail packs list`, `GET /api/rule-packs`).
  for (const skipped of catalog.skipped) {
    process.stdout.write(`rule pack left out: ${skipped.packageName ?? skipped.path}${skipped.version === null ? "" : `@${skipped.version}`}: ${skipped.reason}\n`);
  }
}

async function runRerailDaemon(argv: readonly string[], services: RerailDaemonServices = {}): Promise<void> {
  const arguments_ = parseDaemonArguments(argv);
  // The launcher starts the daemon in the Rerail home and names the checkout it serves (`launchDaemonProcess`). Git is
  // looked for from that checkout, before anything runs Git, so its PATH entries are skipped as a hook skips them. Only
  // the daemon, which lives on, asks whether the home share is there: off the network that ask is pending about 21 s,
  // and it would keep a hook or a command alive that long after its work.
  const workspaceRoot = arguments_.workspaceRoot ?? process.cwd();
  prepareDaemonGit(workspaceRoot);
  const tls = arguments_.tls === undefined ? undefined : await loadDaemonTls(arguments_.tls);
  // Before anything opens a store: the main daemon is the machine's one main instance, and a second one stops
  // here naming the first. A checkout's daemon is a dev build with its own content store and skips this lock. Every
  // build, a checkout's included, takes the home's daemon lock before this, in `runHomeDaemon`.
  const build = isMainRerailBuild() ? currentRerailBuild() : undefined;
  const mainInstance = build?.kind === "main" ? acquireMainInstance(build) : undefined;
  const { dev, uiRoot } = daemonPage(arguments_);
  const databasePath = defaultRerailDatabasePath();
  // A signal (macOS, Linux) or a shutdown request (Windows, where `process.kill` ends a process without running any
  // handler) closes the daemon. `rerail daemon stop` kills a daemon that has not stopped within its wait.
  let shutdown = (): void => undefined;
  const daemon = await RerailDaemon.start({
    onShutdownRequest: () => shutdown(),
    version: await packageVersion(),
    databasePath,
    workspaceRoot,
    host: arguments_.host,
    publicHosts: arguments_.publicHosts,
    portFile: join(defaultRerailHome(), DAEMON_PORT_FILE),
    accessTokenFile: join(defaultRerailHome(), DAEMON_ACCESS_TOKEN_FILE),
    uiRoot,
    dev,
    ...services,
    ...optionalDaemonOptions(arguments_, tls),
  });
  reportStart(daemon);
  const way = daemonExit({
    close: async () => await daemon.close(),
    release: () => mainInstance?.release(),
    // The same arguments and the same launch as `rerail daemon start`, once this daemon's port, socket and state are free.
    relaunch: () => {
      relaunchDaemon(process.argv.slice(1), join(defaultRerailHome(), DAEMON_LOG_FILE));
    },
    exit: code => process.exit(code),
    log: line => process.stderr.write(line),
  });
  shutdown = way.stop;
  process.once("SIGTERM", way.stop);
  process.once("SIGINT", way.stop);
  watchHeap({
    onCeiling: ({ used, limit }) => {
      process.stderr.write(`rerail daemon: heap at ${Math.round(used / 1e6)} MB of its ${Math.round(limit / 1e6)} MB limit; restarting before it runs out\n`);
      way.restart();
    },
  });
}
