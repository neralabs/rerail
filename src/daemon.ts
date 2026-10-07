import { createHash, randomBytes } from "node:crypto";
import { createReadStream, existsSync, realpathSync } from "node:fs";
import { readFile, stat, writeFile, rm, rename } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { dirname, extname, join, normalize, resolve, sep } from "node:path";
import { tmpdir } from "node:os";
import { Readable } from "node:stream";
import { fileURLToPath } from "node:url";
import { KernelReceiptSchema, parseKernelRequest, startKernelService, type KernelRequest, type KernelService } from "./kernel-service.js";
import { DevNativeServer } from "./dev-native-server.js";
import { getRequestListener } from "@hono/node-server";
import { Hono, type Context } from "hono";
import { withinCheckout } from "./checkout-control.js";
import { createRerailApi, hostAllowlist, publicHostSet } from "./api/http.js";
import { createRerailMcpHandler } from "./api/mcp.js";
import { operations, type RerailApiStores } from "./api/operations.js";
import { nativeHealth as loadNativeHealth, type NativeHealth } from "./native.js";
import { loadBundledRules } from "./rule-bootstrap.js";
import { effectiveRepositoryStartState, effectiveStartState, startStateReader, type RepositoryStartState } from "./repository-start.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "./database.js";
import {
  openScopedRuleCatalog,
  resolveRepositoryContext,
  ScopedRuleCatalog,
  type RepositoryContext,
} from "./scoped-rule-catalog.js";
import { SqlitePolicyExecutionReader } from "./policy-execution-store.js";
import { SqliteScanControlStore } from "./scan-control.js";
import { SqliteTranscriptReader, SqliteTranscriptStore } from "./transcript-store.js";
import { acquireMainInstance } from "./main-instance.js";
import { currentRerailBuild, isMainRerailBuild } from "./rerail-build.js";
import { SqlitePresentationPreferenceStore } from "./presentation-preference-store.js";

export const DEFAULT_DAEMON_HOST = "127.0.0.1";
export const DEFAULT_DAEMON_PORT = 4737;

/** What `rerail status` and the launcher need to find a running daemon. */
export interface DaemonState {
  pid: number;
  host: string;
  port: number;
  url: string;
  version: string;
  /** The Rerail database: rules and every repository's history. */
  databasePath: string;
  /** Local IPC endpoint used by harness adapters: a Unix socket or Windows named pipe. */
  kernelSocketPath: string;
  /**
   * `bundled` serves the packaged page from `dist/ui`; `dev` is a daemon started with `--dev`
   * (or `RERAIL_DEV=1`), serving the committed `ui/` source, or one embedded without any page.
   */
  ui: "bundled" | "dev";
  startedAt: number;
  /**
   * The package the daemon was launched from; `rerail setup` restarts a daemon whose root is no
   * longer `current`. Absent in state files written before this field existed.
   */
  packageRoot?: string;
  workspaceRoot?: string;
  /** Same-user capability for normal native sessions; never exposed by health/API responses. */
  nativeToken?: string;
}

export type NativeStatus = { ok: true; kernelVersion: string } | { ok: false; error: string };

export interface DaemonHealth extends Pick<DaemonState, "version" | "pid" | "ui"> {
  status: "ok";
  uptimeMs: number;
  database: { path: string; exists: boolean };
  native: NativeStatus;
  /** Absolute checkout path accepted by the private dev kernel bridge. */
  workspaceRoot: string;
  /** Stable identity of the repository the page, listings, and rule context are scoped to. */
  repositoryId: string;
}

