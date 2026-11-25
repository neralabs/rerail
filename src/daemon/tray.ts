/**
 * The daemon's side of the tray icon (`tray/`, the `rerail-tray` program): whether to start it ({@link trayGate}), the
 * menu it shows ({@link trayMenuState}), and the child process the daemon runs it in ({@link startTray}).
 *
 * The tray is a child of the daemon with three pipes. The daemon writes a menu state, one JSON line, at once and again
 * whenever it changes; the tray writes back one JSON line per click (`open`, `update` or `stop`) and its diagnostics on
 * stderr, which go to daemon.log. The tray exits when its stdin closes, so it never outlives the daemon, however the
 * daemon ends. The tray is an extra: nothing it does or fails to do may stop the daemon, so a failure is logged once,
 * and the tray is never started again in that daemon.
 */
import type { ChildProcess } from "node:child_process";
import { accessSync, chmodSync, constants, existsSync } from "node:fs";
import { homedir, userInfo } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { spawn } from "../engine/child-process.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../engine/package-root.js";
import { NATIVE_ENGINE_SYSTEMS, nativeTrayFileName, resolveNativeTrayPath } from "../engine/native/native.js";
import { isMainRerailBuild } from "../engine/native/rerail-build.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { BROWSER_OFF_NOTE, DAEMON_LOG_FILE, launchDetached, openInBrowser, type DaemonHealth, type DaemonMachine } from "../runtime/daemon-control.js";

/** What {@link trayGate} decides from: this process's environment and system, and what the daemon knows of itself. */
export type TrayGateFacts = {
  env: NodeJS.ProcessEnv;
  platform: NodeJS.Platform;
  /** `process.arch`: whether a missing binary is worth a log line depends on it. */
  architecture: string;
  /** `isMainRerailBuild()`: the machine's managed installation, not a checkout's dev build. */
  mainBuild: boolean;
  /** Whether the daemon binds only a loopback host. */
  loopback: boolean;
  /** `resolveNativeTrayPath()`: the tray program for this system, undefined where this install has none. */
  trayPath: string | undefined;
  /** Linux: whether `$XDG_RUNTIME_DIR/bus`, the default session bus socket, exists. */
  sessionBusExists: boolean;
};

/**
 * Whether the daemon puts up a tray icon. Every gate must hold: a main build or `RERAIL_TRAY=1`, `RERAIL_TRAY` not `0`
 * or `false`, no `CI`, a loopback daemon, no SSH, on Linux a display and a session bus, and the binary. Tests, CI and a
 * daemon reached over SSH never put an icon up. A failed gate is silent, but for a missing binary on a system that
 * should have one, whose `log` line the daemon writes once.
 */
export function trayGate(facts: TrayGateFacts): { start: true; trayPath: string } | { start: false; log?: string } {
  const { platform, architecture, trayPath } = facts;
  if (!trayWanted(facts) || !desktopSession(facts)) return { start: false };
  if (trayPath !== undefined) return { start: true, trayPath };
  // Every engine package ships a tray program (`scripts/release/engine/packages.mjs`): a main build on one of their
  // systems that finds none has lost a file, and says so.
  if (!NATIVE_ENGINE_SYSTEMS.some(([os, cpu]) => os === platform && cpu === architecture)) return { start: false };
  return { start: false, log: `rerail tray: not started: this install has no ${nativeTrayFileName(platform, architecture)}` };
}

/** Whether this daemon should have an icon: a main build or `RERAIL_TRAY=1`, never `RERAIL_TRAY=0`, CI or a remote host. */
function trayWanted({ env, mainBuild, loopback }: TrayGateFacts): boolean {
  if (env.RERAIL_TRAY === "0" || env.RERAIL_TRAY === "false") return false;
  return (mainBuild || env.RERAIL_TRAY === "1") && env.CI === undefined && loopback;
}

/** Whether a desktop is there to show an icon: no SSH, and on Linux a display and a session bus. */
function desktopSession(facts: TrayGateFacts): boolean {
  const { env } = facts;
  if (env.SSH_CONNECTION !== undefined || env.SSH_TTY !== undefined) return false;
  return facts.platform !== "linux" || linuxDesktop(facts);
}

/** Linux: a display, X11 or Wayland, and a session bus for StatusNotifierItem. */
function linuxDesktop({ env, sessionBusExists }: TrayGateFacts): boolean {
  return Boolean(env.DISPLAY || env.WAYLAND_DISPLAY) && Boolean(env.DBUS_SESSION_BUS_ADDRESS || sessionBusExists);
}

