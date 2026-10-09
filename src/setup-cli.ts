import { existsSync } from "node:fs";
import { mkdir, realpath, rename } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface, type Interface } from "node:readline";
import { setTimeout as sleep } from "node:timers/promises";
import { rulesDashboardUrl, type RulesDashboardScope } from "./api/session-contracts.js";
import { codingAgentHome, connectedCodingAgents, registrationRoot as packageRegistrationRoot } from "./coding-agents.js";
import { RERAIL_DATABASE_FILE } from "./database.js";
import { daemonUsesPackageRoot, lookupDaemon, openDashboard, startDaemon, stopDaemon } from "./daemon-control.js";
import { DEFAULT_DAEMON_PORT, parseArguments, parsePort, type DaemonState } from "./daemon.js";
import {
  detectHarnesses,
  registerHarnesses,
  unregisterHarnesses,
  type Harness,
  type RegistrationOptions,
  type RegistrationReport,
} from "./harness-registration.js";
import { checkNodeVersion } from "./node-version.js";
import { checkSetupAnswer, parseCodingAgents, runSetup, runSetupStep, startRerailStep, type DashboardPage, type SetupEnvironment, type SetupRequest } from "./repository-setup.js";
import { workspaceStartState, type RepositorySetupIo } from "./repository-start.js";
import { renderInstalled, shellQuote, type DashboardLink, type SetupLane } from "./setup-screens.js";
import type { SetupStep } from "./setup-step.js";
import type { ManagedUpdater } from "./updater.js";

export interface SetupContext {
  /** Rerail's own state directory (`~/.rerail` unless `RERAIL_HOME`); versions, launcher, daemon live here. */
  home: string;
  /** The package this launcher runs from: an npx cache, a global install, or a checkout. */
  packageRoot: string;
  updater: ManagedUpdater;
}

interface RegistrationArguments {
  hosts: Harness[] | undefined;
  dryRun: boolean;
}

/** `--agents` may repeat; `parseArguments` keeps the last value, so split a comma list instead. */
function registrationArguments(options: ReadonlyMap<string, string>, flags: ReadonlySet<string>): RegistrationArguments {
  const hostList = options.get("--agents");
  return {
    hosts: hostList === undefined ? undefined : parseCodingAgents(hostList),
    dryRun: flags.has("--dry-run"),
  };
}

function registrationOptions(root: string, home: string, parsed: RegistrationArguments): RegistrationOptions {
  return {
    root,
    home,
    workspaceDir: process.cwd(),
    ...(parsed.hosts === undefined ? {} : { harnesses: parsed.hosts }),
    dryRun: parsed.dryRun,
  };
}

function printReport(report: RegistrationReport, dryRun: boolean): void {
  for (const change of report.changes) {
    const verb = dryRun && change.action !== "skipped" && change.action !== "unchanged" ? `would ${change.action.replace(/d$/, "")}` : change.action;
    process.stdout.write(`  ${change.harness.padEnd(11)} ${verb.padEnd(12)} ${change.path}${change.detail === "" ? "" : `  (${change.detail})`}\n`);
  }
  for (const note of report.notes) process.stdout.write(`  note: ${note}\n`);
}

/** Registrations point at `<home>/current` once setup has run, so updates never touch host config again. */
function registrationRoot(context: SetupContext): string {
  return packageRegistrationRoot(context.home, context.packageRoot);
}

/** Steps `install --root` earns, since it writes the launcher and touches coding agent config. */
function nextSteps(home: string): string[] {
  return [
    `  add ${join(home, "bin")} to PATH to run \`rerail\` directly`,
    "  restart Cursor, Claude Code, Codex, OMP, or Pi so they load the new hooks",
  ];
}

/** True when `<home>/current` already resolves to `root`, so hooks written against it name this checkout. */
async function currentPointsAt(context: SetupContext, root: string): Promise<boolean> {
  const current = await realpath(context.updater.currentLink).catch(() => undefined);
  return current !== undefined && current === (await realpath(root).catch(() => root));
}

/**
 * `--root <checkout>`: adopt the checkout the way `setup` adopts a published
 * version, so a source install leaves a `rerail` command behind and its hooks
 * name `<home>/current` rather than the checkout, whose path dies with the
 * workspace. Returns the root registrations should be written against.
 */