export interface RerailDaemonOptions {
  version: string;
  /** Defaults to `$RERAIL_HOME/rerail.sqlite`; embedding and tests pass their own. */
  databasePath?: string;
  /** Checkout whose repository the page and API are scoped to; defaults to the current directory. */
  workspaceRoot?: string;
  /** Stable identity override for embedding and tests. */
  repositoryContext?: RepositoryContext;
  host?: string;
  /** Hostnames besides loopback that requests may be addressed to; anything else is answered 421. */
  publicHosts?: readonly string[];
  /** `0` asks the OS for a free port. */
  port?: number;
  /** Directory holding the page. Omit to serve `/api` and `/healthz` alone. */
  uiRoot?: string;
  /** `uiRoot` is the committed `ui/` source rather than the packaged copy; reported as `ui: "dev"`. */
  dev?: boolean;
  /** Written once the server is listening and removed on close. */
  stateFile?: string;
  nativeHealth?: () => NativeHealth;
  /** Local IPC endpoint for the resident kernel; mainly useful for isolated embedding and tests. */
  kernelSocketPath?: string;
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

/** Private loopback bridges: browsers always send Origin, and Rerail processes always send JSON. */
function rejectBridgeRequest(context: Context, bridge: "kernel" | "native"): Response | undefined {
  if (context.req.header("origin") !== undefined) return context.json({ error: `${bridge}-origin-forbidden` }, 403);
  if (context.req.header("content-type") !== "application/json") return context.json({ error: `${bridge}-content-type-required` }, 415);
  return undefined;
}

/**
 * The daemon is the one process that writes rules, work links, and presentation settings; harness
 * extensions only read. Every store opens the one Rerail database; listings are scoped to the
 * daemon's repository. Writable handles open on first use so a daemon with no sessions yet
 * does not create the database until asked to.
 */
class DaemonStores implements RerailApiStores {
  readonly reader: SqliteTranscriptReader;
  readonly executions: SqlitePolicyExecutionReader;
  readonly #path: string;
  readonly #repository: RepositoryContext;
  #rules: ScopedRuleCatalog | undefined;
  #transcripts: SqliteTranscriptStore | undefined;
  #preferences: SqlitePresentationPreferenceStore | undefined;
  #scanControls: SqliteScanControlStore | undefined;
  #database: RerailDatabase | undefined;
  #startStates: ((workspace: string) => RepositoryStartState) | undefined;

  get repositoryId(): string {
    return this.#repository.repositoryId;
  }

  constructor(path: string, repository: RepositoryContext, readonly health: () => DaemonHealth) {
    this.#path = path;
    this.#repository = repository;
    // The read-only views never upgrade, so bring an existing database to the current schema first.
    if (existsSync(path)) openRerailDatabase(path).$client.close();
    this.reader = new SqliteTranscriptReader(path, { repositoryId: repository.repositoryId });
    this.executions = new SqlitePolicyExecutionReader(path, { repositoryId: repository.repositoryId });
  }

  rules(): ScopedRuleCatalog {
    this.#rules ??= ScopedRuleCatalog.open(this.#path, this.#repository);
    return this.#rules;
  }

  transcripts(): SqliteTranscriptStore {
    this.#transcripts ??= new SqliteTranscriptStore(this.#path, { repositoryId: this.#repository.repositoryId });
    return this.#transcripts;
  }

  preferences(): SqlitePresentationPreferenceStore {
    this.#preferences ??= new SqlitePresentationPreferenceStore(this.#path);
    return this.#preferences;
  }

  scanControls(): SqliteScanControlStore {
    this.#scanControls ??= new SqliteScanControlStore(this.#path);
    return this.#scanControls;
  }

  database(): RerailDatabase {
    this.#database ??= openRerailDatabase(this.#path);
    return this.#database;
  }