/** Where the menu's `rerail update` is: not asked for, under way, or ended, for `version`. */
export type TrayUpdateRun = { state: "idle" } | { state: "running" | "done" | "failed"; version: string };

/** One menu state, the line the tray reads on stdin (`MenuState` in tray/src/protocol.rs); every key is always sent. */
export interface TrayMenuState {
  healthy: boolean;
  status: string;
  version: string;
  dashboard: string;
  home: string | null;
  update: { label: string; enabled: boolean } | null;
  attention: boolean;
}

/**
 * The menu for a daemon whose health is `health`, serving `url`, in the Rerail home `home`. `newer` is the newer Rerail
 * the update check found, or null; `update` is where the menu's update is. The badge asks for attention while the
 * daemon is unhealthy or an update is there to click. The update's refresh replaces this daemon with the new version's,
 * whose icon no longer offers it, so the item says it is done only where this daemon outlives the update: when the
 * refresh keeps it running or could not replace it. The badge then goes back to the plain dot, though
 * `health.update.latest` stays newer until the new version replaces this daemon.
 */
export function trayMenuState(input: {
  health: DaemonHealth;
  url: string;
  home: string;
  defaultHome: string;
  newer: string | null;
  update: TrayUpdateRun;
}): TrayMenuState {
  const { health, newer, update: run } = input;
  const update = updateItem(newer, run);
  return {
    healthy: health.healthy,
    status: health.healthy ? "Rerail is running" : `Rerail needs attention: ${unhealthyBecause(health)}`,
    version: `Version ${health.version}`,
    dashboard: `Dashboard at ${new URL(input.url).host}`,
    home: resolve(input.home) === resolve(input.defaultHome) ? null : `Home: ${input.home}`,
    update,
    attention: !health.healthy || (update !== null && (run.state === "idle" || run.state === "failed")),
  };
}

/** Why an unhealthy daemon cannot serve sessions, as the menu's status line says it. */
function unhealthyBecause(health: DaemonHealth): string {
  if (!health.native.ok) return "native kernel unavailable";
  if (!health.model.ok) return "model unavailable";
  return "unhealthy";
}

function updateItem(newer: string | null, run: TrayUpdateRun): TrayMenuState["update"] {
  switch (run.state) {
    case "idle":
      return newer === null ? null : { label: `Update to ${newer}`, enabled: true };
    case "running":
      return { label: `Updating to ${run.version}…`, enabled: false };
    case "done":
      return { label: `Updated to ${run.version}. New sessions use it.`, enabled: false };
    case "failed":
      return { label: "Update failed, see daemon.log", enabled: true };
  }
}

/** What the tray needs of the running daemon. */
export interface TrayDaemon {
  health(): DaemonHealth;
  /** The URL the daemon serves. */
  url(): string;
  /** The dashboard link with the access token, for this user's browser (`dashboardLink`). */
  dashboardLink(): string;
  /** Whether the daemon binds only a loopback host. */
  loopback: boolean;
  /** Stops the daemon as SIGTERM and `rerail daemon stop` do. */
  stop(): void;
}

export interface TrayHandle {
  /** Stops the menu's timer and closes the tray's stdin, on which the tray exits. */
  close(): void;
}

export interface TrayOptions {
  /** The tray program. */
  trayPath: string;
  /** Arguments for `trayPath`; none for the tray program. A test runs its fake tray as `node fake-tray.mjs`. */
  args?: readonly string[];
  daemon: TrayDaemon;
  /** The daemon's Rerail home, named in the menu when it is not `defaultHome`. */
  home: string;
  defaultHome: string;
  /** Whether the release `latest` is newer than the `running` one. */
  isNewer(latest: string, running: string): boolean;
  openUrl(url: string): void;
  /** Runs `rerail update`; its exit code. */
  runUpdate(): Promise<number>;
  /** Writes one line, with no newline, to daemon.log. */
  log(line: string): void;
  /** How often the daemon's health is read for a changed menu; 5 s by default. */
  intervalMs?: number;
}

const HEALTH_INTERVAL_MS = 5_000;
/** How the tray program starts its own stderr lines; the daemon's log names it its own way. */
const TRAY_PREFIX = "rerail-tray: ";

/**
 * Starts the tray program and keeps its menu current: it writes the first state at once, then reads the daemon's
 * health every `intervalMs` and writes a state only when it changed. That read is in-process and cheap, and sends
 * nothing anywhere. A tray that exits, or cannot start, is logged once and never started again.
 */
