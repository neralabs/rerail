import { createHash, randomBytes } from "node:crypto";
import { createReadStream, existsSync, realpathSync } from "node:fs";
import { readFile, stat, writeFile, rm, rename } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { dirname, extname, join, normalize, resolve, sep } from "node:path";
import { tmpdir } from "node:os";
import { Readable } from "node:stream";
import { fileURLToPath } from "node:url";
import { bundledModelPath, PACKAGE_ROOT } from "../engine/package-root.js";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import { DAEMON_ACCESS_TOKEN_FILE, type DaemonHealth, type DaemonState, type NativeStatus } from "../runtime/daemon-control.js";
import { startEngineService, type EngineService } from "../runtime/engine-service.js";
import { NativeSessionServer } from "./native-session-server.js";
import { getRequestListener } from "@hono/node-server";
import { Hono } from "hono";
import { accessCapability, createRerailApi, hostAllowlist, publicHostSet, sameSecret } from "../api/http.js";
import { createRerailMcpHandler } from "../api/mcp.js";
import { operations } from "../api/operations.js";
import { DatabaseApiStores } from "../api/stores.js";
import { nativeHealth as loadNativeHealth, type NativeHealth } from "../engine/native/native.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { defaultRerailDatabasePath } from "../engine/storage/database.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { acquireMainInstance } from "../engine/native/main-instance.js";
import { currentRerailBuild, isMainRerailBuild } from "../engine/native/rerail-build.js";

export const DEFAULT_DAEMON_HOST = "127.0.0.1";
export const DEFAULT_DAEMON_PORT = 4737;
/** In the Rerail home: the port the daemon last listened on, which the next start without `--port` prefers. */
export const DAEMON_PORT_FILE = "daemon-port.json";

