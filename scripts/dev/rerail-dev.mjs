#!/usr/bin/env node
//
// `just rerail-setup | rerail-dev | rerail-down | rerail-status | rerail-reset | rerail-command`: this checkout's
// development daemon, run on the host from the checkout's own build.
//
// Each checkout gets one dev instance, recorded in `.rerail/dev/instance.json`:
//   { "checkout": <abs>, "home": <RERAIL_HOME>, "port": <n>, "url": "http://127.0.0.1:<n>" }
// The port is reserved once per checkout (stable across restarts, distinct from every other registered
// checkout's); the home is `<checkout>/.rerail/dev/home`, so the dev daemon never shares a database with the
// managed install or another checkout. Every checkout that ran setup is listed in
// `${XDG_STATE_HOME:-~/.local/state}/rerail/dev-checkouts.json`, so machine-wide tools (nuke) can find them.
//
// `up --install-bridge` registers this checkout's build with every coding agent (`rerail install --root`). The
// hooks it writes resolve the default Rerail home (`$RERAIL_HOME`, else `~/.rerail`) in the agent's own
// environment, and OMP and Pi load Rerail in-process, so no registration can point them at a private home.
// A bridged checkout (the default home's `current` resolves to it) therefore runs its dev daemon on the
// default home, where its hooks record; it returns to its private home once the bridge is removed.

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, realpathSync, statSync } from "node:fs";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { createServer } from "node:net";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

const SCRIPT = fileURLToPath(import.meta.url);
const CHECKOUT = resolve(dirname(SCRIPT), "..", "..");

export const DEV_DIRECTORY = join(".rerail", "dev");
export const INSTANCE_FILE = join(DEV_DIRECTORY, "instance.json");
/** Ports dev daemons reserve from; each checkout starts at a slot derived from its path. */
export const PORT_RANGE = { first: 35_000, last: 35_999 };
const DAEMON_STATE_FILE = "daemon.json";
const STOP_TIMEOUT_MS = 10_000;
const HEALTH_TIMEOUT_MS = 30_000;
const LOCK_TIMEOUT_MS = 15_000;

const USAGE = "usage: just rerail-dev [--install-bridge] [--restore [backup-dir]]";

function info(message) {
  process.stdout.write(`==> ${message}\n`);
}

function canonicalPath(path) {
  try {
    return realpathSync(path);
  } catch {
    return resolve(path);
  }
}

/** The default Rerail home the coding agents' hooks resolve: `$RERAIL_HOME`, else `~/.rerail`. */
export function defaultRerailHome(env = process.env) {
  return env.RERAIL_HOME === undefined || env.RERAIL_HOME === "" ? join(homedir(), ".rerail") : resolve(env.RERAIL_HOME);
}

/** Whether `checkout` is the build the coding agents are registered with (`<default home>/current` resolves to it). */
export function isBridgedCheckout(checkout, env = process.env) {
  try {
    return realpathSync(join(defaultRerailHome(env), "current")) === canonicalPath(checkout);
  } catch {
    return false;
  }
}

/** Where `checkout`'s dev daemon keeps its state: its private home, or the default home while it is the bridge. */
export function devHome(checkout, env = process.env) {
  return isBridgedCheckout(checkout, env) ? defaultRerailHome(env) : join(canonicalPath(checkout), DEV_DIRECTORY, "home");
}

/** `${XDG_STATE_HOME:-~/.local/state}/rerail/dev-checkouts.json`. */
export function registryFile(env = process.env) {
  const stateHome = env.XDG_STATE_HOME === undefined || env.XDG_STATE_HOME === ""
    ? join(homedir(), ".local", "state")
    : resolve(env.XDG_STATE_HOME);
  return join(stateHome, "rerail", "dev-checkouts.json");
}

/** `--install-bridge`, and `--restore [dir]` or `--restore=<dir>`; a following argument is the directory unless it is a flag. */
export function parseLaunchArguments(argv) {
  const launch = { installBridge: false };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--install-bridge") {
      launch.installBridge = true;
    } else if (isRestoreFlag(argument) && launch.restore === undefined) {
      const option = readRestoreFlag(argument, argv[index + 1]);
      launch.restore = option.restore;
      index += option.consumed;
    } else {
      throw new Error(`${USAGE} (unexpected: ${argument})`);
    }
  }
  return launch;
}