export function startTray(options: TrayOptions): TrayHandle {
  const { daemon, log } = options;
  let run: TrayUpdateRun = { state: "idle" };
  let sent: string | undefined;
  /** Set by `close()`, or when the tray has gone: nothing is written to it after. */
  let ended = false;
  let timer: NodeJS.Timeout | undefined;

  const end = (line: string | undefined): void => {
    if (ended) return;
    ended = true;
    clearInterval(timer);
    if (line !== undefined) log(line);
  };

  makeExecutable(options.trayPath);
  let child: ChildProcess;
  try {
    child = spawnTray(options.trayPath, options.args ?? []);
  } catch (error) {
    end(`rerail tray: could not start: ${(error as Error).message}`);
    return { close: () => undefined };
  }
  child.once("error", error => end(`rerail tray: could not start: ${error.message}`));
  const { stdin, stdout, stderr } = child;
  if (stdin === null || stdout === null || stderr === null) {
    // A spawn that ran out of file descriptors (EMFILE, ENFILE) opens no pipes, and says why in an error event on the
    // next tick, which runs before this.
    setImmediate(() => end("rerail tray: could not start: its pipes could not be opened"));
    return { close: () => end(undefined) };
  }
  /** Set once a menu state could not be made, so that is logged once, not at every tick. */
  let menuFailed = false;

  /** The newer Rerail the daemon's update check found, if any. */
  const newer = (health: DaemonHealth): string | null => {
    const latest = health.update?.latest;
    if (latest === undefined) return null;
    try {
      return options.isNewer(latest, health.version) ? latest : null;
    } catch {
      // A version that does not parse is no update to offer.
      return null;
    }
  };
  const menu = (): TrayMenuState => {
    const health = daemon.health();
    return trayMenuState({ health, url: daemon.url(), home: options.home, defaultHome: options.defaultHome, newer: newer(health), update: run });
  };
  const send = (): void => {
    if (ended) return;
    let line: string;
    try {
      line = JSON.stringify(menu());
    } catch (error) {
      // The menu keeps its last state, and the next tick tries again.
      if (!menuFailed) log(`rerail tray: could not read the daemon's state for the menu: ${(error as Error).message}`);
      menuFailed = true;
      return;
    }
    if (line === sent) return;
    sent = line;
    stdin.write(`${line}\n`);
  };

  const update = (): void => {
    // Only what the menu offers: an update to a newer Rerail, or a retry of one that failed. One runs at a time.
    if (run.state === "running" || run.state === "done") return;
    const version = newer(daemon.health()) ?? (run.state === "failed" ? run.version : null);
    if (version === null) return;
    run = { state: "running", version };
    send();
    // Through a promise, so a runUpdate that throws as it is called fails the update like one that rejects.
    void Promise.resolve().then(() => options.runUpdate()).then(
      code => code === 0,
      () => false,
    ).then(succeeded => {
      run = { state: succeeded ? "done" : "failed", version };
      send();
    });
  };

  const actions: Readonly<Record<string, () => void>> = {
    open: () => options.openUrl(daemon.dashboardLink()),
    update,
    stop: () => daemon.stop(),
  };

  eachLine(stdout, line => {
    const action = actionOf(line);
    if (action === undefined || !Object.hasOwn(actions, action)) return;
    try {
      actions[action]!();
    } catch (error) {
      log(`rerail tray: could not ${action}: ${(error as Error).message}`);
    }
  });
  eachLine(stderr, line => log(`rerail tray: ${line.startsWith(TRAY_PREFIX) ? line.slice(TRAY_PREFIX.length) : line}`));
  // A write after the tray has gone fails with EPIPE here; its exit says why it went.
  stdin.on("error", () => undefined);
  // On close, not exit: the tray's last stderr lines, which say why it went, are logged first.
  child.once("close", (code, signal) => end(`rerail tray: exited (${code ?? signal})`));

  send();
  timer = setInterval(send, options.intervalMs ?? HEALTH_INTERVAL_MS);
  timer.unref();

  return {
    close: () => {
      end(undefined);
      stdin.end();
    },
  };
}

