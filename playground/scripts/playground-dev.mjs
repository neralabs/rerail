#!/usr/bin/env node
//
// `just rerail-playground`: bring up this checkout's dev daemon through
// `scripts/rerail-dev.mjs up`, then supervise the development rule playground on the host. Its
// OMP workers need the macOS seatbelt, the host's OMP login, and a PTY, so the supervisor only
// runs on an attended macOS host. Harness registration never changes here.

import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { accessSync, constants, existsSync, realpathSync, statSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { readDevInstance } from "../../scripts/rerail-dev.mjs";

const playgroundRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const checkoutRoot = dirname(playgroundRoot);
const MIN_NODE_MAJOR = 24;

function info(message) {
  process.stdout.write(`==> ${message}\n`);
}

/** `--install-bridge` is forwarded to `rerail-dev.mjs up` unchanged; nothing else is accepted. */
export function parseLaunchArguments(argv) {
  const launch = { installBridge: false };
  for (const argument of argv) {
    if (argument === "--install-bridge") launch.installBridge = true;
    else throw new Error(`usage: just rerail-playground [--install-bridge] (unexpected: ${argument})`);
  }
  return launch;
}

/** Why the host supervisor stays off; undefined when it should start. */
export function playgroundSkipReason({ platform, unattended }) {
  if (unattended) return "RERAIL_UNATTENDED=1: the playground supervisor runs only in an attended terminal";
  if (platform !== "darwin") {
    return `playground workers need macOS sandbox-exec (this host is ${platform}); use just rerail-dev`;
  }
  return undefined;
}

function isExecutableFile(path) {
  try {
    accessSync(path, constants.X_OK);
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function installPinnedOmpHost() {
  const installer = join(checkoutRoot, "scripts", "install-omp-host.mjs");
  const result = spawnSync(process.execPath, [installer], {
    cwd: checkoutRoot,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) {
    const failureDetail = (result.stderr || result.stdout || `exit ${result.status}`).trim();
    throw new Error(`checksum-pinned OMP host installation failed: ${failureDetail}`);
  }
  const executable = result.stdout.trim();
  if (!isAbsolute(executable) || !isExecutableFile(executable)) {
    throw new Error(`checksum-pinned OMP installer returned an unusable executable: ${executable || "(empty output)"}`);
  }
  return executable;
}

/**
 * An explicit absolute override supports CI and local host testing. Normal
 * playground startup always uses the release checksum-pinned to the SDK.
 */
export function resolveOmpExecutable(
  environment = process.env,
  { executable = isExecutableFile, installPinned = installPinnedOmpHost } = {},
) {
  const explicit = environment.RERAIL_OMP_EXECUTABLE;
  if (explicit !== undefined && explicit !== "") {
    if (!isAbsolute(explicit)) {
      throw new Error("RERAIL_OMP_EXECUTABLE must be an absolute path to a standalone OMP executable");
    }
    if (!executable(explicit)) {
      throw new Error(`RERAIL_OMP_EXECUTABLE is not executable: ${explicit}`);
    }
    return explicit;
  }
  const pinned = installPinned();
  if (!isAbsolute(pinned) || !executable(pinned)) {
    throw new Error(`checksum-pinned OMP host is not executable: ${pinned}`);
  }
  return pinned;
}

/** The host supervisor reaches this checkout's dev daemon on its port and shares its home. */
export function supervisorEnvironment(base, ompExecutable, instance) {
  return {
    ...base,
    RERAIL_DEV: "1",
    RERAIL_HOME: instance.home,
    RERAIL_PLAYGROUND_DAEMON_PORT: String(instance.port),
    RERAIL_OMP_EXECUTABLE: ompExecutable,
  };
}

/** Run the compiled playground under the Node runtime that launched this lifecycle. */
export function supervisorCommand(nodeExecutable = process.execPath) {
  return {
    executable: nodeExecutable,
    args: [join(playgroundRoot, "dist", "supervisor.js")],
  };
}

/** `up` returns once /healthz answers; the supervisor still needs a daemon that stays up while it prepares. */
export async function waitForDaemon(origin, { fetch = globalThis.fetch, timeoutMs = 120_000, intervalMs = 1_000, sleep } = {}) {
  const pause = sleep ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
  const deadline = Date.now() + timeoutMs;
  let last = "no answer yet";
  for (;;) {
    try {
      const response = await fetch(`${origin}/healthz`, { signal: AbortSignal.timeout(2_000) });
      const text = await response.text();
      let body;
      try {
        body = JSON.parse(text);
      } catch {
        body = undefined;
      }
      if (response.ok && body !== null && typeof body === "object" && body.status === "ok") return body;
      last = `${response.status} ${text.slice(0, 120)}`;
    } catch (error) {
      last = error instanceof Error ? error.message : String(error);
    }
    if (Date.now() >= deadline) {
      throw new Error(`Rerail daemon at ${origin} did not report healthy within ${Math.round(timeoutMs / 1000)}s (last: ${last})`);
    }
    await pause(intervalMs);
  }
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? checkoutRoot,
    env: options.env ?? process.env,
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error !== undefined) throw result.error;
  return result.status ?? 1;
}

function runOrThrow(command, args, options) {
  const status = run(command, args, options);
  if (status !== 0) throw new Error(`${command} ${args.join(" ")} exited with code ${status}`);
}

/**
 * Rebuild exactly once when a native dependency cannot load under this Node
 * ABI, then surface the real loader diagnostic if the rebuilt binary is still
 * unusable.
 */
export function reconcileNativeDependency({ probe, rebuild, name = "native dependency" }) {
  const initial = probe();
  if (initial.ok) return false;
  rebuild();
  const repaired = probe();
  if (!repaired.ok) {
    throw new Error(`${name} is unusable under Node ABI ${process.versions.modules}: ${repaired.error || initial.error}`);
  }
  return true;
}

function probeBetterSqlite(cwd) {
  const result = spawnSync(
    process.execPath,
    ["-e", "const Database=require('better-sqlite3');const db=new Database(':memory:');db.prepare('SELECT 1').get();db.close()"],
    { cwd, encoding: "utf8", windowsHide: true },
  );
  if (result.error !== undefined) return { ok: false, error: result.error.message };
  if (result.status === 0) return { ok: true };
  return { ok: false, error: (result.stderr || result.stdout || `probe exited ${result.status}`).trim() };
}

/** The core dist loads the checkout's better-sqlite3 for the catalog; real-agent.ts loads the playground's copy. Both must match this Node. */
function ensureHostSqliteAbi() {
  for (const root of [checkoutRoot, playgroundRoot]) {
    const rebuilt = reconcileNativeDependency({
      name: `better-sqlite3 in ${root}`,
      probe: () => probeBetterSqlite(root),
      rebuild: () => {
        info(`Rebuilding better-sqlite3 in ${root} for Node ABI ${process.versions.modules}`);
        runOrThrow("npm", ["rebuild", "better-sqlite3"], { cwd: root });
      },
    });
    if (rebuilt) info(`SQLite dependency in ${root} now matches the playground Node runtime`);
  }
}

/** npm records the lockfile it installed from; a newer lockfile means a stale tree. */
function needsInstall(directory) {
  const stamp = join(directory, "node_modules", ".package-lock.json");
  if (!existsSync(stamp)) return true;
  return statSync(join(directory, "package-lock.json")).mtimeMs > statSync(stamp).mtimeMs;
}

/**
 * `rerail-dev.mjs up` has already installed the checkout, built its native engine for this
 * platform, and compiled `dist/`; the playground owns only its own package.
 */
function ensurePlaygroundRuntime() {
  const nodeMajor = Number.parseInt(process.versions.node.split(".")[0] ?? "0", 10);
  if (nodeMajor < MIN_NODE_MAJOR) throw new Error(`Node ${nodeMajor} is too old; the playground needs ${MIN_NODE_MAJOR}+.`);
  if (needsInstall(playgroundRoot)) {
    info("Installing playground dependencies");
    runOrThrow("npm", ["ci"], { cwd: playgroundRoot }); // NOSONAR -- trusted native dependency lifecycle scripts are required
  }
  ensureHostSqliteAbi();
  info("Building the host playground runtime");
  runOrThrow("npm", ["run", "build"], { cwd: playgroundRoot });
}

async function supervisePlayground(ompExecutable, instance) {
  const command = supervisorCommand();
  const child = spawn(command.executable, command.args, {
    cwd: checkoutRoot,
    env: supervisorEnvironment(process.env, ompExecutable, instance),
    stdio: "inherit",
    windowsHide: true,
  });
  // Ctrl-C reaches both processes; stay alive until the supervisor has stopped its workers.
  const forward = (signal) => () => {
    if (child.exitCode === null && child.signalCode === null) child.kill(signal);
  };
  const handlers = ["SIGINT", "SIGTERM", "SIGHUP"].map((signal) => [signal, forward(signal)]);
  for (const [signal, handler] of handlers) process.on(signal, handler);
  let code;
  try {
    [code] = await once(child, "exit");
  } finally {
    for (const [signal, handler] of handlers) process.off(signal, handler);
  }
  return code ?? 1;
}

async function main() {
  const launch = parseLaunchArguments(process.argv.slice(2));
  const skip = playgroundSkipReason({
    platform: process.platform,
    unattended: process.env.RERAIL_UNATTENDED === "1",
  });
  if (skip !== undefined) {
    info(`Playground supervisor not started: ${skip}`);
    return 0;
  }
  const status = run(process.execPath, [
    join(checkoutRoot, "scripts", "rerail-dev.mjs"),
    "up",
    ...(launch.installBridge ? ["--install-bridge"] : []),
  ]);
  if (status !== 0) return status;
  const instance = readDevInstance(checkoutRoot);
  if (!instance) throw new Error("rerail-dev up finished without recording .rerail/dev/instance.json");
  ensurePlaygroundRuntime();
  // After every install: the pinned OMP host lives under the checkout's node_modules/.cache, so
  // resolving it before an `npm ci` would hand the supervisor a deleted path, and every worker
  // would then die as `sandbox-exec: execvp() failed` (exit 71).
  const ompExecutable = resolveOmpExecutable(process.env);
  await waitForDaemon(instance.url);
  info(`Starting the rule playground beside the dev daemon at ${instance.url}`);
  return supervisePlayground(ompExecutable, instance);
}

const launched = process.argv[1];
if (launched !== undefined && realpathSync(resolve(launched)) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    process.exitCode = await main();
  } catch (error) {
    process.stderr.write(`rerail-playground: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