function isRestoreFlag(argument) {
  return argument === "--restore" || argument.startsWith("--restore=");
}

/** One `--restore` flag, and whether it consumed `next` as its directory. */
function readRestoreFlag(argument, next) {
  if (argument !== "--restore") {
    const backupDir = argument.slice("--restore=".length);
    if (backupDir === "") throw new Error(`${USAGE} (--restore= needs a backup directory)`);
    return { restore: { backupDir }, consumed: 0 };
  }
  if (next !== undefined && !next.startsWith("-")) return { restore: { backupDir: next }, consumed: 1 };
  return { restore: {}, consumed: 0 };
}

/** Validates a parsed instance file: absolute paths, and a loopback URL naming exactly its port. */
export function validateDevInstance(value) {
  const fail = (reason) => {
    throw new Error(`Invalid Rerail dev instance (${reason}); rerun just rerail-setup`);
  };
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail("not an object");
  const { checkout, home, port, url } = value;
  if (typeof checkout !== "string" || !isAbsolute(checkout)) fail("checkout");
  if (typeof home !== "string" || resolve(home) !== home) fail("home");
  if (!Number.isSafeInteger(port) || port < 1024 || port > 65_535) fail("port");
  if (url !== `http://127.0.0.1:${port}`) fail(`url must be http://127.0.0.1:${port}`);
  return { checkout, home, port, url };
}

/** This checkout's persisted dev instance, or undefined before `just rerail-setup`. */
export function readDevInstance(checkout = CHECKOUT) {
  let text;
  try {
    text = readFileSync(join(checkout, INSTANCE_FILE), "utf8");
  } catch (error) {
    if (error?.code === "ENOENT") return undefined;
    throw error;
  }
  return validateDevInstance(JSON.parse(text));
}