  repositoryState(workspace: string): RepositoryStartState {
    // A daemon with no sessions yet must not create the database to answer that nothing is started.
    if (!existsSync(this.#path)) return effectiveStartState(workspace, this.#path);
    this.#startStates ??= startStateReader(this.database());
    return this.#startStates(workspace);
  }

  close(): void {
    this.reader.close();
    this.#rules?.close();
    this.#transcripts?.close();
    this.#scanControls?.close();
    this.executions.close();
    this.#preferences?.close();
    this.#database?.$client.close();
  }
}

/** The directory `dist/ui` beside this module once built; the UI is not present when running from source. */
export function bundledUiRoot(): string {
  return join(dirname(fileURLToPath(import.meta.url)), "ui");
}

/** The committed page itself, one level up from `dist/` (or `src/`): editing a file and reloading is the whole dev loop. */
export function sourceUiRoot(): string {
  return join(dirname(fileURLToPath(import.meta.url)), "..", "ui");
}

/** A short, database-derived local IPC endpoint isolates each Rerail home's resident kernel. */
export function defaultKernelSocketPath(
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
  readonly #kernel: KernelService;
  readonly #stores: DaemonStores;
  readonly #ui: DaemonState["ui"];
  readonly #native: NativeStatus;
  readonly #options: RerailDaemonOptions;
  readonly #databasePath: string;
  readonly #workspaceRoot: string;
  readonly kernelSocketPath: string;
  readonly #nativeServer: DevNativeServer;
  readonly #nativeToken: string;
  #closed = false;

  private constructor({
    server, kernel, kernelSocketPath, stores, options, databasePath, native, ui, workspaceRoot, nativeServer, nativeToken,
  }: {
    server: Server;
    kernel: KernelService;
    kernelSocketPath: string;
    stores: DaemonStores;
    options: RerailDaemonOptions;
    databasePath: string;
    native: NativeStatus;
    ui: DaemonState["ui"];
    workspaceRoot: string;
    nativeServer: DevNativeServer;
    nativeToken: string;
  }) {
    const address = server.address() as AddressInfo;
    this.host = address.address;
    this.port = address.port;
    this.url = `http://${this.host}:${this.port}/`;
    this.startedAt = Date.now();
    this.#server = server;
    this.#kernel = kernel;
    this.kernelSocketPath = kernelSocketPath;
    this.#stores = stores;
    this.#options = options;
    this.#databasePath = databasePath;
    this.#native = native;
    this.#ui = ui;
    this.#workspaceRoot = workspaceRoot;
    this.#nativeServer = nativeServer;
    this.#nativeToken = nativeToken;
  }

  static async start(options: RerailDaemonOptions): Promise<RerailDaemon> {
    const bindHost = options.host ?? DEFAULT_DAEMON_HOST;
    // A daemon bound off loopback (`--allow-remote`) is dialled by its own literal address, e.g. the
    // container's `http://0.0.0.0:4737/` state URL. An IP literal cannot arrive by DNS rebinding —
    // that attack's `Host` is a DNS name — so the bound address is as safe to answer as loopback.
    const publicHosts = publicHostSet([
      ...(options.publicHosts ?? []),
      ...(LOOPBACK_BIND_HOSTS.includes(bindHost.toLowerCase()) ? [] : [bindHost.includes(":") ? `[${bindHost}]` : bindHost]),
    ]);
    const uiRoot = options.uiRoot === undefined ? undefined : realpathSync(options.uiRoot);
    const ui: DaemonState["ui"] = uiRoot === undefined || options.dev === true ? "dev" : "bundled";
    const native = readNativeStatus(options.nativeHealth ?? loadNativeHealth);
    const databasePath = resolve(options.databasePath ?? defaultRerailDatabasePath());
    const kernelSocketPath = options.kernelSocketPath ?? defaultKernelSocketPath(databasePath);
    const workspaceRoot = resolve(options.workspaceRoot ?? process.cwd());
    const storageRoot = join(workspaceRoot, ".rerail", "sessions");
    const repository = options.repositoryContext ?? resolveRepositoryContext(workspaceRoot);
    const modelPath = resolve(dirname(fileURLToPath(import.meta.url)), "..", "model");
    const kernel = await startKernelService({
      socketPath: kernelSocketPath,
      storageRoot,
      modelPath,
      databasePath,
      workspaceRoot,
      repositoryContext: repository,
    });
    const nativeToken = randomBytes(32).toString("hex");
    const nativeServer = new DevNativeServer({ workspaceRoot, modelPath, databasePath });
    let daemon: RerailDaemon | undefined;
    const stores = new DaemonStores(databasePath, repository, () => daemon!.health());
    const mcp = createRerailMcpHandler(stores, operations, { version: options.version, publicHosts });
    const app = new Hono();
    app.use("*", hostAllowlist(publicHosts));
    app.get("/healthz", context => context.json(daemon!.health()));
    app.post("/kernel/events", async (context) => {
      const rejected = rejectBridgeRequest(context, "kernel");
      if (rejected !== undefined) return rejected;
      let request: KernelRequest;
      try {
        request = parseKernelRequest(await context.req.json());
      } catch {
        return context.json({ error: "kernel-request-invalid" }, 400);
      }
      const { event, configuration } = request;
      const sessionCwd = event.kind === "session-start" ? event.session.cwd : event.session?.cwd;
      if (
        configuration?.workspaceRoot === undefined
        || resolve(configuration.workspaceRoot) !== workspaceRoot
        || (sessionCwd !== undefined && !withinCheckout(workspaceRoot, sessionCwd))
      ) {
        return context.json({ error: "kernel-workspace-mismatch" }, 403);
      }
      const deliveries = await kernel.ingest(event, { storageRoot, modelPath, databasePath, workspaceRoot });
      return context.json({ eventId: event.eventId, deliveries });
    });
    app.post("/kernel/steering-receipts", async (context) => {
      const rejected = rejectBridgeRequest(context, "kernel");
      if (rejected !== undefined) return rejected;
      const receipt = KernelReceiptSchema.safeParse(await context.req.json().catch(() => undefined));
      if (!receipt.success) return context.json({ error: "kernel-request-invalid" }, 400);
      if (!withinCheckout(workspaceRoot, receipt.data.session.cwd)) return context.json({ error: "kernel-workspace-mismatch" }, 403);
      stores.transcripts().recordSteeringDelivery(receipt.data.session, receipt.data.receiptId, receipt.data.ruleIds);
      return context.json({ receiptId: receipt.data.receiptId });
    });
    app.post("/native/session", async (context) => {
      const rejected = rejectBridgeRequest(context, "native");
      if (rejected !== undefined) return rejected;
      const authorization = context.req.header("authorization");
      if (authorization !== undefined && authorization !== `Bearer ${nativeToken}`) {
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
        const trusted = authorization !== undefined;
        return context.json(await nativeServer.handle(body, { trusted }));
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
    const server = createServer((request, response) => void listener(request, response));
    try {
      await listen(server, options.port ?? DEFAULT_DAEMON_PORT, bindHost);
      daemon = new RerailDaemon({
        server, kernel, kernelSocketPath, stores, options, databasePath, native, ui, workspaceRoot, nativeServer, nativeToken,
      });
      if (options.stateFile !== undefined) {
        const temporaryStateFile = `${options.stateFile}.${process.pid}.tmp`;
        try {
          await writeFile(temporaryStateFile, `${JSON.stringify(daemon.state(), null, 2)}\n`, { mode: 0o600 });
          await rename(temporaryStateFile, options.stateFile);
        } finally {
          await rm(temporaryStateFile, { force: true });
        }
      }
      return daemon;
    } catch (error) {
      try {
        if (server.listening) await closeHttpServer(server);
      } finally {
        try {
          await nativeServer.close();
          await kernel.close();
        } finally {
          stores.close();
        }
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
      packageRoot: resolve(dirname(fileURLToPath(import.meta.url)), ".."),
      workspaceRoot: this.#workspaceRoot,
      nativeToken: this.#nativeToken,
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
      repositoryId: this.#stores.repositoryId,
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
        await this.#kernel.close();
      } finally {
        this.#stores.close();
        if (this.#options.stateFile !== undefined) await rm(this.#options.stateFile, { force: true });
      }
    }
  }
}

/**
 * Page files with an SPA fallback; paths that escape the UI root are 404. Every URL the page
 * uses is unversioned (`/app.js`, `/render/*.js`), so nothing may be cached past a revalidation:
 * a browser that kept `app.js` for a year would run the previous release's page, or in `--dev`
 * miss every edit. The validator is a stat, so the common answer is a bodyless 304.
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
  port: number;
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
    port: port === undefined ? DEFAULT_DAEMON_PORT : parsePort(port),
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
  const manifestPath = join(dirname(fileURLToPath(import.meta.url)), "..", "package.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8")) as { version?: unknown };
  return typeof manifest.version === "string" ? manifest.version : "0.0.0";
}

export async function runRerailDaemon(argv: readonly string[]): Promise<void> {
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
  const catalog = openScopedRuleCatalog(workspaceRoot, databasePath);
  const repository = catalog.context();
  // The daemon is shared by every repository; it seeds bundled rules only where Rerail is started.
  const bootstrap = effectiveRepositoryStartState(databasePath, repository) === "started"
    ? catalog.seed(loadBundledRules())
    : undefined;
  catalog.close();
  const daemon = await RerailDaemon.start({
    version: await packageVersion(),
    databasePath,
    workspaceRoot,
    repositoryContext: repository,
    host: arguments_.host,
    publicHosts: arguments_.publicHosts,
    port: arguments_.port,
    uiRoot,
    dev,
    ...(arguments_.stateFile === undefined ? {} : { stateFile: arguments_.stateFile }),
  });
  const health = daemon.health();
  process.stdout.write(`rerail daemon ${health.version} listening at ${daemon.url} (pid ${process.pid})\n`);
  process.stdout.write(`database: ${health.database.path}\n`);
  if (bootstrap !== undefined) process.stdout.write(`rules: ${bootstrap.inserted} seeded, ${bootstrap.refreshed} refreshed, ${bootstrap.kept} kept\n`);
  process.stdout.write(
    health.native.ok
      ? `native kernel: ok (version ${health.native.kernelVersion})\n`
      : `native kernel: unavailable: ${health.native.error}\n`,
  );
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

const launchedPath = process.argv[1];
if (
  launchedPath !== undefined &&
  realpathSync(fileURLToPath(import.meta.url)) === realpathSync(resolve(launchedPath))
) {
  try {
    await runRerailDaemon(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`rerail daemon: ${(error as Error).message}\n`);
    process.exit(1);
  }
}
