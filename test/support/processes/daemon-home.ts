import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { createServer, type AddressInfo } from "node:net";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { DAEMON_STATE_FILE, readDaemonState } from "../../../src/runtime/index.js";

/** The checkout under test, whose `dist/` holds the daemon and the CLI that starts it. */
export const packageRoot = resolve(import.meta.dirname, "..", "..", "..");

/** A pid no live process can hold on the platforms Rerail supports. */
export const DEAD_PID = 2 ** 22 - 1;

/** Waits until `holds` is true, polling, for at most `ms`; the error names `what` was waited for. */
export async function until(holds: () => boolean | Promise<boolean>, ms = 10_000, what = "the condition"): Promise<void> {
  const deadline = Date.now() + ms;
  while (!(await holds())) {
    if (Date.now() > deadline) throw new Error(`${what}: still not true after ${ms} ms`);
    await delay(20);
  }
}

/** A loopback port free at the moment of asking. */
export async function freePort(): Promise<number> {
  const server = createServer().listen(0, "127.0.0.1");
  await once(server, "listening");
  const { port } = server.address() as AddressInfo;
  server.close();
  await once(server, "close");
  return port;
}

/** The pids of the daemons, and of the `rerail daemon start` launchers, started in a home, and of those that exited. */
const STARTED = {
  daemons: { started: "test-daemons.txt", exited: "test-daemons.exited.txt" },
  launchers: { started: "test-launchers.txt", exited: "test-launchers.exited.txt" },
} as const;
const OLDER_DAEMON_FILE = "older-daemon.mjs";
/** How long a daemon holds its event loop once asked ({@link holdEventLoop}), as a long check holds a busy daemon's. */
const BUSY_MS = 8_000;

/**
 * The environment of the Rerail processes a test starts in `home`, through which the test sees every one of them: each
 * daemon (`dist/cli/daemon-entry.js`, or the older build's `olderDaemon`) adds its pid to a file in `home`, and so does
 * each `rerail daemon start`, the launcher an autostart runs; each adds it to another as it exits, however it exits but
 * a kill. Processes those start inherit it. With `busy`, a daemon's
 * event loop can be held ({@link holdEventLoop}): once the file `busy` appears, the daemon removes it, makes
 * `<busy>.held`, and spins {@link BUSY_MS} without returning to its event loop, then removes `<busy>.held`. The daemon
 * looks only for the file, never at what it holds: it polls, and could read a file the test has created and not yet
 * written. With `slowStart`, the first daemon to listen while that file is there removes it, makes `<slowStart>.held`,
 * and right after that listen, before it can write daemon.json, spins without returning to its event loop until the
 * test writes `<slowStart>.release` ({@link releaseSlowStart}), then removes the `.held` file; a test writes `slowStart`
 * before any daemon starts.
 */
export function watchedEnvironment(home: string, busy?: string, slowStart?: string): NodeJS.ProcessEnv {
  const script = join(home, "watch.cjs");
  writeFileSync(script, `const fs = require("node:fs");
const entry = process.argv[1] ?? "";
const daemon = /[\\\\/](daemon-entry\\.js|older-daemon\\.mjs)$/u.test(entry);
const launcher = /[\\\\/]cli\\.js$/u.test(entry) && process.argv[2] === "daemon" && process.argv[3] === "start";
const records = ${JSON.stringify(Object.fromEntries(Object.entries(STARTED).map(([kind, files]) => [kind, { started: join(home, files.started), exited: join(home, files.exited) }])))};
const files = daemon ? records.daemons : launcher ? records.launchers : null;
if (files !== null) {
  fs.appendFileSync(files.started, process.pid + "\\n");
  process.on("exit", () => fs.appendFileSync(files.exited, process.pid + "\\n"));
}
const busy = ${JSON.stringify(busy ?? null)};
if (daemon && busy !== null) {
  setInterval(() => {
    if (!fs.existsSync(busy)) return;
    fs.rmSync(busy, { force: true });
    fs.writeFileSync(busy + ".held", "");
    const end = Date.now() + ${BUSY_MS};
    while (Date.now() < end);
    fs.rmSync(busy + ".held", { force: true });
  }, 20).unref();
}
const slowStart = ${JSON.stringify(slowStart ?? null)};
if (daemon && slowStart !== null) {
  const listen = require("node:net").Server.prototype.listen;
  require("node:net").Server.prototype.listen = function (...args) {
    let held = false;
    try {
      fs.rmSync(slowStart);
      fs.writeFileSync(slowStart + ".held", "");
      held = true;
    } catch {}
    if (held) setImmediate(() => {
      const pause = new Int32Array(new SharedArrayBuffer(4));
      while (!fs.existsSync(slowStart + ".release")) Atomics.wait(pause, 0, 0, 10);
      fs.rmSync(slowStart + ".held", { force: true });
    });
    return listen.apply(this, args);
  };
}
`);
  const preload = `--require ${JSON.stringify(script.replaceAll("\\", "/"))}`;
  return { ...process.env, RERAIL_HOME: home, NODE_OPTIONS: [process.env.NODE_OPTIONS, preload].filter(Boolean).join(" ") };
}