async function writeJsonAtomically(path, value) {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const pending = `${path}.${randomUUID()}.tmp`;
  await writeFile(pending, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  await rename(pending, path);
}

async function readRegistry(env) {
  try {
    const value = JSON.parse(await readFile(registryFile(env), "utf8"));
    return Array.isArray(value?.checkouts) ? value.checkouts.filter(entry => typeof entry === "string") : [];
  } catch (error) {
    if (error?.code === "ENOENT") return [];
    throw error;
  }
}

/** Every registered checkout that still has a dev instance; vanished checkouts are skipped. */
export async function listDevCheckouts(env = process.env) {
  return (await readRegistry(env)).filter(checkout => existsSync(join(checkout, INSTANCE_FILE)));
}

/** Serializes port reservation and registry writes across concurrent setups. */
async function withRegistryLock(env, operation) {
  const lock = `${registryFile(env)}.lock`;
  await mkdir(dirname(lock), { recursive: true, mode: 0o700 });
  const deadline = Date.now() + LOCK_TIMEOUT_MS;
  for (;;) {
    try {
      await mkdir(lock);
      break;
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      const owner = Number.parseInt(await readFile(join(lock, "pid"), "utf8").catch(() => ""), 10);
      if (!processAlive(owner)) {
        await rm(lock, { recursive: true, force: true });
        continue;
      }
      if (Date.now() > deadline) throw new Error(`another rerail-dev (pid ${owner}) holds ${lock}`);
      await sleep(50);
    }
  }
  try {
    await writeFile(join(lock, "pid"), String(process.pid));
    return await operation();
  } finally {
    await rm(lock, { recursive: true, force: true });
  }
}

/** The first port at or after `checkout`'s preferred slot that no other checkout holds and `isFree` accepts. */
export async function choosePort(checkout, taken, isFree, range = PORT_RANGE) {
  const size = range.last - range.first + 1;
  const slot = createHash("sha256").update(canonicalPath(checkout)).digest().readUInt32BE(0) % size;
  for (let offset = 0; offset < size; offset += 1) {
    const port = range.first + ((slot + offset) % size);
    if (!taken.has(port) && await isFree(port)) return port;
  }
  throw new Error(`no free loopback port left in ${range.first}-${range.last}`);
}

/** Whether nothing listens on 127.0.0.1:`port`. */
export async function loopbackPortFree(port) {
  return await new Promise((resolveFree) => {
    const server = createServer();
    server.once("error", () => resolveFree(false));
    server.listen({ host: "127.0.0.1", port, exclusive: true }, () => server.close(() => resolveFree(true)));
  });
}

/**
 * Resolves and persists `checkout`'s dev instance: its port is kept once reserved, the home follows the bridge,
 * and the checkout is (re)registered. Starts nothing.
 */
export async function setupDevInstance(checkout = CHECKOUT, { env = process.env, isFree = loopbackPortFree } = {}) {
  const root = canonicalPath(checkout);
  return await withRegistryLock(env, async () => {
    const previous = readDevInstance(root);
    const registered = await readRegistry(env);
    let port = previous?.port;
    if (port === undefined) {
      const taken = new Set();
      for (const other of registered) {
        if (other === root) continue;
        try {
          const instance = readDevInstance(other);
          if (instance !== undefined) taken.add(instance.port);
        } catch {
          // An unreadable instance elsewhere reserves nothing.
        }
      }
      port = await choosePort(root, taken, isFree);
    }
    const instance = { checkout: root, home: devHome(root, env), port, url: `http://127.0.0.1:${port}` };
    await mkdir(instance.home, { recursive: true, mode: 0o700 });
    await writeJsonAtomically(join(root, INSTANCE_FILE), instance);
    const checkouts = [...new Set([...registered.filter(entry => existsSync(join(entry, INSTANCE_FILE))), root])].sort();
    await writeJsonAtomically(registryFile(env), { checkouts });
    return instance;
  });
}

function processAlive(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

/** `<home>/daemon.json` when it names a pid and URL. */
export function readDaemonStateFile(home) {
  try {
    const value = JSON.parse(readFileSync(join(home, DAEMON_STATE_FILE), "utf8"));
    if (Number.isSafeInteger(value?.pid) && typeof value.url === "string") return value;
  } catch {
    // Missing or torn: no daemon recorded.
  }
  return undefined;
}

export async function daemonHealth(url, fetch = globalThis.fetch) {
  try {
    const response = await fetch(new URL("/healthz", url), { signal: AbortSignal.timeout(2_000) });
    if (!response.ok) return undefined;
    const body = await response.json();
    return body?.status === "ok" ? body : undefined;
  } catch {
    return undefined;
  }
}

/**
 * The daemon recorded in `home`, judged against `checkout`: `ours` when it was launched from this checkout's
 * build, `foreign` when another package runs it (only possible on the shared default home).
 */
export async function daemonInHome(home, checkout) {
  const state = readDaemonStateFile(home);
  if (state === undefined || !processAlive(state.pid)) return { status: "stopped" };
  const health = await daemonHealth(state.url);
  const owner = typeof state.packageRoot === "string" && canonicalPath(state.packageRoot) === canonicalPath(checkout)
    ? "ours"
    : "foreign";
  if (health === undefined || health.pid !== state.pid) return { status: "unresponsive", state, owner };
  return { status: "running", state, health, owner };
}

async function terminate(pid) {
  process.kill(pid, "SIGTERM");
  const deadline = Date.now() + STOP_TIMEOUT_MS;
  while (processAlive(pid) && Date.now() < deadline) await sleep(100);
  if (processAlive(pid)) {
    process.kill(pid, "SIGKILL");
    while (processAlive(pid)) await sleep(100);
  }
}

/**
 * Stops `checkout`'s dev daemon (the one recorded in its instance home and launched from its build), keeping all
 * of its state. Returns what it found; a checkout without an instance, or whose daemon is not running, is a no-op.
 */
export async function stopDevCheckout(checkout = CHECKOUT) {
  const root = canonicalPath(checkout);
  const instance = readDevInstance(root);
  if (instance === undefined) return { checkout: root, stopped: false };
  const daemon = await daemonInHome(instance.home, root);
  if (daemon.status === "stopped" || daemon.owner !== "ours") return { checkout: root, home: instance.home, stopped: false };
  await terminate(daemon.state.pid);
  // A clean shutdown removes the state file; a killed daemon leaves it behind.
  await rm(join(instance.home, DAEMON_STATE_FILE), { force: true });
  return { checkout: root, home: instance.home, pid: daemon.state.pid, stopped: true };
}

// Build freshness ------------------------------------------------------------------------------------------------

const NATIVE_INPUTS = ["native/src", "native/grammars", "native/Cargo.toml", "native/build.rs", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"];
const TS_INPUTS = ["src", "package.json", "tsconfig.json", "tsconfig.build.json"];
const TS_OUTPUTS = [
  "dist/cli/cli.js", "dist/cli/daemon-entry.js", "dist/presentation/session-presentation.js", "dist/hosts/omp/default-extension.js", "dist/harness/event-stream.js",
  "dist/hosts/cursor/cursor-hook.js", "dist/hosts/claude/claude-hook.js", "dist/hosts/codex/codex-hook.js",
];
const TS_BUILD_SCRIPTS = ["build:ts", "build:omp", "build:events", "build:cursor", "build:claude", "build:codex"];

/** Newest modification time under `paths` (files or directories), or 0 when none exist. */
export function newestMtime(root, paths) {
  let newest = 0;
  const visit = (path) => {
    let stats;
    try {
      stats = statSync(path);
    } catch {
      return;
    }
    if (!stats.isDirectory()) {
      newest = Math.max(newest, stats.mtimeMs);
      return;
    }
    for (const entry of readdirSync(path)) {
      if (entry === "node_modules" || entry === "target" || entry.startsWith(".")) continue;
      visit(join(path, entry));
    }
  };
  for (const path of paths) visit(join(root, path));
  return newest;
}

/** Oldest modification time among `paths`, or undefined when any is missing. */
function oldestMtime(root, paths) {
  let oldest = Number.POSITIVE_INFINITY;
  for (const path of paths) {
    try {
      oldest = Math.min(oldest, statSync(join(root, path)).mtimeMs);
    } catch {
      return undefined;
    }
  }
  return oldest;
}

function nativeAddon(checkout) {
  const prefix = `rerail_engine.${process.platform}-${process.arch}`;
  try {
    const name = readdirSync(join(checkout, "native")).find(entry => entry.startsWith(prefix) && entry.endsWith(".node"));
    return name === undefined ? undefined : join("native", name);
  } catch {
    return undefined;
  }
}

function readLockPackages(path) {
  try {
    const packages = JSON.parse(readFileSync(path, "utf8"))?.packages;
    return packages !== null && typeof packages === "object" ? packages : undefined;
  } catch {
    return undefined;
  }
}

/**
 * `missing` without an installed tree, `stale` when an installed package's version differs from
 * package-lock.json (or a required one is absent), else `fresh`. Compares content, not mtimes: a checkout
 * switch rewrites the lockfile without changing what it pins.
 */
export function dependencyFreshness(checkout = CHECKOUT) {
  const installed = readLockPackages(join(checkout, "node_modules", ".package-lock.json"));
  if (installed === undefined) return "missing";
  const locked = readLockPackages(join(checkout, "package-lock.json")) ?? {};
  for (const [path, entry] of Object.entries(locked)) {
    if (path === "" || !path.startsWith("node_modules/")) continue;
    const present = installed[path];
    if (present === undefined ? entry?.optional !== true && entry?.peer !== true : present.version !== entry?.version) return "stale";
  }
  return "fresh";
}

/** `fresh`, `stale` or `missing` for the dependencies, the native addon, the TypeScript build and the copied UI. */
export function buildFreshness(checkout = CHECKOUT) {
  const judge = (outputs, inputs) => {
    const built = outputs === undefined ? undefined : oldestMtime(checkout, outputs);
    if (built === undefined) return "missing";
    return newestMtime(checkout, inputs) > built ? "stale" : "fresh";
  };
  const addon = nativeAddon(checkout);
  return {
    dependencies: dependencyFreshness(checkout),
    native: judge(addon === undefined ? undefined : [addon], NATIVE_INPUTS),
    typescript: judge(TS_OUTPUTS, TS_INPUTS),
    ui: judge(["dist/ui/index.html"], ["ui"]),
  };
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: CHECKOUT, env: process.env, stdio: "inherit", windowsHide: true, ...options });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) throw new Error(`${[command, ...args].join(" ")} exited ${result.status ?? result.signal}`);
}

