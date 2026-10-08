#!/usr/bin/env node
//
// `just rerail-playground`: bring up this checkout's Rerail container through
// `rerail/scripts/rerail-dev.mjs`, then supervise the development rule playground on the
// host. Its OMP workers need the macOS seatbelt, the host's OMP login, and a PTY, none of
// which exist in the Linux container, so the supervisor only ever runs here. Harness
// registration never changes.

import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { accessSync, constants, existsSync, readdirSync, realpathSync, statSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseInstanceEnvironment, workspaceTargetFromInstance } from "../../rerail/scripts/rerail-dev.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const rerailRoot = join(repoRoot, "rerail");
const playgroundRoot = join(repoRoot, "rerail-playground");
const MIN_NODE_MAJOR = 24;

function info(message) {
  process.stdout.write(`==> ${message}\n`);
}

/** `--install-bridge` is forwarded to rerail-dev unchanged; nothing else is accepted. */
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
  if (unattended) return "NERA_UNATTENDED=1: the playground supervisor runs only in an attended terminal";
  if (platform !== "darwin") {
    return `playground workers need macOS sandbox-exec (this host is ${platform})`;
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
  const installer = join(rerailRoot, "scripts", "install-omp-host.mjs");
  const result = spawnSync(process.execPath, [installer], {
    cwd: rerailRoot,
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

/** The host supervisor reaches its workspace daemon through that workspace's public origin. */
export function supervisorEnvironment(base, ompExecutable, target) {
  return {
    ...base,
    RERAIL_DEV: "1",
    RERAIL_PORT: String(target.port),
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

/** rerail-dev returns once /healthz answers, but a recreated container can still be settling; the supervisor needs a daemon that stays up. */
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
    cwd: options.cwd ?? repoRoot,
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

/** Rerail's dist loads its own better-sqlite3 for the catalog; real-agent.ts loads the playground's copy. Both must match this Node. */
function ensureHostSqliteAbi() {
  for (const root of [rerailRoot, playgroundRoot]) {
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

// npm records the lockfile it installed from; a newer lockfile means a stale tree.
function needsInstall(directory) {
  const stamp = join(directory, "node_modules", ".package-lock.json");
  if (!existsSync(stamp)) return true;
  return statSync(join(directory, "package-lock.json")).mtimeMs > statSync(stamp).mtimeMs;
}

function newestModifiedAt(path) {
  const entry = statSync(path);
  if (!entry.isDirectory()) return entry.mtimeMs;
  let newest = entry.mtimeMs;
  for (const child of readdirSync(path)) newest = Math.max(newest, newestModifiedAt(join(path, child)));
  return newest;
}

/** Matches `nativePlatformSuffix` in rerail/src/native.ts. */
export function engineSuffix(platform, architecture) {
  if (platform === "darwin") return `darwin-${architecture}`;
  if (platform === "linux") return `linux-${architecture}-gnu`;
  if (platform === "win32") return `win32-${architecture}-msvc`;
  return `${platform}-${architecture}`;
}

/** Host workers load the checkout's own engine for this platform; the container's Linux build lives in its volume. */
function hostEngineStale() {
  const engine = join(rerailRoot, "native", `omp_cosine_rules.${engineSuffix(process.platform, process.arch)}.node`);
  if (!existsSync(engine)) return true;
  const built = statSync(engine).mtimeMs;
  return [join(rerailRoot, "native", "src"), join(rerailRoot, "native", "Cargo.toml"), join(rerailRoot, "Cargo.lock")]
    .some((input) => newestModifiedAt(input) > built);
}

function ensureHostRuntime() {
  const nodeMajor = Number.parseInt(process.versions.node.split(".")[0] ?? "0", 10);
  if (nodeMajor < MIN_NODE_MAJOR) throw new Error(`Node ${nodeMajor} is too old; the playground needs ${MIN_NODE_MAJOR}+.`);
  // repoRoot too: the client borrows nera-landing's components, tokens, fonts, and React.
  for (const root of [repoRoot, rerailRoot, playgroundRoot]) {
    if (needsInstall(root)) {
      info(`Installing ${root} dependencies for the host playground`);
      runOrThrow("npm", ["ci"], { cwd: root }); // NOSONAR -- trusted native dependency lifecycle scripts are required
    }
  }
  ensureHostSqliteAbi();
  if (hostEngineStale()) {
    info(`Building the ${engineSuffix(process.platform, process.arch)} native kernel for playground workers`);
    runOrThrow("/bin/bash", [join(repoRoot, "scripts", "rerail-native.sh")]);
  }
  info("Building the host playground runtime");
  runOrThrow("npm", ["run", "build:ts"], { cwd: rerailRoot });
  runOrThrow("npm", ["run", "build"], { cwd: playgroundRoot });
}

async function supervisePlayground(ompExecutable, target) {
  const command = supervisorCommand();
  const child = spawn(command.executable, command.args, {
    cwd: repoRoot,
    env: supervisorEnvironment(process.env, ompExecutable, target),
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
    unattended: process.env.NERA_UNATTENDED === "1",
  });
  if (skip !== undefined) {
    info(`Playground supervisor not started: ${skip}`);
    return 0;
  }
  const status = run(process.execPath, [
    join(rerailRoot, "scripts", "rerail-dev.mjs"),
    ...(launch.installBridge ? ["--install-bridge"] : []),
  ]);
  if (status !== 0) return status;
  const instance = parseInstanceEnvironment(await readFile(join(repoRoot, ".nera", "instance.env"), "utf8"));
  const target = workspaceTargetFromInstance(instance);
  // Order matters: `npm ci` in rerail/ wipes node_modules, and the pinned OMP host lives in
  // rerail/node_modules/.cache. Resolving first leaves the supervisor holding a deleted path, and
  // every worker then dies as `sandbox-exec: execvp() failed` (exit 71).
  ensureHostRuntime();
  const ompExecutable = resolveOmpExecutable(process.env);
  await waitForDaemon(target.origin);
  info(`Starting the rule playground beside the container at ${target.origin}`);
  return supervisePlayground(ompExecutable, target);
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