async function activateExplicitRoot(context: SetupContext, root: string, remove: boolean, dryRun: boolean): Promise<string> {
  const { updater } = context;
  // Removal has to name whichever path the hooks in place were written with.
  if (remove) return (await currentPointsAt(context, root)) ? updater.currentLink : root;
  if (dryRun) {
    process.stdout.write(`Would point ${updater.currentLink} at ${root} and write the launcher ${updater.launcherPath}.\n`);
    return updater.currentLink;
  }
  const result = await updater.activateRoot(root);
  process.stdout.write(`${result.changed ? "Activated" : "Already active:"} ${result.current.version} at ${result.current.packageRoot}\n`);
  process.stdout.write(`Launcher: ${updater.launcherPath}\n`);
  return updater.currentLink;
}

/** The backup folder: `~/.rerail-clean-YYYYMMDD`, or `-YYYYMMDD-HHMMSS` when today's exists. */
function backupRoot(now = new Date()): string {
  const pad = (value: number): string => String(value).padStart(2, "0");
  const day = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`;
  const daily = join(codingAgentHome(), `.rerail-clean-${day}`);
  return existsSync(daily) ? `${daily}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}` : daily;
}

/** Rules and every repository's history, with their SQLite sidecars. */
const BACKED_UP_DATABASES = ["", "-wal", "-shm"].map(suffix => `${RERAIL_DATABASE_FILE}${suffix}`);
/** How long a database move waits for Windows to release a lock: 40 attempts, 250 ms apart. */
const LOCKED_MOVE_ATTEMPTS = 40;
const LOCKED_MOVE_RETRY_MS = 250;

/**
 * After the coding agents are disconnected, nothing reopens the database: stop the daemon, then
 * move rules and history aside so a later install starts clean and can restore them. The files are
 * listed only after the stop: closing the daemon's last connection checkpoints the WAL and deletes
 * the `-wal` and `-shm` sidecars.
 */
async function backUpRulesAndHistory(home: string, dryRun: boolean): Promise<void> {
  const target = join(backupRoot(), "rerail-home");
  const present = () => BACKED_UP_DATABASES.filter(name => existsSync(join(home, name)));
  if (dryRun) {
    const listed = present();
    const moves = listed.length === 0 ? "" : " and move " + listed.join(", ") + " to " + target;
    process.stdout.write(`Would stop the Rerail daemon${moves}.\n`);
    return;
  }
  process.stdout.write(await stopDaemon(home) === "stopped" ? "Stopped the Rerail daemon.\n" : "The Rerail daemon was not running.\n");
  const listed = present();
  if (listed.length === 0) {
    process.stdout.write("No rules or history to back up.\n");
    return;
  }
  await mkdir(target, { recursive: true, mode: 0o700 });
  for (const name of listed) {
    // Windows keeps a file locked for a moment after the process holding it has exited (and an
    // antivirus scan can hold it too), so a locked move is retried briefly before giving up.
    for (let attempt = 1; ; attempt += 1) {
      try {
        await rename(join(home, name), join(target, name));
        break;
      } catch (error) {
        const code = error instanceof Error && "code" in error ? error.code : undefined;
        // A sidecar SQLite removed between the listing and the move holds nothing the main file lacks.
        if (code === "ENOENT" && /-(wal|shm)$/u.test(name)) break;
        const locked = process.platform === "win32" && (code === "EBUSY" || code === "EPERM" || code === "EACCES");
        if (!locked || attempt === LOCKED_MOVE_ATTEMPTS) throw error;
        await sleep(LOCKED_MOVE_RETRY_MS);
      }
    }
  }
  process.stdout.write(`Backup: ${target} holds your rules and history. Install Rerail again and restore this backup to get them back.\n`);
}

export async function runInstallCommand(args: readonly string[], context: SetupContext): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--dry-run", "--remove"], options: ["--root", "--agents"] },
    argument => `Unknown install option: ${argument}`,
  );
  const parsed = registrationArguments(options, flags);
  const rootArgument = options.get("--root");
  const explicitRoot = rootArgument === undefined ? undefined : resolve(rootArgument);
  const remove = flags.has("--remove");
  const root = explicitRoot === undefined
    ? registrationRoot(context)
    : await activateExplicitRoot(context, explicitRoot, remove, parsed.dryRun);
  process.stdout.write(`${remove ? "Removing Rerail from" : "Connecting Rerail to"} coding agents for ${root}${parsed.dryRun ? " (dry run)" : ""}\n`);
  const report = remove
    ? await unregisterHarnesses(registrationOptions(root, codingAgentHome(), parsed))
    : await registerHarnesses(registrationOptions(root, codingAgentHome(), parsed));
  printReport(report, parsed.dryRun);
  // Removing every coding agent from the managed install is the uninstall. Removing one coding agent, or a
  // checkout registered with `--root`, leaves the shared rules and history in place.
  const uninstall = remove && parsed.hosts === undefined && (explicitRoot === undefined || root === context.updater.currentLink);
  if (uninstall) await backUpRulesAndHistory(context.home, parsed.dryRun);
  if (explicitRoot !== undefined && !remove) {
    process.stdout.write(["", "Next:", ...nextSteps(context.home), ""].join("\n"));
  }
  return 0;
}

/** Where set-up talks: a terminal it can ask, or (no TTY) a coding agent that relays each question. */
export interface SetupIo extends RepositorySetupIo {
  close(): void;
}

export function setupIo(): SetupIo {
  const say = (text: string): void => {
    process.stdout.write(`${text}\n`);
  };
  if (!process.stdout.isTTY) return { lane: "agent", say, close: () => {} };
  // The iterator buffers lines typed (or piped) ahead of the next question; `question()` would drop them.
  let input: Interface | undefined;
  let lines: AsyncIterator<string> | undefined;
  return {
    lane: "terminal",
    say,
    async ask(prompt) {
      process.stdout.write(prompt);
      input ??= createInterface({ input: process.stdin, crlfDelay: Number.POSITIVE_INFINITY });
      lines ??= input[Symbol.asyncIterator]();
      const next = await lines.next();
      return next.done === true ? undefined : next.value;
    },
    close: () => input?.close(),
  };
}

/** Why no browser can open here (E6), or undefined when one can. */
export function noBrowserReason(env: NodeJS.ProcessEnv, platform: NodeJS.Platform): string | undefined {
  if (env.BROWSER === "none") return "BROWSER is set to none";
  if (env.SSH_CONNECTION || env.SSH_CLIENT || env.SSH_TTY) return "this looks like a remote session";
  if (platform === "linux" && !env.DISPLAY && !env.WAYLAND_DISPLAY) return "there is no display here";
  return undefined;
}

/** The dashboard's hash route for each page set-up links to. The review page is WR. */
const DASHBOARD_ROUTES: Record<DashboardPage, string> = { rules: "rules", review: "rule-packs?review=1" };

/**
 * The daemon of the active install; it serves every repository. A running daemon from another package (an older
 * version), or on another port than `port` names, is replaced, on the port it had unless `port` names one. A daemon
 * that does not answer is never killed here; its pid may belong to another process by now, so `startDaemon`'s
 * refusal (with how to stop it) is the answer. `RERAIL_AUTOSTART=0` (the updater's health check) uses only a
 * daemon already running from this install, and starts or replaces none.
 */
async function activeDaemon(context: SetupContext, port: number | undefined): Promise<DaemonState> {
  // The version directory, not the current link: the daemon and its SQLite binding belong to this install.
  const packageRoot = await realpath(registrationRoot(context));
  const lookup = await lookupDaemon(context.home);
  if (lookup.status === "running" && await daemonUsesPackageRoot(lookup.state, packageRoot) && (port === undefined || lookup.state.port === port)) {
    return lookup.state;
  }
  if (process.env.RERAIL_AUTOSTART === "0") throw new Error("RERAIL_AUTOSTART=0 keeps set-up from starting it");
  if (lookup.status === "running") await stopDaemon(context.home);
  const listen = port ?? (lookup.status === "running" ? lookup.state.port : undefined);
  return (await startDaemon({ home: context.home, packageRoot }, { dev: false, ...(listen === undefined ? {} : { port: listen }) })).daemon.state;
}

/** `<origin>/?repository=<key>&checkout=<path>#<page>`: the page, scoped to the repository and checkout set-up ran for. */
function pageUrl(origin: string, scope: RulesDashboardScope, page: DashboardPage): string {
  const url = new URL(rulesDashboardUrl(origin, scope));
  url.hash = DASHBOARD_ROUTES[page];
  return url.href;
}