function npm(...args) {
  run(process.platform === "win32" ? "npm.cmd" : "npm", args, { shell: process.platform === "win32" });
}

/**
 * Rebuilds whatever is older than its sources. Dependencies are installed only when there are none: `npm ci`
 * deletes node_modules under every process using it, so a drifted tree is reported for the user to refresh.
 */
function buildStale(checkout) {
  const freshness = buildFreshness(checkout);
  if (freshness.dependencies === "missing") {
    info("Installing dependencies (npm ci)");
    npm("ci");
  } else if (freshness.dependencies === "stale") {
    process.stderr.write("rerail-dev: node_modules differs from package-lock.json; run npm ci to refresh it\n");
  }
  if (freshness.native !== "fresh") {
    info(`Native rules engine is ${freshness.native}; building it`);
    run("/bin/bash", [join(checkout, "scripts", "dev", "rerail-native.sh")]);
  }
  if (freshness.typescript !== "fresh" || freshness.dependencies === "missing") {
    info(`TypeScript build is ${freshness.typescript}; building it`);
    for (const script of TS_BUILD_SCRIPTS) npm("run", "--silent", script);
  }
  if (freshness.ui !== "fresh") {
    info(`UI copy is ${freshness.ui}; copying it`);
    npm("run", "--silent", "build:ui");
  }
}