/**
 * Holds the event loop of the daemon started with `busy` ({@link watchedEnvironment}) for {@link BUSY_MS}, as a long
 * check holds a busy daemon's; resolves once it is held. Its socket and port stay bound meanwhile.
 */
export async function holdEventLoop(busy: string): Promise<void> {
  await writeFile(busy, "");
  await until(() => existsSync(`${busy}.held`));
}

/** Lets the daemon held at its start by `slowStart` ({@link watchedEnvironment}) go on to write daemon.json. */
export function releaseSlowStart(slowStart: string): void {
  writeFileSync(`${slowStart}.release`, "");
}

function recorded(home: string, file: string): number[] {
  const path = join(home, file);
  return existsSync(path) ? readFileSync(path, "utf8").split("\n").filter(Boolean).map(Number) : [];
}

/** Every `rerail daemon start` launched in `home` under `watchedEnvironment`. */
export function launchers(home: string): number[] {
  return recorded(home, STARTED.launchers.started);
}

/**
 * The daemons, or the `rerail daemon start` launchers, started in `home` under `watchedEnvironment` that are not
 * recorded as exited as often as they were started, for the caller to ask whether each runs. A pid one of them gave up
 * as it exited can belong to another process on the machine by now, or to another of the same kind, which then has a
 * start of its own. A process killed outright records no exit.
 */
export function notExited(home: string, kind: "daemons" | "launchers"): number[] {
  const files = STARTED[kind];
  const exits = new Map<number, number>();
  for (const pid of recorded(home, files.exited)) exits.set(pid, (exits.get(pid) ?? 0) + 1);
  const left = recorded(home, files.started).filter(pid => {
    const pending = exits.get(pid) ?? 0;
    if (pending === 0) return true;
    exits.set(pid, pending - 1);
    return false;
  });
  return [...new Set(left)];
}

/** The pid `daemon.json` in `home` names, if it names one. */
export async function recordedPid(home: string): Promise<number | undefined> {
  return (await readDaemonState(join(home, DAEMON_STATE_FILE)))?.pid;
}

/** What each daemon `daemonProcess` started wrote to stderr, whole once it has closed. */
const errorsOf = new WeakMap<ChildProcess, Promise<string>>();

/** A daemon process of this checkout's build in `home`, started as a start launches one, with `args` after its own. */
export function daemonProcess(home: string, env: NodeJS.ProcessEnv, args: readonly string[] = []): ChildProcess {
  const entry = join(packageRoot, "dist", "cli", "daemon-entry.js");
  const child = spawn(process.execPath, [entry, "--state-file", join(home, DAEMON_STATE_FILE), "--workspace-root", home, "--dev", ...args], {
    cwd: home, env, stdio: ["ignore", "ignore", "pipe"], windowsHide: true,
  });
  let errors = "";
  child.stderr!.setEncoding("utf8").on("data", (chunk: string) => { errors += chunk; });
  errorsOf.set(child, once(child, "close").then(() => errors, () => errors));
  return child;
}

/** What `daemon`, started by {@link daemonProcess}, wrote to stderr, once it has exited: why it stopped, when it did. */
export async function exitMessage(daemon: ChildProcess): Promise<string> {
  return await (errorsOf.get(daemon) ?? Promise.resolve(""));
}

/**
 * A daemon in `home` as every build before the home's daemon lock ran one: its entry point called the daemon straight
 * away (`serveRerailDaemon` now, `runRerailDaemon` before it), so it takes no lock and writes daemon.json once.
 * Resolves once daemon.json names it.
 */
export async function olderDaemon(home: string, env: NodeJS.ProcessEnv): Promise<ChildProcess> {
  const script = join(home, OLDER_DAEMON_FILE);
  const daemonModule = pathToFileURL(join(packageRoot, "dist", "daemon", "daemon.js")).href;
  writeFileSync(script, `const { serveRerailDaemon } = await import(${JSON.stringify(daemonModule)});
await serveRerailDaemon(process.argv.slice(2));
`);
  const child = spawn(process.execPath, [script, "--state-file", join(home, DAEMON_STATE_FILE), "--workspace-root", home, "--dev", "--port", "0"], {
    cwd: home, env, stdio: ["ignore", "ignore", "pipe"], windowsHide: true,
  });
  let errors = "";
  child.stderr!.setEncoding("utf8").on("data", (chunk: string) => { errors += chunk; });
  await until(async () => {
    if (child.exitCode !== null) throw new Error(`the older daemon exited with code ${child.exitCode}: ${errors.slice(-600)}`);
    return await recordedPid(home) === child.pid;
  }, 30_000, "daemon.json naming the older daemon");
  return child;
}
