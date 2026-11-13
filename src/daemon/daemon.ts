import { randomBytes, X509Certificate } from "node:crypto";
import { createReadStream, existsSync, realpathSync } from "node:fs";
import { readFile, stat, rm } from "node:fs/promises";
import { createServer, type RequestListener, type Server as HttpServer } from "node:http";
import { createServer as createHttpsServer } from "node:https";
import type { AddressInfo, Server as NetServer } from "node:net";
import { basename, dirname, extname, join, normalize, resolve, sep } from "node:path";
import { tmpdir, userInfo } from "node:os";
import { Readable } from "node:stream";
import { fileURLToPath } from "node:url";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { replaceFile } from "../engine/file-replace.js";
import { searchGitFrom } from "../engine/git.js";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import { DAEMON_ACCESS_TOKEN_FILE, DAEMON_LOG_FILE, relaunchDaemon, sameSecret, type DaemonHealth, type DaemonLaunchOptions, type DaemonState, type DaemonTlsFiles, type NativeStatus } from "../runtime/daemon-control.js";
import { daemonExit } from "./daemon-exit.js";
import { watchHeap } from "./heap-ceiling.js";
import { startRemoteSink, type RemoteSinkLoop } from "../runtime/remote-outbox.js";
import { chooseEngineSocketPath, startEngineService, type EngineService } from "../runtime/engine-service.js";
import { engineIdentity, enginePipeName, fallbackPipe } from "../runtime/engine-pipe.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import { NativeSessionServer } from "./native-session-server.js";
import { getRequestListener } from "@hono/node-server";
import { Hono } from "hono";
import { HTTPException } from "hono/http-exception";
import { accessCapability, createRerailApi, hostAllowlist, publicHostSet, requestBodyLimit } from "../api/http.js";
import { createRerailMcpHandler } from "../api/mcp.js";
import { operations } from "../api/operations.js";
import { DatabaseApiStores } from "../api/stores.js";
import { nativeHealth as loadNativeHealth, type NativeHealth } from "../engine/native/native.js";
import { modelStatus } from "../engine/native/model-status.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { defaultRerailDatabasePath } from "../engine/storage/database.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { openToOthers, type AccessReader } from "../engine/storage/private-files.js";
import { acquireMainInstance } from "../engine/native/main-instance.js";
import { currentRerailBuild, isMainRerailBuild } from "../engine/native/rerail-build.js";

export const DEFAULT_DAEMON_HOST = "127.0.0.1";
export const DEFAULT_DAEMON_PORT = 4737;
/** In the Rerail home: the port the daemon last listened on, which the next start without `--port` prefers. */
export const DAEMON_PORT_FILE = "daemon-port.json";

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
  /** Written once the server is listening and removed on close. */
  stateFile?: string;
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
   * The certificate (a PEM chain, leaf first) and private key to serve https with; omitted, the daemon serves http.
   * The state file records the leaf's SHA-256 fingerprint, which Rerail's own clients pin.
   */
  tls?: DaemonTlsMaterial;
  /**
   * Why a daemon started with a TLS key serves this computer alone, over plain http on loopback: other accounts can read
   * the key (`serveRerailDaemon`). Its arguments asked for https, and may have asked to be reachable from other
   * machines. The state file records it, for `rerail status` to say.
   */
  remoteRefused?: string;
  /**
   * Called when a Rerail process asks the daemon to stop (`POST /native/shutdown`, which `rerail daemon stop` sends on
   * Windows, where no signal reaches a handler). Omitted, the daemon serves no such request.
   */
  onShutdownRequest?: () => void;
}

/** What `--tls-cert` and `--tls-key` name, read: PEM text. */
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
}

/**
 * Every one of them, whatever another does: a socket left bound makes every later start fail (EADDRINUSE); the
 * database last, once nothing uses it. The start's own error is the one thrown; a close that failed too is written to
 * stderr, so a later EADDRINUSE has its cause on record.
 */
