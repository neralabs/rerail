/**
 * Internal testing: the tray icon, off unless the daemon runs with `RERAIL_EXPERIMENTAL_TRAY_ICON=1`
 * (docs/internal-testing.md).
 *
 * The daemon's side of the tray icon (`tray/`, the `rerail-tray` program): whether to start it ({@link trayGate}), the
 * menu it shows ({@link trayMenuState}), and the child process the daemon runs it in ({@link startTray}).
 *
 * The tray is a child of the daemon with three pipes. The daemon writes a menu state, one JSON line, at once and again
 * whenever it changes; the tray writes back one JSON line per click (`open`, `settings`, `docs`, `repository` with the
 * key of the repository clicked, `repositories`, `update`, `restart` or `stop`) and its diagnostics on stderr, which
 * go to daemon.log. The tray exits when its stdin closes, so it never outlives the daemon, however the daemon ends.
 * The tray is an extra: nothing it does or fails to do may stop the daemon, so a failure is logged once, and the tray
 * is never started again in that daemon.
 */
import type { ChildProcess } from "node:child_process";
import { accessSync, chmodSync, constants, existsSync } from "node:fs";
import { homedir, userInfo } from "node:os";
import { join, posix } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { spawn } from "../engine/child-process.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../engine/package-root.js";
import { NATIVE_ENGINE_SYSTEMS, nativeTrayFileName, resolveNativeTrayPath } from "../engine/native/native.js";
import { isMainRerailBuild } from "../engine/native/rerail-build.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import { experimentEnabled } from "../engine/experimental-features.js";
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
 * Whether the daemon puts up a tray icon. Every gate must hold: the experiment on (`RERAIL_EXPERIMENTAL_TRAY_ICON`),
 * a main build or `RERAIL_TRAY=1`, no `CI`, a loopback daemon, no SSH, on Linux a display and a session bus, and the
 * binary. Tests, CI and a daemon reached over SSH never put an icon up. A failed gate is silent, but for a missing
 * binary on a system that should have one, whose `log` line the daemon writes once.
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

/**
 * Whether this daemon should have an icon: the experiment on, and a main build or a development checkout's daemon
 * asked for one with `RERAIL_TRAY=1`, so the daemons of a developer's worktrees put up none of their own; never under
 * CI or for a remote host.
 */
function trayWanted({ env, mainBuild, loopback }: TrayGateFacts): boolean {
  if (!experimentEnabled("TRAY_ICON", env)) return false;
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
  /**
   * The third line, `Home path: <path>`. Its key is the one the line had while it named the dashboard's address, so a
   * tray and a daemon from either side of that change still read each other's lines; the tray's `MenuState` calls it
   * `home_path` (`#[serde(rename = "dashboard")]`).
   */
  dashboard: string;
  update: { label: string; enabled: boolean } | null;
  attention: boolean;
  /**
   * The Repositories section ({@link repositorySection}); null while the daemon has not read its repositories, and
   * the tray then leaves the section out, as a tray from before the section ignores the key.
   */
  repositories: TrayRepositorySection | null;
}

/** A repository the daemon knows, as the menu's Repositories section names it. */
export interface TrayRepository {
  /** The repository's key, which a click on its row sends back. */
  key: string;
  name: string;
  setUp: boolean;
  /** Whether its rules run (`stateOf`): `started`, `stopped` while it is off, or `not-set-up`. */
  state: RepositoryStartState;
  /** How many rules it has, one per rule id, and how many of them run (`repositoryRuleCounts`). */
  total: number;
  on: number;
}

/** The menu's Repositories section: a row per repository, and a last line under them, or none. */
export interface TrayRepositorySection {
  /** `running` draws the row's circle green, else grey. */
  rows: { key: string; label: string; running: boolean }[];
  /** "3 more…", which opens the dashboard's Settings, where every repository is listed; "No repositories yet", dimmed. */
  more: { label: string; enabled: boolean } | null;
}

/** How many repositories the menu lists before its "N more…" line. */
export const MENU_REPOSITORIES = 8;
/** How long a repository's name runs in the menu before it is cut with "…". */
const NAME_LENGTH = 40;