// Commands -------------------------------------------------------------------------------------------------------

function cliEnvironment(instance) {
  return { ...process.env, RERAIL_HOME: instance.home };
}

function requireInstance() {
  const instance = readDevInstance(CHECKOUT);
  if (instance === undefined) throw new Error("this checkout has no dev instance; run just rerail-setup (or just rerail-dev)");
  return instance;
}

function requireBuild() {
  if (!existsSync(join(CHECKOUT, "dist", "cli", "cli.js"))) throw new Error("this checkout is not built; run just rerail-dev");
}

async function setup() {
  const instance = await setupDevInstance(CHECKOUT);
  info(`Rerail dev instance: ${instance.url}`);
  info(`Home: ${instance.home}${isBridgedCheckout(CHECKOUT) ? " (default home: this checkout is the installed bridge)" : ""}`);
  return 0;
}

async function loadRestore() {
  return await import(pathToFileURL(join(CHECKOUT, "scripts", "dev", "rerail-restore.mjs")).href);
}

/** Frees the reserved port and the target home of this checkout's daemons; anything else in the way is refused. */
async function clearTarget(instance, previousHome) {
  for (const home of new Set([previousHome, instance.home].filter(Boolean))) {
    const daemon = await daemonInHome(home, CHECKOUT);
    if (daemon.status === "stopped") continue;
    // The default home is shared with the installed Rerail; while this checkout is the bridge, its daemon is ours to replace.
    if (daemon.owner !== "ours" && !(home === instance.home && isBridgedCheckout(CHECKOUT))) continue;
    info(`Stopping Rerail daemon pid ${daemon.state.pid} in ${home}`);
    await terminate(daemon.state.pid);
    await rm(join(home, DAEMON_STATE_FILE), { force: true });
  }
  if (!(await loopbackPortFree(instance.port))) {
    // Every daemon of this checkout is stopped by now, so whatever still listens belongs to someone else.
    const health = await daemonHealth(instance.url);
    const occupant = health === undefined ? "an unrecognized process" : `Rerail pid ${health.pid} for ${health.workspaceRoot ?? "another workspace"}`;
    throw new Error(
      `${instance.url}, this checkout's reserved port, is already served by ${occupant}; stop it, `
      + `or delete ${INSTANCE_FILE} and rerun just rerail-setup for another port`,
    );
  }
}

