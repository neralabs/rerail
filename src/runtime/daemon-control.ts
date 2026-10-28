import { spawn } from "node:child_process";
import { closeSync, existsSync, mkdirSync, openSync, readFileSync } from "node:fs";
import { mkdir, readFile, realpath, rename, rm, stat, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { loadNativeBinding } from "../engine/native/native.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../engine/package-root.js";
import { resolveExecutable } from "./npm.js";
import { ensureSqlite, type SqliteRepairResult } from "./sqlite-repair.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";

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
}

/** Header a Rerail process sends the daemon's {@link DaemonState.accessToken} in. */
export const DAEMON_ACCESS_HEADER = "x-rerail-access";
/** The cookie a browser holds that capability in, and the query parameter of the access link that sets it. */
export const DAEMON_ACCESS_COOKIE = "rerail-access";

/** The headers that admit a request to the daemon `state` records: its access capability, when it has one. */
export function daemonAccessHeaders(state: Pick<DaemonState, "accessToken">): Record<string, string> {
  return state.accessToken === undefined ? {} : { [DAEMON_ACCESS_HEADER]: state.accessToken };
}

export type NativeStatus = { ok: true; kernelVersion: string } | { ok: false; error: string };

export interface DaemonHealth extends Pick<DaemonState, "version" | "pid" | "ui"> {
  status: "ok";
  uptimeMs: number;
  database: { path: string; exists: boolean };
  native: NativeStatus;
  /** Absolute checkout path native sessions address. */
  workspaceRoot: string;
  /** When Rerail was installed on this machine (epoch ms); null before the database exists. */
  installedAt: number | null;
}

/**
 * Rerail CLI children require Node; a compiled Bun host's execPath may launch the host itself. There the name
 * is resolved through PATH once, so the SQLite probe, its rebuild and the daemon all run under one binary.
 */
export function nodeExecutable(): string {
  return process.versions.bun === undefined ? process.execPath : resolveExecutable("node") ?? "node";
}

export const DAEMON_STATE_FILE = "daemon.json";
export const DAEMON_LOG_FILE = "daemon.log";
const START_LOCK_DIR = "start.lock";

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
 * - `absent`: no state, or a stale file (pid dead, or a different process
 *   answering on that URL) which has been cleared.
 */
export type DaemonLookup =
  | { status: "running"; state: DaemonState; health: DaemonHealth }
  | { status: "unresponsive"; state: DaemonState }
  | { status: "absent" };

export interface DaemonLaunchOptions {
  dev: boolean;
  host?: string;
  /** Required for a `host` off loopback; see `assertHostBindable`. */
  allowRemote?: boolean;
  /** Extra hostnames the daemon answers to; passed through as `--public-host`. */
  publicHosts?: readonly string[];
  port?: number;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
}

export interface StartDaemonResult {
  /** `false` when a daemon was already running and has been reused. */
  started: boolean;
  daemon: RunningDaemon;
}

export function processAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
  } catch (error) {
    // EPERM means the process exists but belongs to another user; it is still alive.
    if ((error as NodeJS.ErrnoException).code !== "EPERM") return false;
  }
  return !isZombie(pid);
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
    && typeof record.startedAt === "number"
    && typeof record.packageRoot === "string"
    && (record.workspaceRoot === undefined || typeof record.workspaceRoot === "string");
}

export function isUsableEngineSocketPath(
  value: unknown,
  platform: NodeJS.Platform = process.platform,
): value is string {
  if (typeof value !== "string" || value.length <= 1 || value.includes("\0")) return false;
  if (platform === "win32") return /^\\\\\.\\pipe\\[^\\]+$/u.test(value);
  return isAbsolute(value) && (platform !== "darwin" || Buffer.byteLength(value) <= MAX_MACOS_UNIX_SOCKET_PATH_BYTES);
}

function isDaemonState(value: unknown): value is DaemonState {
  return hasDaemonStateFields(value) && isUsableEngineSocketPath(value.kernelSocketPath);
}