/**
 * The menu's Repositories section for `repositories`, in the order given (the dashboard's, by name): each row says
 * the repository's name and what its rules do, in the words of the dashboard's repository list ({@link repositoryTag}),
 * with a green circle where its rules run. Past {@link MENU_REPOSITORIES} rows, a last line says how many more there are.
 */
export function repositorySection(repositories: readonly TrayRepository[]): TrayRepositorySection {
  const shown = repositories.slice(0, MENU_REPOSITORIES);
  const rest = repositories.length - shown.length;
  return {
    rows: shown.map(repository => ({
      key: repository.key,
      label: `${cut(repository.name)} · ${repositoryTag(repository)}`,
      running: repository.state === "started" && repository.on > 0,
    })),
    more: repositories.length === 0
      ? { label: "No repositories yet", enabled: false }
      : rest > 0 ? { label: `${count(rest)} more…`, enabled: true } : null,
  };
}

/**
 * What a repository's rules do, as the dashboard's repository list says it (`repoLine` in ui/repo-rules.js): "12 rules
 * on" ("0 rules on" when each is off), "Off · 12 rules" while the repository is off, "No rules added", "Not set up".
 */
export function repositoryTag(repository: TrayRepository): string {
  if (!repository.setUp) return "Not set up";
  if (repository.total === 0) return "No rules added";
  if (repository.state === "stopped") return `Off · ${count(repository.total)} ${plural(repository.total, "rule")}`;
  return `${count(repository.on)} ${plural(repository.on, "rule")} on`;
}

const count = (n: number): string => n.toLocaleString("en-US");
const plural = (n: number, one: string): string => (n === 1 ? one : `${one}s`);
/** `name`, cut to {@link NAME_LENGTH} characters with "…" where it runs longer. */
const cut = (name: string): string => {
  const characters = [...name];
  return characters.length <= NAME_LENGTH ? name : `${characters.slice(0, NAME_LENGTH - 1).join("")}…`;
};

/**
 * The menu for a daemon whose health is `health`, in the Rerail home `homePath` names ({@link displayHomePath}).
 * `newer` is the newer Rerail the update check found, or null; `update` is where the menu's update is. The icon
 * carries its "!" badge only while the daemon is unhealthy or an update is there to click. The update's refresh
 * replaces this daemon with the new version's, whose icon no longer offers it, so the item says it is done only where
 * this daemon outlives the update: when the refresh keeps it running or could not replace it. The icon then drops its
 * badge, though `health.update.latest` stays newer until the new version replaces this daemon; a later release than
 * the one installed is offered again, never an older one, as when a pulled release moves the channel back. `isNewer`
 * says whether release `latest` is newer than `running`.
 */
export function trayMenuState(input: {
  health: DaemonHealth;
  homePath: string;
  newer: string | null;
  update: TrayUpdateRun;
  isNewer(latest: string, running: string): boolean;
  /** The repositories the daemon knows, or null before it read them. */
  repositories: readonly TrayRepository[] | null;
}): TrayMenuState {
  const { health, newer, update: run, isNewer } = input;
  const update = updateItem(newer, run, isNewer);
  return {
    healthy: health.healthy,
    status: health.healthy ? "Rerail is running" : `Rerail needs attention: ${unhealthyBecause(health)}`,
    version: `Version ${health.version}`,
    dashboard: `Home path: ${input.homePath}`,
    update,
    attention: !health.healthy || updateOffered(newer, run, isNewer) !== null,
    repositories: input.repositories === null ? null : repositorySection(input.repositories),
  };
}

/**
 * The version a click on the menu's update line updates to, or null where the line offers none: nothing newer, an
 * update under way, or one done to the newest release found or a later one. A failed update is retried at the newest
 * release found.
 */
function updateOffered(newer: string | null, run: TrayUpdateRun, isNewer: TrayOptions["isNewer"]): string | null {
  switch (run.state) {
    case "idle":
      return newer;
    case "running":
      return null;
    case "done":
      return newer !== null && isNewer(newer, run.version) ? newer : null;
    case "failed":
      return newer ?? run.version;
  }
}

/** Why an unhealthy daemon cannot serve sessions, as the menu's status line says it. */
function unhealthyBecause(health: DaemonHealth): string {
  if (!health.native.ok) return "native kernel unavailable";
  if (!health.model.ok) return "model unavailable";
  return "unhealthy";
}