/** What set-up needs from this machine: the coding agents, their registration, and the dashboard. */
export function setupEnvironment(context: SetupContext, port?: number): SetupEnvironment {
  return {
    agentsFound: () => detectHarnesses(codingAgentHome()),
    async connect(agents, repoRoot) {
      // Claude Code's project status line lives in the repository: only one the person said yes to.
      await registerHarnesses({
        root: registrationRoot(context),
        home: codingAgentHome(),
        harnesses: agents,
        ...(repoRoot === undefined ? {} : { workspaceDir: repoRoot }),
      });
    },
    async connectedAgents() {
      // A dry run plans every write; an agent with nothing to write is connected already.
      return connectedCodingAgents(registrationRoot(context), codingAgentHome());
    },
    async dashboard(scope, page, open): Promise<DashboardLink> {
      let origin: string;
      try {
        origin = (await activeDaemon(context, port)).url;
      } catch (error) {
        // Rerail may be on already; the link works once a session brings the daemon up.
        const reason = `the dashboard did not start: ${error instanceof Error ? error.message : String(error)}`;
        return { url: pageUrl(`http://127.0.0.1:${port ?? DEFAULT_DAEMON_PORT}/`, scope, page), browserOpened: false, noBrowserReason: reason };
      }
      const url = pageUrl(origin, scope, page);
      if (!open) return { url, browserOpened: false, linkOnly: true };
      const reason = noBrowserReason(process.env, process.platform);
      if (reason !== undefined) return { url, browserOpened: false, noBrowserReason: reason };
      await openDashboard(url);
      return { url, browserOpened: true };
    },
  };
}