export interface RerailDaemonOptions {
  version: string;
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
  /** The coding agent check `GET /api/coding-agents` reports; the daemon's entry point passes set-up's. */
  codingAgents?: () => Promise<CodingAgentState[]>;
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

/** A short, database-derived local IPC endpoint isolates each Rerail home's engine service. */
export function defaultEngineSocketPath(
  databasePath: string,
  platform: NodeJS.Platform = process.platform,
): string {
  const identity = createHash("sha256").update(resolve(databasePath)).digest("hex").slice(0, 24);
  return platform === "win32" ? String.raw`\\.\pipe\rerail-${identity}` : join(tmpdir(), `rerail-${identity}.sock`);
}

async function listen(server: Server, port: number, host: string): Promise<void> {
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
async function listenPreferred(server: Server, port: number | undefined, remembered: number | undefined, host: string): Promise<void> {
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

/** `content` as the whole of `file`, readable by its owner only, through a temporary file renamed over it. */
async function writePrivateFile(file: string, content: string): Promise<void> {
  const temporaryFile = `${file}.${process.pid}.tmp`;
  try {
    await writeFile(temporaryFile, content, { mode: 0o600 });
    await rename(temporaryFile, file);
  } finally {
    await rm(temporaryFile, { force: true });
  }
}

const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/** Remembering the port is a convenience: a failed write leaves the daemon running and is only reported. */
async function rememberPort(portFile: string, port: number): Promise<void> {
  try {
    await writePrivateFile(portFile, `${JSON.stringify({ port })}\n`);
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
    await writePrivateFile(file, `${token}\n`);
  } catch (error) {
    process.stderr.write(`rerail daemon: could not keep the access capability in ${file}: ${errorText(error)}\n`);
  }
  return token;
}

async function closeHttpServer(server: Server): Promise<void> {
  server.closeIdleConnections();
  const closed = Promise.withResolvers<void>();
  server.close((error) => (error === undefined ? closed.resolve() : closed.reject(error)));
  server.closeAllConnections();
  await closed.promise;
}

export class RerailDaemon {
  readonly host: string;
  readonly port: number;
  readonly url: string;
  readonly startedAt: number;
  readonly #server: Server;
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
  #closed = false;

  private constructor({
    server, engineService, kernelSocketPath, stores, options, databasePath, native, ui, workspaceRoot, nativeServer, nativeToken, accessToken,
  }: {
    server: Server;
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
  }) {
    const address = server.address() as AddressInfo;
    this.host = address.address;
    this.port = address.port;
    this.url = `http://${this.host}:${this.port}/`;
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
  }

  static async start(options: RerailDaemonOptions): Promise<RerailDaemon> {
    const bindHost = options.host ?? DEFAULT_DAEMON_HOST;
    const loopbackBind = LOOPBACK_BIND_HOSTS.includes(bindHost.toLowerCase());
    // A daemon bound off loopback (`--allow-remote`) is dialled by its own literal address, e.g. a
    // `http://0.0.0.0:4737/` state URL. An IP literal cannot arrive by DNS rebinding —
    // that attack's `Host` is a DNS name — so the bound address is as safe to answer as loopback.
    const publicHosts = publicHostSet([
      ...(options.publicHosts ?? []),
      ...(loopbackBind ? [] : [bindHost.includes(":") ? `[${bindHost}]` : bindHost]),
    ]);
    // Every request carries the daemon's access capability: `Host`/`Origin` are only what a client claims, and on
    // loopback another user of the machine, or a sandboxed process allowed localhost, reaches the socket as the person
    // would. A loopback daemon keeps its capability in the home across restarts, so a browser that holds it keeps
    // working; one reachable off loopback (`--allow-remote`, or a `--public-host` a reverse proxy forwards) mints a new
    // one on every start.
    const remote = !loopbackBind || (options.publicHosts ?? []).length > 0;
    const accessToken = options.accessToken
      ?? (remote || options.accessTokenFile === undefined ? newAccessToken() : await persistentAccessToken(options.accessTokenFile));
    const uiRoot = options.uiRoot === undefined ? undefined : realpathSync(options.uiRoot);
    const ui: DaemonState["ui"] = uiRoot === undefined || options.dev === true ? "dev" : "bundled";
    const native = readNativeStatus(options.nativeHealth ?? loadNativeHealth);
    const databasePath = resolve(options.databasePath ?? defaultRerailDatabasePath());
    const kernelSocketPath = options.kernelSocketPath ?? defaultEngineSocketPath(databasePath);
    const workspaceRoot = resolve(options.workspaceRoot ?? process.cwd());
    const storageRoot = join(workspaceRoot, ".rerail", "sessions");
    const modelPath = bundledModelPath();
    let daemon: RerailDaemon | undefined;
    // Open the database before binding anything: a refused open (a file this build cannot read, say) must leave no
    // engine socket behind, or every later start fails with EADDRINUSE.
    const stores = new DatabaseApiStores(databasePath, {
      health: () => daemon!.health(),
      origin: () => new URL(daemon!.url).origin,
      ...(options.codingAgents === undefined ? {} : { codingAgents: options.codingAgents }),
    });
    let engineService: EngineService | undefined;
    let nativeServer: NativeSessionServer | undefined;
    let server: Server | undefined;
    try {
      engineService = await startEngineService({
        socketPath: kernelSocketPath,
        storageRoot,
        modelPath,
        databasePath,
        workspaceRoot,
      });
      const nativeToken = randomBytes(32).toString("hex");
      const sessions = new NativeSessionServer({ workspaceRoot, modelPath, databasePath });
      nativeServer = sessions;
      const mcp = createRerailMcpHandler(stores, operations, { version: options.version, publicHosts });
      const app = new Hono();
      app.use("*", hostAllowlist(publicHosts));
      app.use("*", accessCapability(accessToken));
      app.get("/healthz", context => context.json(daemon!.health()));

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
      app.onError((error, context) => context.json({ error: "internal", message: error.message }, 500));
      const listener = getRequestListener(app.fetch);
      server = createServer((request, response) => void listener(request, response));
      const remembered = options.portFile === undefined ? undefined : await readRememberedPort(options.portFile);
      await listenPreferred(server, options.port, remembered, bindHost);
      daemon = new RerailDaemon({
        server, engineService, kernelSocketPath, stores, options, databasePath, native, ui, workspaceRoot,
        nativeServer: sessions, nativeToken, accessToken,
      });
      if (options.stateFile !== undefined) await writePrivateFile(options.stateFile, `${JSON.stringify(daemon.state(), null, 2)}\n`);
      // A start that asked for any free port (`0`) takes a throwaway one; it does not replace the port links point at.
      if (options.portFile !== undefined && options.port !== 0 && daemon.port !== remembered) await rememberPort(options.portFile, daemon.port);
      return daemon;
    } catch (error) {
      // Every one of them, whatever another does: a socket left bound makes every later start fail (EADDRINUSE); the
      // database last, once nothing uses it. The start's own error is the one thrown; a close that failed too is
      // written to stderr, so a later EADDRINUSE has its cause on record.
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
      throw error;
    }
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
    };
  }

  health(): DaemonHealth {
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
      ui: this.#ui,
      workspaceRoot: this.#workspaceRoot,
      installedAt: this.#stores.installedAt(),
    };
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    try {
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

function parseDaemonArguments(argv: readonly string[]): DaemonArguments {
  const { flags, options } = parseArguments(
    argv,
    { flags: ["--dev", "--allow-remote"], options: ["--host", "--port", "--state-file", "--public-host"] },
    (argument) => `Unknown daemon argument: ${argument}`,
  );
  const host = options.get("--host") ?? DEFAULT_DAEMON_HOST;
  assertHostBindable(host, flags.has("--allow-remote"));
  const port = options.get("--port");
  const stateFile = options.get("--state-file");
  return {
    host,
    publicHosts: parsePublicHosts(options.get("--public-host")),
    ...(port === undefined ? {} : { port: parsePort(port) }),
    dev: flags.has("--dev"),
    ...(stateFile === undefined ? {} : { stateFile: resolve(stateFile) }),
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

/** What the process entry point (`src/cli/daemon-entry.ts`) supplies from the layers above the daemon. */
export type RerailDaemonServices = Pick<RerailDaemonOptions, "codingAgents">;

export async function runRerailDaemon(argv: readonly string[], services: RerailDaemonServices = {}): Promise<void> {
  const arguments_ = parseDaemonArguments(argv);
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
  const workspaceRoot = process.cwd();
  const databasePath = defaultRerailDatabasePath();
  const daemon = await RerailDaemon.start({
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
  });
  const health = daemon.health();
  process.stdout.write(`rerail daemon ${health.version} listening at ${daemon.url} (pid ${process.pid})\n`);
  process.stdout.write(`database: ${health.database.path}\n`);
  process.stdout.write(
    health.native.ok
      ? `native kernel: ok (version ${health.native.kernelVersion})\n`
      : `native kernel: unavailable: ${health.native.error}\n`,
  );
  const catalog = loadRuleCatalog();
  const packs = [...catalog.packs.values()].map(pack => `${pack.id}@${pack.version ?? "unversioned"}`);
  process.stdout.write(`rule packs: ${packs.length === 0 ? "none installed" : packs.join(", ")}\n`);
  // A pack left out is otherwise only visible to someone who asks (`rerail packs list`, `GET /api/rule-packs`).
  for (const skipped of catalog.skipped) {
    process.stdout.write(`rule pack left out: ${skipped.packageName ?? skipped.path}${skipped.version === null ? "" : `@${skipped.version}`}: ${skipped.reason}\n`);
  }
  const shutdown = (): void => {
    daemon.close().then(
      () => {
        mainInstance?.release();
        process.exit(0);
      },
      (error) => {
        process.stderr.write(`rerail daemon: shutdown failed: ${(error as Error).message}\n`);
        process.exit(1);
      },
    );
  };
  process.once("SIGTERM", shutdown);
  process.once("SIGINT", shutdown);
}