function updateItem(
  newer: string | null,
  run: TrayUpdateRun,
  isNewer: TrayOptions["isNewer"],
): TrayMenuState["update"] {
  switch (run.state) {
    case "idle":
      return newer === null ? null : { label: `Update to ${newer}`, enabled: true };
    case "running":
      return { label: `Updating to ${run.version}…`, enabled: false };
    case "done": {
      const next = updateOffered(newer, run, isNewer);
      if (next !== null) return { label: `Update to ${next}`, enabled: true };
      return { label: `Updated to ${run.version}. New sessions use it.`, enabled: false };
    }
    case "failed":
      return { label: "Update failed, see daemon.log", enabled: true };
  }
}

/** What the tray needs of the running daemon. */
export interface TrayDaemon {
  health(): DaemonHealth;
  /**
   * Every repository the daemon knows, as the dashboard lists them. Read every few seconds: its rules are counted again
   * only once the repositories or their rules changed, never for a session's records alone.
   */
  repositories(): readonly TrayRepository[];
  /**
   * The dashboard link with the access token, for this user's browser (`dashboardLink`): at the dashboard's `view`
   * (`settings` opens `#settings`), its main page without one; on `repository`, by its key, where one is named.
   */
  dashboardLink(view?: string, repository?: string): string;
  /** Whether the daemon binds only a loopback host. */
  loopback: boolean;
  /** Closes the daemon and starts it again with the same arguments, as at its heap ceiling. Its new tray is its own. */
  restart(): void;
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
  /** The daemon's Rerail home as the menu names it ({@link displayHomePath}). */
  homePath: string;
  /** Whether the release `latest` is newer than the `running` one. */
  isNewer(latest: string, running: string): boolean;
  openUrl(url: string): void;
  /** Runs `rerail update`; how it ended ({@link runRerailUpdate}). */
  runUpdate(): Promise<UpdateExit>;
  /** Writes one line, with no newline, to daemon.log. */
  log(line: string): void;
  /** How often the daemon's health and repositories are read for a changed menu; 5 s by default. */
  intervalMs?: number;
}

const HEALTH_INTERVAL_MS = 5_000;
/** Rerail's documentation site, which the menu's Documentation opens. */
const DOCUMENTATION_URL = "https://neralabs.github.io/rerail/";
/** How the tray program starts its own stderr lines; the daemon's log names it its own way. */
const TRAY_PREFIX = "rerail-tray: ";