/** What a set-up run carries into every answer it prints, so the next run acts on the same database, port and scope. */
export interface SetupCarry {
  readonly port?: number;
  readonly databasePath?: string;
  /** `rerail start --repository`: set-up starts every checkout of the repository. */
  readonly repository?: boolean;
}

function carriedArgs(carry: SetupCarry): string[] {
  return [
    ...(carry.port === undefined ? [] : ["--port", String(carry.port)]),
    ...(carry.databasePath === undefined ? [] : ["--database", carry.databasePath]),
    ...(carry.repository === true ? ["--repository"] : []),
  ];
}

/**
 * What the coding agent runs to answer a set-up question: the launcher setup wrote, which stays put
 * between answers (an `npx` copy may not), with what this run carries. `rerail setup` until the launcher exists.
 */
export function setupAnswerCommand(context: SetupContext, carry: SetupCarry = {}): string | undefined {
  const carried = carriedArgs(carry).map(shellQuote);
  if (!existsSync(context.updater.launcherPath)) return carried.length === 0 ? undefined : ["rerail", "setup", ...carried].join(" ");
  return [shellQuote(context.updater.launcherPath), "setup", ...carried].join(" ");
}

/** True when `<home>/current` already is the package running now, so installing again would change nothing. */
async function isActiveInstall(context: SetupContext): Promise<boolean> {
  const [current, running] = await Promise.all([
    realpath(context.updater.currentLink).catch(() => undefined),
    realpath(context.packageRoot).catch(() => context.packageRoot),
  ]);
  return current === running;
}

/** `--json` on `rerail setup` and `rerail start`: one `SetupStep` on stdout instead of text. */
export const JSON_FLAG = "--json";

/** A failure as `--json` reports it, so stdout stays one JSON object. */
export function errorStep(message: string): SetupStep {
  return { step: "error", screen: message, question: null };
}

const messageOf = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/** Where a set-up run happens: the folder it ran in, and the machine it changes (coding agents, daemon, browser). */
export interface SetupPlace {
  readonly cwd: string;
  /** A database configured in place of the Rerail home's, as an extension's start gate reads it. */
  readonly databasePath?: string;
  environment(port?: number): SetupEnvironment;
}