export async function readDaemonState(stateFile: string): Promise<DaemonState | undefined> {
  let text: string;
  try {
    text = await readFile(stateFile, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
  try {
    const value: unknown = JSON.parse(text);
    return isDaemonState(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

/**
 * The health the daemon `state` records reports; `fetchImpl` lets a client with its own transport, such as the MCP
 * bridge, probe through it.
 */
export async function probeDaemon(
  state: Pick<DaemonState, "url" | "accessToken">,
  fetchImpl: (endpoint: URL, init: RequestInit) => Promise<Response> = fetch,
): Promise<DaemonHealth | undefined> {
  try {
    const response = await fetchImpl(new URL("/healthz", state.url), {
      headers: daemonAccessHeaders(state), signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    });
    if (!response.ok) return undefined;
    const body = await response.json() as DaemonHealth;
    return body.status === "ok" ? body : undefined;
  } catch {
    return undefined;
  }
}

export async function lookupDaemon(home: string): Promise<DaemonLookup> {
  const stateFile = join(home, DAEMON_STATE_FILE);
  const state = await readDaemonState(stateFile);
  if (state === undefined) return { status: "absent" };
  if (!processAlive(state.pid)) {
    await rm(stateFile, { force: true });
    return { status: "absent" };
  }
  const health = await probeDaemon(state);
  if (health === undefined) return { status: "unresponsive", state };
  if (health.pid !== state.pid) {
    // Something else answers on that URL; the recorded daemon is not the listener.
    await rm(stateFile, { force: true });
    return { status: "absent" };
  }
  return { status: "running", state, health };
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
    await rename(written, leasePath);
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
  await mkdir(control.home, { recursive: true, mode: 0o700 });
  return await withStartLock(control.home, async () => {
    const lookup = await lookupDaemon(control.home);
    if (lookup.status === "running") return { started: false, daemon: { state: lookup.state, health: lookup.health } };
    if (lookup.status === "unresponsive") {
      throw new Error(
        `Rerail daemon pid ${lookup.state.pid} is alive but not answering at ${lookup.state.url}; run rerail daemon stop, then rerail daemon start.`,
      );
    }
    return await spawnDaemon(control, options);
  });
}

/**
 * Starts a daemon from `control.packageRoot`, replacing a healthy daemon from
 * another managed package. Hooks use this before sending engine service configuration:
 * an older daemon cannot safely accept fields introduced by the active hook.
 */
export async function startDaemonForPackage(control: DaemonControl, options: DaemonLaunchOptions): Promise<StartDaemonResult> {
  await mkdir(control.home, { recursive: true, mode: 0o700 });
  return await withStartLock(control.home, async () => {
    const lookup = await lookupDaemon(control.home);
    if (lookup.status === "running") {
      if (await daemonUsesPackageRoot(lookup.state, control.packageRoot)) {
        return { started: false, daemon: { state: lookup.state, health: lookup.health } };
      }
      await stopDaemon(control.home);
      return await spawnDaemon(control, options);
    }
    if (lookup.status === "unresponsive") {
      throw new Error(
        `Rerail daemon pid ${lookup.state.pid} is alive but not answering at ${lookup.state.url}; run rerail daemon stop, then rerail daemon start.`,
      );
    }
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
 * The engine service socket main host clients send events to; only the daemon evaluates them, since it alone opens
 * the machine-wide content store. `replaceOtherPackage` brings up this package's daemon in place of one from
 * another package, which is how a hook keeps an older daemon from receiving fields it predates; a client that
 * outlives an update passes false after its first event so it keeps using the newer daemon instead of displacing
 * it. `RERAIL_AUTOSTART=0` starts nothing and requires a running daemon.
 */
export async function daemonEngineSocket(replaceOtherPackage: boolean): Promise<string> {
  const home = defaultRerailHome();
  const packageRoot = PACKAGE_ROOT;
  const control = { home, packageRoot };
  const options = { dev: false };
  const daemon = process.env.RERAIL_AUTOSTART === "0"
    ? await findRunningDaemon(home)
    : (replaceOtherPackage ? await startDaemonForPackage(control, options) : await startDaemon(control, options)).daemon;
  if (daemon === undefined) throw new Error("RERAIL_AUTOSTART=0 requires a running daemon; run rerail daemon start or set RERAIL_ENGINE_SOCKET");
  if (replaceOtherPackage && !(await daemonUsesPackageRoot(daemon.state, packageRoot))) {
    throw new Error("RERAIL_AUTOSTART=0 requires a daemon from the active Rerail package; run rerail daemon start or set RERAIL_ENGINE_SOCKET");
  }
  return daemon.state.kernelSocketPath;
}

/**
 * The engine service socket for a hook after a session's start, or undefined when no daemon answers. A daemon that is
 * down is not started: each tool call then costs one failed probe instead of an autostart that can outlast the coding
 * agent's hook timeout (design N11). A healthy daemon from another package (an update installed mid-session) is
 * replaced as `daemonEngineSocket(true)` replaces it, so steering carries on with this package's engine.
 */
export async function runningDaemonEngineSocket(): Promise<string | undefined> {
  const daemon = await findRunningDaemon(defaultRerailHome());
  if (daemon === undefined) return undefined;
  return await daemonUsesPackageRoot(daemon.state, PACKAGE_ROOT) ? daemon.state.kernelSocketPath : await daemonEngineSocket(true);
}

interface LaunchedDaemon {
  pid: number | undefined;
  /** Why the launched process is gone, or undefined while it runs. */
  exitReason: () => string | undefined;
}

function daemonArguments(control: DaemonControl, options: DaemonLaunchOptions, stateFile: string): string[] {
  const args = [
    join(control.packageRoot, ENTRY_POINTS.daemon),
    "--state-file", stateFile,
  ];
  if (options.dev) args.push("--dev");
  if (options.host !== undefined) args.push("--host", options.host);
  if (options.allowRemote) args.push("--allow-remote");
  if (options.publicHosts !== undefined && options.publicHosts.length > 0) args.push("--public-host", options.publicHosts.join(","));
  if (options.port !== undefined) args.push("--port", String(options.port));
  return args;
}

function launchDaemonProcess(args: string[], options: DaemonLaunchOptions, env: NodeJS.ProcessEnv, logFile: string): LaunchedDaemon {
  if (process.platform === "win32") {
    // Node's spawn hands the child every inheritable handle in this process, not just its stdio. A Rust launcher
    // such as `just` leaves its own stdout inheritable, so a daemon started below it held the caller's output
    // pipe open and a captured `rerail setup | tee` never finished. The native spawn passes only NUL stdin and the log.
    const definedEnv = Object.fromEntries(Object.entries(env).filter((entry): entry is [string, string] => entry[1] !== undefined));
    const pid = loadNativeBinding().spawnDetached(nodeExecutable(), args, options.cwd ?? null, definedEnv, logFile);
    return { pid, exitReason: () => processAlive(pid) ? undefined : "exited" };
  }
  const log = openSync(logFile, "a", 0o600);
  let child;
  try {
    child = spawn(nodeExecutable(), args, { detached: true, stdio: ["ignore", log, log], cwd: options.cwd, env });
  } finally {
    closeSync(log);
  }
  let exited: number | null | undefined;
  child.once("exit", code => {
    exited = code;
  });
  child.unref();
  return { pid: child.pid, exitReason: () => exited === undefined ? undefined : `exited with code ${exited ?? "signal"}` };
}

async function spawnDaemon(control: DaemonControl, options: DaemonLaunchOptions): Promise<StartDaemonResult> {
  await repairSqlite(control);
  await mkdir(control.home, { recursive: true, mode: 0o700 });
  const stateFile = join(control.home, DAEMON_STATE_FILE);
  const logFile = join(control.home, DAEMON_LOG_FILE);
  const env = { ...(options.env ?? process.env), RERAIL_HOME: control.home };
  // The log is appended to across runs; a failure shows only what this launch wrote.
  const logOffset = await stat(logFile).then(info => info.size, () => 0);
  const { pid, exitReason } = launchDaemonProcess(daemonArguments(control, options, stateFile), options, env, logFile);

  const deadline = Date.now() + START_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const exit = exitReason();
    if (exit !== undefined) {
      const output = await tailFile(logFile, logOffset, 20);
      throw new Error(`Rerail daemon ${exit} before it was ready${output === "" ? "." : `:\n${output}`}`);
    }
    const state = await readDaemonState(stateFile);
    if (state !== undefined && state.pid === pid) {
      const health = await probeDaemon(state);
      if (health?.pid === state.pid) return { started: true, daemon: { state, health } };
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
 * is visible there and through `rerail status`, never in the host.
 *
 * Every session records into the one database in the Rerail home, and the one
 * daemon serves every repository; sessions in any repository reuse it for their
 * engine service and status.
 */
export function autostartDaemon(): void {
  if (autostartAttempted) return;
  autostartAttempted = true;
  const setting = process.env.RERAIL_AUTOSTART;
  if (setting === "0" || setting === "false") return;
  const home = defaultRerailHome();
  const packageRoot = PACKAGE_ROOT;
  try {
    mkdirSync(home, { recursive: true, mode: 0o700 });
    const log = openSync(join(home, DAEMON_LOG_FILE), "a", 0o600);
    try {
      const child = spawn(
        nodeExecutable(),
        [join(packageRoot, ENTRY_POINTS.cli), "daemon", "start"],
        { detached: true, stdio: ["ignore", log, log], env: process.env },
      );
      child.on("error", () => undefined);
      child.unref();
    } finally {
      closeSync(log);
    }
  } catch {
    // Autostart is a convenience; the session must never notice it failing.
  }
}

export type StopDaemonResult = "stopped" | "not-running";

export async function stopDaemon(home: string): Promise<StopDaemonResult> {
  const lookup = await lookupDaemon(home);
  if (lookup.status === "absent") return "not-running";
  const pid = lookup.state.pid;
  process.kill(pid, "SIGTERM");
  const deadline = Date.now() + STOP_TIMEOUT_MS;
  while (processAlive(pid) && Date.now() < deadline) await sleep(POLL_INTERVAL_MS);
  if (processAlive(pid)) {
    process.kill(pid, "SIGKILL");
    const killDeadline = Date.now() + KILL_TIMEOUT_MS;
    while (processAlive(pid) && Date.now() < killDeadline) await sleep(POLL_INTERVAL_MS);
    // The state file stays, so a later stop can find the process again.
    if (processAlive(pid)) {
      throw new Error(`Rerail daemon pid ${pid} is still running ${KILL_TIMEOUT_MS / 1_000}s after SIGKILL; check it with ps -p ${pid}, then run rerail daemon stop again.`);
    }
  }
  // The daemon removes its own state file on a clean shutdown; a killed one leaves it behind.
  await rm(join(home, DAEMON_STATE_FILE), { force: true });
  return "stopped";
}

/**
 * Platform openers; anything not listed is assumed to be a freedesktop environment. Windows goes
 * through `rundll32` rather than `cmd /c start`: `start` parses its command line through cmd, so
 * the `&` in a session URL's query string would split it into separate commands.
 */
const BROWSER_OPENERS: Readonly<Record<string, (url: string) => [string, string[]]>> = {
  darwin: url => ["open", [url]],
  win32: url => ["rundll32", ["url.dll,FileProtocolHandler", url]],
};

/** Hands the URL to the desktop's default browser. Returns once the opener has been spawned. */
export function openInBrowser(url: string): void {
  const [command, args] = (BROWSER_OPENERS[process.platform] ?? (target => ["xdg-open", [target]]))(url);
  const child = spawn(command, args, { detached: true, stdio: "ignore" });
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

/**
 * `url` with the home daemon's access capability attached when it has one and the page is that daemon's: the
 * browser then sets its cookie and drops the capability from the address. Only for a page about to open in the
 * person's browser; a link handed to a coding agent never carries it.
 */
export async function dashboardAccessLink(url: string, home: string): Promise<string> {
  const state = await readDaemonState(join(home, DAEMON_STATE_FILE)).catch(() => undefined);
  if (state?.accessToken === undefined) return url;
  const page = new URL(url);
  if (page.origin !== new URL(state.url).origin) return url;
  page.searchParams.set(DAEMON_ACCESS_COOKIE, state.accessToken);
  return page.href;
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
