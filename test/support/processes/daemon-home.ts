import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { createServer, type AddressInfo } from "node:net";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { DAEMON_STATE_FILE, readDaemonState } from "../../../src/runtime/index.js";

/** The checkout under test, whose `dist/` holds the daemon and the CLI that starts it. */
export const packageRoot = resolve(import.meta.dirname, "..", "..", "..");

/** Waits until `holds` is true, polling, for at most `ms`. */
export async function until(holds: () => boolean | Promise<boolean>, ms = 10_000): Promise<void> {
  const deadline = Date.now() + ms;
  while (!(await holds())) {
    if (Date.now() > deadline) throw new Error(`still not true after ${ms} ms`);
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

const DAEMONS_FILE = "test-daemons.txt";
const LAUNCHERS_FILE = "test-launchers.txt";
const OLDER_DAEMON_FILE = "older-daemon.mjs";

/**
 * The environment of the Rerail processes a test starts in `home`, through which the test sees every one of them: each
 * daemon (`dist/cli/daemon-entry.js`, or the older build's `olderDaemon`) adds its pid to a file in `home`, and so does
 * each `rerail daemon start`, the launcher an autostart runs. Processes those start inherit it. With `busy`, a daemon's
 * event loop can be held: once the file `busy` appears, holding a number of milliseconds, the daemon removes it, makes
 * `<busy>.held`, and spins that long without returning to its event loop, then removes `<busy>.held`.
 */
export function watchedEnvironment(home: string, busy?: string): NodeJS.ProcessEnv {
  const script = join(home, "watch.cjs");
  writeFileSync(script, `const fs = require("node:fs");
const entry = process.argv[1] ?? "";
const daemon = /[\\\\/](daemon-entry\\.js|older-daemon\\.mjs)$/u.test(entry);
const launcher = /[\\\\/]cli\\.js$/u.test(entry) && process.argv[2] === "daemon" && process.argv[3] === "start";
if (daemon) fs.appendFileSync(${JSON.stringify(join(home, DAEMONS_FILE))}, process.pid + "\\n");
if (launcher) fs.appendFileSync(${JSON.stringify(join(home, LAUNCHERS_FILE))}, process.pid + "\\n");
const busy = ${JSON.stringify(busy ?? null)};
if (daemon && busy !== null) {
  setInterval(() => {
    if (!fs.existsSync(busy)) return;
    const ms = Number(fs.readFileSync(busy, "utf8"));
    fs.rmSync(busy, { force: true });
    fs.writeFileSync(busy + ".held", "");
    const end = Date.now() + ms;
    while (Date.now() < end);
    fs.rmSync(busy + ".held", { force: true });
  }, 20).unref();
}
`);
  const preload = `--require ${JSON.stringify(script.replaceAll("\\", "/"))}`;
  return { ...process.env, RERAIL_HOME: home, NODE_OPTIONS: [process.env.NODE_OPTIONS, preload].filter(Boolean).join(" ") };
}

function recorded(home: string, file: string): number[] {
  const path = join(home, file);
  return existsSync(path) ? readFileSync(path, "utf8").split("\n").filter(Boolean).map(Number) : [];
}

/** Every daemon started in `home` under `watchedEnvironment`, running or not. */
export function startedDaemons(home: string): number[] {
  return recorded(home, DAEMONS_FILE);
}

/** Every `rerail daemon start` launched in `home` under `watchedEnvironment`. */
export function launchers(home: string): number[] {
  return recorded(home, LAUNCHERS_FILE);
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
 * A daemon in `home` as every build before the home's daemon lock ran one: its entry point called `runRerailDaemon`
 * straight away, so it takes no lock and writes daemon.json once. Resolves once daemon.json names it.
 */
export async function olderDaemon(home: string, env: NodeJS.ProcessEnv): Promise<ChildProcess> {
  const script = join(home, OLDER_DAEMON_FILE);
  const daemonModule = pathToFileURL(join(packageRoot, "dist", "daemon", "daemon.js")).href;
  writeFileSync(script, `const { runRerailDaemon } = await import(${JSON.stringify(daemonModule)});
await runRerailDaemon(process.argv.slice(2));
`);
  const child = spawn(process.execPath, [script, "--state-file", join(home, DAEMON_STATE_FILE), "--workspace-root", home, "--dev", "--port", "0"], {
    cwd: home, env, stdio: "ignore", windowsHide: true,
  });
  await until(async () => {
    if (child.exitCode !== null) throw new Error(`the older daemon exited with code ${child.exitCode}`);
    return await recordedPid(home) === child.pid;
  }, 30_000);
  return child;
}