function spawnTray(trayPath: string, args: readonly string[]) {
  return spawn(trayPath, [...args], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
}

/**
 * Gives the tray program back its executable bit on macOS and Linux, where an npm install may have dropped it. A
 * failure is left to the spawn, which logs why it could not start.
 */
function makeExecutable(trayPath: string, platform: NodeJS.Platform = process.platform): void {
  if (platform === "win32") return;
  try {
    accessSync(trayPath, constants.X_OK);
  } catch {
    try {
      chmodSync(trayPath, 0o755);
    } catch {
      // The spawn says why the program cannot run.
    }
  }
}

/** The action a line from the tray names, `{"action":"open"}` naming `open`; undefined for a line that names none. */
function actionOf(line: string): string | undefined {
  let message: unknown;
  try {
    message = JSON.parse(line);
  } catch {
    return undefined;
  }
  if (typeof message !== "object" || message === null) return undefined;
  const { action } = message as { action?: unknown };
  return typeof action === "string" ? action : undefined;
}

/** Calls `take` with each line `stream` carries, without its newline, however the lines are split across reads. */
function eachLine(stream: NodeJS.ReadableStream, take: (line: string) => void): void {
  let pending = "";
  stream.setEncoding("utf8");
  // A pipe that fails to read ends with the tray, whose exit is logged; unheard, its error would end the daemon.
  stream.on("error", () => undefined);
  stream.on("data", (chunk: string) => {
    pending += chunk;
    let newline = pending.indexOf("\n");
    while (newline !== -1) {
      const line = pending.slice(0, newline).replace(/\r$/u, "");
      pending = pending.slice(newline + 1);
      if (line !== "") take(line);
      newline = pending.indexOf("\n");
    }
  });
}

/** How often a running `rerail update` is asked whether it has exited. */
const UPDATE_POLL_MS = 1_000;

/**
 * Starts the tray for the running `daemon` when every gate holds ({@link trayGate}); undefined when one does not. The
 * gates read this process's environment and system. `isNewer` compares versions, which this layer cannot import.
 */
export function startDaemonTray(daemon: TrayDaemon, deps: { isNewer(latest: string, running: string): boolean }): TrayHandle | undefined {
  const env = process.env;
  const gate = trayGate({
    env,
    platform: process.platform,
    architecture: process.arch,
    mainBuild: isMainRerailBuild(),
    loopback: daemon.loopback,
    trayPath: resolveNativeTrayPath(),
    sessionBusExists: process.platform === "linux" && !env.DBUS_SESSION_BUS_ADDRESS && env.XDG_RUNTIME_DIR !== undefined
      && existsSync(join(env.XDG_RUNTIME_DIR, "bus")),
  });
  const log = (line: string): void => void process.stderr.write(`${line}\n`);
  if (!gate.start) {
    if (gate.log !== undefined) log(gate.log);
    return undefined;
  }
  const home = defaultRerailHome(env);
  return startTray({
    trayPath: gate.trayPath,
    daemon,
    home,
    defaultHome: accountRerailHome(),
    isNewer: deps.isNewer,
    // The link carries the access token, so the line does not name it.
    openUrl: url => {
      if (!openInBrowser(url)) log(`rerail tray: could not open the dashboard: ${BROWSER_OFF_NOTE}`);
    },
    runUpdate: async () => await runRerailUpdate(home, env),
    log,
  });
}

/**
 * `~/.rerail` in the account's own home, not `HOME`'s: a sandbox that sets `HOME` runs a daemon whose home the menu
 * names, so its icon is told apart from the user's own. An account without a user database entry (a container's
 * arbitrary uid) falls back to `HOME`.
 */
export function accountRerailHome(): string {
  let home: string;
  try {
    home = userInfo().homedir;
  } catch {
    home = homedir();
  }
  return join(home, ".rerail");
}

/**
 * Runs `rerail update` with the active launcher's CLI, detached, with its output appended to daemon.log, as
 * `autostartDaemon` runs `daemon start` (`launchDetached`): on Windows it then opens no console window, and an update
 * that stops this daemon still finishes: its refresh stops it to start the new version's in its place, and an update
 * that rebuilds the SQLite binding stops it first. Its exit code; 1 when it could not start, was ended by a signal, or
 * is gone where no code was kept. `machine` is `launchDetached`'s, this one unless a test's.
 */
export async function runRerailUpdate(home: string, env: NodeJS.ProcessEnv, machine?: DaemonMachine): Promise<number> {
  const args = [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "update"];
  const launched = launchDetached({ args, cwd: home, env, logFile: join(home, DAEMON_LOG_FILE) }, machine);
  for (;;) {
    const code = launched.exitCode();
    if (code !== undefined) return code ?? 1;
    await delay(UPDATE_POLL_MS, undefined, { ref: false });
  }
}