async function closeAfterFailedStart({ engineService, nativeServer, server }: StartedServices, stores: DatabaseApiStores): Promise<void> {
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
  try {
    stores.close();
  } catch (reason) {
    failed("the database", reason);
  }
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
  readonly #nativeToken: string;
  readonly #accessToken: string;
  readonly #tlsFingerprint256: string | undefined;
  #closed = false;
  #remoteSink: RemoteSinkLoop | undefined;

  private constructor({
    server, engineService, kernelSocketPath, stores, options, databasePath, native, ui, workspaceRoot, nativeServer, nativeToken, accessToken,
    tlsFingerprint256,
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
    nativeToken: string;
    accessToken: string;
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
    this.#nativeToken = nativeToken;
    this.#accessToken = accessToken;
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
    // Open the database before binding anything: a refused open (a file this build cannot read, say) must leave no
    // engine socket behind, or every later start fails with EADDRINUSE.
    const stores = new DatabaseApiStores(databasePath, {
      health: () => daemon!.health(),
      origin: () => new URL(daemon!.url).origin,
      ...(options.codingAgents === undefined ? {} : { codingAgents: options.codingAgents }),
    });
    const started: StartedServices = {};
    try {
      const engineService = await startEngineService({
        socketPath: kernelSocketPath,
        storageRoot,
        modelPath,
        databasePath,
        workspaceRoot,
        accessToken,
        // On Windows any local user can create the default pipe first; the engine then listens on a random name.
        ...(options.kernelSocketPath === undefined
          ? { pipeFallback: () => fallbackPipe(kernelSocketPath) }
          : {}),
      });
      started.engineService = engineService;
      const nativeToken = randomBytes(32).toString("hex");
      const sessions = new NativeSessionServer({ workspaceRoot, modelPath, databasePath });
      started.nativeServer = sessions;
      const app = daemonApp({ stores, options, publicHosts, accessToken, nativeToken, sessions, workspaceRoot, uiRoot, health: () => daemon!.health() });
      const server = createDaemonServer(options, app);
      started.server = server;
      const remembered = options.portFile === undefined ? undefined : await readRememberedPort(options.portFile);
      await listenPreferred(server, options.port, remembered, bindHost);
      daemon = new RerailDaemon({
        server, engineService, kernelSocketPath: engineService.socketPath, stores, options, databasePath, native, ui, workspaceRoot,
        nativeServer: sessions, nativeToken, accessToken, tlsFingerprint256,
      });
      await daemon.#announce(remembered);
      return daemon;
    } catch (error) {
      await closeAfterFailedStart(started, stores);
      throw error;
    }
  }

  /** What a started daemon leaves behind for others: its state file, its remote sink and the port links point at. */
  async #announce(remembered: number | undefined): Promise<void> {
    const options = this.#options;
    if (options.stateFile !== undefined) await replaceFile(options.stateFile, `${JSON.stringify(this.state(), null, 2)}\n`, { mode: 0o600 });
    // Internal testing: the remote sink sends nothing until a remote is added (`rerail remote add`).
    if (options.remoteSink !== false) this.#remoteSink = startRemoteSink({ home: dirname(this.#databasePath), ...(options.remoteSink ?? {}) });
    // A start that asked for any free port (`0`) takes a throwaway one; it does not replace the port links point at.
    if (options.portFile !== undefined && options.port !== 0 && this.port !== remembered) await rememberPort(options.portFile, this.port);
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
    };
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    try {
      await this.#remoteSink?.stop();
      await closeHttpServer(this.#server);
    } finally {
      try {
        await this.#nativeServer.close();
        await this.#engineService.close();
      } finally {
        this.#stores.close();
        if (this.#options.stateFile !== undefined) await rm(this.#options.stateFile, { force: true });
      }
    }
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
 * A TLS private key file this account reads that the daemon may not serve with because other accounts can read it too.
 * Its message names the key and the command that makes it its owner's alone.
 */
export class TlsKeyExposedError extends Error {
  override readonly name = "TlsKeyExposedError";
}

/**
 * The PEM files `tls` names, read once at start. The private key must be its owner's alone (`openToOthers`): a key
 * other users can read lets any of them impersonate the daemon, so a key with any group or other permission is refused,
 * as ssh refuses one, and on Windows, which has no mode bits, a key whose ACL lets any account beyond the person,
 * SYSTEM and Administrators read it, as OpenSSH for Windows refuses one. A key outside the profile folder, under
 * `C:\certs` say, inherits an ACL that lets every local user read it. A key file this account reads and others can read
 * too is a `TlsKeyExposedError`, on which the daemon serves this computer alone (`serveRerailDaemon`). Any other path
 * that is no key this account can read (a missing or misspelled one, a folder, a file this account may not read, one
 * whose readers cannot be read) is a mistake in the command and stops the start. `access` is for tests.
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
    const refusal = `${tls.keyFile} ${others}; a TLS private key must be readable by its owner only: ${hint}`;
    throw (await readsAsFile(tls.keyFile)) ? new TlsKeyExposedError(refusal) : new Error(refusal);
  }
  const [cert, key] = await Promise.all([readFile(tls.certFile, "utf8"), readFile(tls.keyFile, "utf8")]);
  return { cert, key };
}

/** Whether this account can read `path` as a file. A folder or FIFO is never read: a FIFO's read waits for a writer. */
async function readsAsFile(path: string): Promise<boolean> {
  try {
    if (!(await stat(path)).isFile()) return false;
    await readFile(path);
    return true;
  } catch {
    return false;
  }
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
 * What `runRerailDaemon` starts the daemon with: what the process entry point (`src/cli/daemon-entry.ts`) supplies from
 * the layers above the daemon, and why it serves this computer alone, which only `serveRerailDaemon`'s loopback restart
 * sets.
 */
type RerailDaemonServices = Pick<RerailDaemonOptions, "codingAgents" | "remoteRefused">;

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

/**
 * The daemon process: `runRerailDaemon`, except that a TLS key it may not serve with (`TlsKeyExposedError`), which keeps
 * it from serving https and from other machines, no longer keeps it from this one. The daemon starts again without the
 * options that reach other machines (`loopbackArguments`): on loopback, over http, as a daemon started with no options
 * is, so hooks keep recording and steering, and the state file, daemon.log, `rerail daemon start` and `rerail status`
 * say why (`loopbackOnlyNotice`). The refusal comes before anything is opened, so the second start begins from nothing.
 * Its caller supplies only the services of the layers above: why the daemon serves this computer alone is this
 * function's alone to say, so no caller can record a daemon other machines reach as one they cannot.
 */
export async function serveRerailDaemon(argv: readonly string[], services: Pick<RerailDaemonOptions, "codingAgents"> = {}): Promise<void> {
  try {
    await runRerailDaemon(argv, services);
  } catch (error) {
    if (!(error instanceof TlsKeyExposedError)) throw error;
    process.stderr.write(`rerail daemon: ${loopbackOnlyNotice(error.message)}\n`);
    await runRerailDaemon(loopbackArguments(argv), { ...services, remoteRefused: error.message });
  }
}

async function runRerailDaemon(argv: readonly string[], services: RerailDaemonServices = {}): Promise<void> {
  const arguments_ = parseDaemonArguments(argv);
  // The launcher starts the daemon in the Rerail home and names the checkout it serves (`launchDaemonProcess`). Git is
  // looked for from that checkout, before anything runs Git, so its PATH entries are skipped as a hook skips them.
  const workspaceRoot = arguments_.workspaceRoot ?? process.cwd();
  searchGitFrom(workspaceRoot);
  const tls = arguments_.tls === undefined ? undefined : await loadDaemonTls(arguments_.tls);
  // Before anything opens a store: the main daemon is the machine's one main instance, and a second one stops
  // here naming the first. A checkout's daemon is a dev build with its own content store and takes no lock.
  const build = isMainRerailBuild() ? currentRerailBuild() : undefined;
  const mainInstance = build?.kind === "main" ? acquireMainInstance(build) : undefined;
  // `--dev` serves the committed page straight from `ui/`, so an edit needs only a reload.
  const dev = arguments_.dev || process.env.RERAIL_DEV === "1";
  const uiRoot = dev ? sourceUiRoot() : bundledUiRoot();
  if (!existsSync(join(uiRoot, "index.html"))) {
    throw new Error(`Rerail page is missing at ${uiRoot}; run npm run build:ui.`);
  }
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
    ...(arguments_.port === undefined ? {} : { port: arguments_.port }),
    portFile: join(defaultRerailHome(), DAEMON_PORT_FILE),
    accessTokenFile: join(defaultRerailHome(), DAEMON_ACCESS_TOKEN_FILE),
    uiRoot,
    dev,
    ...services,
    ...(arguments_.stateFile === undefined ? {} : { stateFile: arguments_.stateFile }),
    ...(tls === undefined ? {} : { tls }),
  });
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