/** This machine, from `cwd`. */
export function machinePlace(context: SetupContext, cwd: string = process.cwd()): SetupPlace {
  return { cwd, environment: port => setupEnvironment(context, port) };
}

/**
 * `rerail setup` (`npx @neralabs/rerail setup`): the six steps of set-up. Install first (T2), which
 * makes this version the managed install and turns nothing on, then ask, recommend, confirm, start
 * in this repository only, and the summary. Without a TTY each run stops at the next question for the
 * coding agent, which runs set-up again with `--answer`, `--repo` and `--agents`. With `--json` the
 * same run prints one `SetupStep` and nothing else, failures included.
 */
export async function runSetupCommand(
  args: readonly string[],
  context: SetupContext,
  io: SetupIo = setupIo(),
  nodeVersion: string = process.versions.node,
): Promise<number> {
  try {
    if (args.includes(JSON_FLAG)) {
      const step = await setupStep(args, context, machinePlace(context), nodeVersion);
      io.say(JSON.stringify(step));
      return step.step === "error" ? 1 : 0;
    }
    return await setupText(args, context, io, nodeVersion);
  } finally {
    io.close();
  }
}

interface SetupArguments extends SetupCarry {
  answer?: string;
  repo?: string;
  agents?: Harness[];
}

/** Checked before the install, so a typo changes nothing. */
function setupArguments(args: readonly string[]): SetupArguments {
  const { flags, options } = parseArguments(
    args,
    { flags: [JSON_FLAG, "--repository"], options: ["--answer", "--repo", "--agents", "--port", "--database"] },
    argument => `Unknown setup option: ${argument}`,
  );
  const port = options.get("--port");
  const answer = checkSetupAnswer(options.get("--answer"));
  const repo = options.get("--repo");
  const agents = options.get("--agents");
  const database = options.get("--database");
  return {
    ...(port === undefined ? {} : { port: parsePort(port) }),
    ...(database === undefined ? {} : { databasePath: resolve(database) }),
    ...(flags.has("--repository") ? { repository: true } : {}),
    ...(answer === undefined ? {} : { answer }),
    ...(repo === undefined ? {} : { repo }),
    ...(agents === undefined ? {} : { agents: parseCodingAgents(agents) }),
  };
}

/** Install when this package is not the active one. T2's line belongs to a first run that installed something, in a repo not on already. */
async function install(context: SetupContext, parsed: SetupArguments, cwd: string, lane: SetupLane): Promise<string | undefined> {
  if (await isActiveInstall(context)) return undefined;
  await context.updater.install();
  if (parsed.answer !== undefined || parsed.repo !== undefined || workspaceStartState(cwd, parsed.databasePath) === "started") return undefined;
  return renderInstalled({ version: context.updater.packageVersionLabel }, lane).replace(/\n$/u, "");
}

/** The run's carry: its own arguments, with the place's configured database when it names none. */
function carryOf(parsed: SetupArguments, place: SetupPlace): SetupCarry {
  const databasePath = parsed.databasePath ?? place.databasePath;
  return {
    ...(parsed.port === undefined ? {} : { port: parsed.port }),
    ...(databasePath === undefined ? {} : { databasePath }),
    ...(parsed.repository === true ? { repository: true } : {}),
  };
}

function setupRequest(context: SetupContext, parsed: SetupArguments, place: SetupPlace): SetupRequest {
  const carry = carryOf(parsed, place);
  const setupCommand = setupAnswerCommand(context, carry);
  return {
    cwd: place.cwd,
    ...(carry.databasePath === undefined ? {} : { databasePath: carry.databasePath }),
    ...(carry.repository === true ? { repository: true } : {}),
    ...(parsed.repo === undefined ? {} : { repo: parsed.repo }),
    ...(parsed.answer === undefined ? {} : { answer: parsed.answer }),
    ...(parsed.agents === undefined ? {} : { agents: parsed.agents }),
    ...(setupCommand === undefined ? {} : { setupCommand }),
    environment: place.environment(parsed.port),
  };
}