/**
 * Starts the tray program and keeps its menu current: it writes the first state at once, then reads the daemon's
 * health and repositories every `intervalMs` and writes a state only when it changed. Both reads are in-process and
 * send nothing anywhere; the repositories' rules are counted again only after a database commit or a pack change
 * (`RerailDaemon.repositories`). A tray that exits, or cannot start, is logged once and never started again.
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
  /**
   * Set while menu states cannot be made, so an outage is logged once, not at every tick; the next state made clears
   * it, and the outage after is logged again.
   */
  let menuFailed = false;

  const isNewer = (latest: string, running: string): boolean => {
    try {
      return options.isNewer(latest, running);
    } catch {
      // A version that does not parse is no update to offer.
      return false;
    }
  };
  /** The newer Rerail the daemon's update check found, if any. */
  const newer = (health: DaemonHealth): string | null => {
    const latest = health.update?.latest;
    return latest !== undefined && isNewer(latest, health.version) ? latest : null;
  };
  /** The repositories last read, kept while a read fails so the section never empties for an outage. */
  let repositories: readonly TrayRepository[] | null = null;
  /** Set while repositories cannot be read, so an outage is logged once, as {@link menuFailed} is. */
  let repositoriesFailed = false;
  /** The keys of the rows the last state listed: a click names one of them, or does nothing. */
  let listed = new Set<string>();
  const readRepositories = (): void => {
    try {
      repositories = daemon.repositories();
      repositoriesFailed = false;
    } catch (error) {
      if (!repositoriesFailed) log(`rerail tray: could not read the repositories for the menu: ${(error as Error).message}`);
      repositoriesFailed = true;
    }
  };
  const menu = (): TrayMenuState => {
    const health = daemon.health();
    readRepositories();
    return trayMenuState({ health, homePath: options.homePath, newer: newer(health), update: run, isNewer, repositories });
  };
  const send = (): void => {
    if (ended) return;
    let state: TrayMenuState;
    let line: string;
    try {
      state = menu();
      line = JSON.stringify(state);
    } catch (error) {
      // The menu keeps its last state, and the next tick tries again.
      if (!menuFailed) log(`rerail tray: could not read the daemon's state for the menu: ${(error as Error).message}`);
      menuFailed = true;
      return;
    }
    menuFailed = false;
    if (line === sent) return;
    sent = line;
    listed = new Set(state.repositories?.rows.map(row => row.key) ?? []);
    stdin.write(`${line}\n`);
  };

  const update = (): void => {
    // Only what the menu offers: an update to a newer Rerail, or a retry of one that failed. One runs at a time.
    const version = updateOffered(newer(daemon.health()), run, isNewer);
    if (version === null) return;
    run = { state: "running", version };
    send();
    // daemon.log, which the failed line names, says why: an update that never started wrote nothing there itself.
    const failed = (why: string): false => {
      log(`rerail tray: the update to ${version} failed: ${why}`);
      return false;
    };
    // One still running at the deadline may yet finish: the menu stopped waiting, and offers it again.
    const settled = (exit: UpdateExit): boolean => {
      if (exit.code === 0) return true;
      if (exit.stillRunning !== true) return failed(exit.reason);
      log(`rerail tray: the update to ${version} was ${exit.reason}; the menu stopped waiting for it`);
      return false;
    };
    // Through a promise, so a runUpdate that throws as it is called fails the update like one that rejects.
    void Promise.resolve().then(() => options.runUpdate()).then(
      settled,
      (error: unknown) => failed(error instanceof Error ? error.message : String(error)),
    ).then(succeeded => {
      run = { state: succeeded ? "done" : "failed", version };
      send();
    });
  };

  /** A click on a repository's row: the dashboard at that repository, when the menu lists it. */
  const openRepository = (message: TrayMessage): void => {
    if (message.repository === undefined || !listed.has(message.repository)) return;
    options.openUrl(daemon.dashboardLink(undefined, message.repository));
  };
  const actions: Readonly<Record<string, (message: TrayMessage) => void>> = {
    open: () => options.openUrl(daemon.dashboardLink()),
    settings: () => options.openUrl(daemon.dashboardLink("settings")),
    repository: openRepository,
    // Settings lists every repository, the menu only the first few.
    repositories: () => options.openUrl(daemon.dashboardLink("settings")),
    docs: () => options.openUrl(DOCUMENTATION_URL),
    update,
    // daemon.log says why the daemon went: its own close says nothing of who asked.
    restart: () => {
      log("rerail tray: restart requested from the menu");
      daemon.restart();
    },
    stop: () => {
      log("rerail tray: stop requested from the menu");
      daemon.stop();
    },
  };

  eachLine(stdout, line => {
    const message = messageOf(line);
    if (message === undefined || !Object.hasOwn(actions, message.action)) return;
    try {
      actions[message.action]!(message);
    } catch (error) {
      log(`rerail tray: could not ${message.action}: ${(error as Error).message}`);
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

/** A click the tray tells of: its action, and for a repository's row the key the row was sent with. */
interface TrayMessage {
  action: string;
  repository?: string;
}

/**
 * The click a line from the tray tells of, `{"action":"open"}` or `{"action":"repository","repository":"<key>"}`;
 * undefined for a line that names no action. A `repository` that is not a string is left out.
 */
function messageOf(line: string): TrayMessage | undefined {
  let message: unknown;
  try {
    message = JSON.parse(line);
  } catch {
    return undefined;
  }
  if (typeof message !== "object" || message === null) return undefined;
  const { action, repository } = message as { action?: unknown; repository?: unknown };
  if (typeof action !== "string") return undefined;
  return typeof repository === "string" ? { action, repository } : { action };
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
 * How long the menu waits for `rerail update` before it counts the update as failed and offers it again. An update
 * downloads the release and its engine package, may rebuild the SQLite binding, and refreshes the daemons: minutes on a
 * slow connection, never half an hour, so only a hung one reaches this.
 */
const UPDATE_DEADLINE_MS = 30 * 60_000;

/** How a `rerail update` the menu ran ended: its exit code, and why it ended, for daemon.log. */
export interface UpdateExit {
  code: number;
  reason: string;
  /** Set where it had not ended at the deadline: the menu stopped waiting for it, and it is left to run. */
  stillRunning?: true;
}

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
    homePath: displayHomePath(home, accountHome(), process.platform),
    isNewer: deps.isNewer,
    openUrl: url => openTrayPage(url, log),
    runUpdate: async () => await runRerailUpdate(home, env),
    log,
  });
}

/**
 * Opens a page the menu names in the desktop's browser, on `machine`, this one unless a test's. Why it could not is
 * logged without the URL: a dashboard link carries the access token, and the log is daemon.log. A spawn error's message
 * names the opener alone; its `spawnargs`, which hold the URL, are never logged.
 */
export function openTrayPage(url: string, log: (line: string) => void, machine?: DaemonMachine): void {
  const failed = (why: string): void => log(`rerail tray: could not open the page: ${why}`);
  if (!openInBrowser(url, machine, { onError: error => failed(error.message) })) failed(BROWSER_OFF_NOTE);
}

/**
 * The Rerail home `home` as the menu names it, so two daemons' icons can be told apart. On macOS and Linux a home in
 * the account's own home folder `userHome` starts with `~` (`~/.rerail`); any other home, and every home on Windows,
 * is named in full.
 */
export function displayHomePath(home: string, userHome: string, platform: NodeJS.Platform): string {
  if (platform === "win32") return home;
  const inside = posix.relative(userHome, home);
  if (inside === "") return "~";
  if (inside === ".." || inside.startsWith("../") || posix.isAbsolute(inside)) return home;
  return `~/${inside}`;
}

/**
 * The account's own home folder, not `HOME`'s: a sandbox that sets `HOME` gets a Rerail home the menu names in full, so
 * its icon is told apart from the user's own. An account without a user database entry (a container's arbitrary uid)
 * falls back to `HOME`.
 */
export function accountHome(): string {
  try {
    return userInfo().homedir;
  } catch {
    return homedir();
  }
}

/**
 * Runs `rerail update` with the active launcher's CLI, detached, with its output appended to daemon.log, as
 * `autostartDaemon` runs `daemon start` (`launchDetached`): on Windows it then opens no console window, and an update
 * that stops this daemon still finishes: its refresh stops it to start the new version's in its place, and an update
 * that rebuilds the SQLite binding stops it first. Its exit code and why it ended; code 1 when it could not start, was
 * ended by a signal, is gone where no code was kept, or still runs at the deadline (30 minutes). That one is not
 * killed, which could leave an install half done; a retry meanwhile fails at once while it holds the update lock,
 * rather than run beside it. `machine` is `launchDetached`'s, and `timing` the deadline's and the poll's; a test passes
 * its own.
 */
export async function runRerailUpdate(
  home: string,
  env: NodeJS.ProcessEnv,
  machine?: DaemonMachine,
  timing: { deadlineMs?: number; pollMs?: number } = {},
): Promise<UpdateExit> {
  const { deadlineMs = UPDATE_DEADLINE_MS, pollMs = UPDATE_POLL_MS } = timing;
  const args = [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "update"];
  const launched = launchDetached({ args, cwd: home, env, logFile: join(home, DAEMON_LOG_FILE) }, machine);
  // A monotonic clock: a change to the wall clock neither ends the wait early nor stretches it.
  const deadline = performance.now() + deadlineMs;
  for (;;) {
    const code = launched.exitCode();
    if (code !== undefined) return { code: code ?? 1, reason: launched.exitReason() ?? "exited" };
    if (performance.now() >= deadline) {
      const waited = deadlineMs >= 60_000 ? `${Math.round(deadlineMs / 60_000)} minutes` : `${deadlineMs} ms`;
      return { code: 1, reason: `still running after ${waited}`, stillRunning: true };
    }
    await delay(pollMs, undefined, { ref: false });
  }
}