async function up(argv) {
  const launch = parseLaunchArguments(argv);
  const restore = launch.restore === undefined ? undefined : await loadRestore();
  if (restore !== undefined) {
    // Fails before anything is built or stopped when there is no backup to restore.
    const plan = await restore.restoreBackup({ ...launch.restore, home: devHome(CHECKOUT), dryRun: true });
    info(`Will restore the Rerail databases from ${plan.backupDir}`);
  }
  const previousHome = readDevInstance(CHECKOUT)?.home;
  let instance = await setupDevInstance(CHECKOUT);
  buildStale(CHECKOUT);
  if (launch.installBridge) {
    info("Registering this checkout's build with every coding agent");
    const bridgeEnv = { ...process.env, RERAIL_HOME: defaultRerailHome() };
    run(process.execPath, [join(CHECKOUT, "dist", "cli", "cli.js"), "install", "--root", CHECKOUT], { env: bridgeEnv });
    instance = await setupDevInstance(CHECKOUT);
    info(`The coding agents' hooks record into ${instance.home}; this dev daemon now runs there`);
  }
  await clearTarget(instance, previousHome);
  if (restore !== undefined) {
    const result = await restore.restoreBackup({ ...launch.restore, home: instance.home, log: message => info(message) });
    const rules = result.rules === undefined ? "" : ` (${result.rules.count} rules)`;
    info(`Restored the Rerail databases from ${result.backupDir} into ${instance.home}${rules}`);
  }
  info(`Starting this checkout's Rerail dev daemon on ${instance.url}`);
  run(process.execPath, [join(CHECKOUT, "dist", "cli", "cli.js"), "daemon", "start", "--dev", "--port", String(instance.port)], {
    env: cliEnvironment(instance),
  });
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  for (;;) {
    const daemon = await daemonInHome(instance.home, CHECKOUT);
    if (daemon.status === "running" && daemon.owner === "ours" && daemon.state.port === instance.port) break;
    if (Date.now() > deadline) throw new Error(`Rerail did not answer /healthz at ${instance.url}; see ${join(instance.home, "daemon.log")}`);
    await sleep(200);
  }
  info(`Rerail dev daemon healthy at ${instance.url}`);
  return 0;
}

async function down() {
  const result = await stopDevCheckout(CHECKOUT);
  if (result.stopped) info(`Stopped this checkout's Rerail dev daemon (pid ${result.pid}); its state in ${result.home} was kept.`);
  else info("This checkout's Rerail dev daemon was not running.");
  return 0;
}

async function status() {
  const instance = readDevInstance(CHECKOUT);
  const lines = [`Checkout: ${canonicalPath(CHECKOUT)}`];
  if (instance === undefined) {
    lines.push("Instance: not configured; run just rerail-setup");
  } else {
    const daemon = await daemonInHome(instance.home, CHECKOUT);
    let state = "stopped";
    if (daemon.status === "running") state = `running (pid ${daemon.state.pid}${daemon.owner === "ours" ? "" : `, from ${daemon.state.packageRoot}`}) at ${daemon.state.url}`;
    else if (daemon.status === "unresponsive") state = `unresponsive (pid ${daemon.state.pid} alive, ${daemon.state.url} not answering)`;
    lines.push(
      `Home: ${instance.home}`,
      `Port: ${instance.port}`,
      `URL: ${instance.url}`,
      `Bridge: ${isBridgedCheckout(CHECKOUT) ? "installed (the coding agents record into this home)" : "not installed"}`,
      `Daemon: ${state}`,
    );
  }
  const freshness = buildFreshness(CHECKOUT);
  lines.push(`Build: ${Object.entries(freshness).map(([part, value]) => `${part} ${value}`).join(", ")}`);
  process.stdout.write(`${lines.join("\n")}\n`);
  return 0;
}

function cli(instance, args) {
  requireBuild();
  const result = spawnSync(process.execPath, [join(CHECKOUT, "dist", "cli", "cli.js"), ...args], {
    cwd: process.cwd(),
    env: cliEnvironment(instance),
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error !== undefined) throw result.error;
  return result.status ?? 1;
}

function reset() {
  const instance = requireInstance();
  const status = cli(instance, ["reset"]);
  if (status === 0) info(`Rerail session history of this repository (all worktrees) reset in ${instance.home}; rules were kept.`);
  return status;
}

function command(args) {
  if (args.length === 0) throw new Error("usage: just rerail-command <rerail CLI arguments>");
  return cli(requireInstance(), args);
}

async function main(argv) {
  const [subcommand, ...args] = argv;
  const noArguments = () => {
    if (args.length > 0) throw new Error(`rerail-dev ${subcommand} takes no arguments`);
  };
  switch (subcommand) {
    case "setup": noArguments(); return await setup();
    case "up": return await up(args);
    case "down": noArguments(); return await down();
    case "status": noArguments(); return await status();
    case "reset": noArguments(); return reset();
    case "command": return command(args);
    default: throw new Error("usage: rerail-dev.mjs setup | up [--install-bridge] [--restore [dir]] | down | status | reset | command <args...>");
  }
}

const launched = process.argv[1];
if (launched !== undefined && canonicalPath(resolve(launched)) === canonicalPath(SCRIPT)) {
  try {
    process.exitCode = await main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`rerail-dev: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