async function setupText(args: readonly string[], context: SetupContext, io: SetupIo, nodeVersion: string): Promise<number> {
  // E2 before anything else: an older Node changes nothing on this machine.
  const node = checkNodeVersion(nodeVersion, io.lane);
  if (!node.ok) {
    io.say(node.message.replace(/\n$/u, ""));
    return 1;
  }
  const parsed = setupArguments(args);
  const place = machinePlace(context);
  const installedLine = await install(context, parsed, place.cwd, io.lane);
  if (installedLine !== undefined) io.say(installedLine);
  const outcome = await runSetup(io, setupRequest(context, parsed, place));
  return outcome.status === "cancelled" ? 1 : 0;
}

/** `rerail setup --json` as one step: what the CLI prints, and what an extension running Node calls in-process. */
export async function setupStep(args: readonly string[], context: SetupContext, place: SetupPlace, nodeVersion: string = process.versions.node): Promise<SetupStep> {
  try {
    const node = checkNodeVersion(nodeVersion, "json");
    if (!node.ok) return errorStep(node.message.replace(/\n$/u, ""));
    const parsed = setupArguments(args);
    const installedLine = await install(context, parsed, place.cwd, "json");
    const step = await runSetupStep({ ...setupRequest(context, parsed, place), nextArgs: jsonNextArgs(carryOf(parsed, place)) });
    return installedLine === undefined ? step : { ...step, screen: `${installedLine}\n${step.screen}` };
  } catch (error) {
    return errorStep(messageOf(error));
  }
}

/** `rerail start --json` as one step: the set-up's first question where the repository is not set up, `done` otherwise. */
export async function startStep(args: readonly string[], context: SetupContext, place: SetupPlace): Promise<SetupStep> {
  try {
    const { flags, options } = parseArguments(
      args,
      { flags: [JSON_FLAG, "--repository"], options: ["--repo", "--agents", "--answer", "--database"] },
      argument => `Unknown start option: ${argument}`,
    );
    const agents = options.get("--agents");
    const database = options.get("--database");
    const databasePath = database === undefined ? place.databasePath : resolve(database);
    const repository = flags.has("--repository");
    const answer = checkSetupAnswer(options.get("--answer"));
    const workspace = resolve(place.cwd, options.get("--repo") ?? ".");
    // A session opened in the home folder has no repository to start: the set-up asks which project (E3).
    if (workspace === resolve(homedir())) {
      const passed = [
        ...(answer === undefined ? [] : ["--answer", answer]),
        ...(agents === undefined ? [] : ["--agents", agents]),
        ...(databasePath === undefined ? [] : ["--database", databasePath]),
        ...(repository ? ["--repository"] : []),
      ];
      return await setupStep([JSON_FLAG, ...passed], context, { ...place, cwd: workspace });
    }
    const carry: SetupCarry = { ...(databasePath === undefined ? {} : { databasePath }), ...(repository ? { repository } : {}) };
    const setupCommand = setupAnswerCommand(context, carry);
    return await startRerailStep(workspace, {
      ...carry,
      ...(agents === undefined ? {} : { agents: parseCodingAgents(agents) }),
      ...(answer === undefined ? {} : { answer }),
      ...(setupCommand === undefined ? {} : { setupCommand }),
      nextArgs: jsonNextArgs(carry),
      environment: place.environment(),
    });
  } catch (error) {
    return errorStep(messageOf(error));
  }
}

/** One `--json` run, `setup …` or `start …`, answered in-process as the launcher answers it (the picker's tests). */
export function runSetupStepArgs(args: readonly string[], context: SetupContext, place: SetupPlace): Promise<SetupStep> {
  const [command, ...rest] = args;
  if (command === "setup") return setupStep(rest, context, place);
  if (command === "start") return startStep(rest, context, place);
  return Promise.resolve(errorStep(`Unknown set-up command: ${command ?? ""}`));
}

/** What every `--json` answer's `next.args` start with: set-up again, with what this run carries, as JSON. */
function jsonNextArgs(carry: SetupCarry): string[] {
  return ["setup", ...carriedArgs(carry), JSON_FLAG];
}
