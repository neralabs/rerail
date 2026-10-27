import { existsSync, lstatSync, readdirSync, readFileSync } from "node:fs";
import { mkdir, readdir, realpath, rename, rm, unlink } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { rulesDashboardUrl, type RulesDashboardScope } from "../api/session-contracts.js";
import {
  CODING_AGENT_NAMES,
  codingAgentHome,
  connectCodingAgents,
  forgetSeenCodingAgents,
  recordSeenCodingAgents,
  refreshCodingAgents,
  registrationRoot as packageRegistrationRoot,
  seenCodingAgents,
} from "../setup/coding-agents.js";
import { RERAIL_DATABASE_FILE } from "../engine/storage/database.js";
import { installedPacksPrefix } from "../engine/catalog/rule-catalog.js";
import { MAIN_INSTANCE_HOLDER_FILE } from "../engine/native/main-instance.js";
import { mainContentStoreDirectory } from "../engine/native/rerail-build.js";
import { daemonUsesPackageRoot, lookupDaemon, openDashboard, processAlive, startDaemon, stopDaemon, type DaemonState } from "../runtime/daemon-control.js";
import { DEFAULT_DAEMON_PORT, parseArguments, parsePort } from "../daemon/daemon.js";
import {
  detectHarnesses,
  registerHarnesses,
  unregisterHarnesses,
  type Harness,
  type RegistrationOptions,
  type RegistrationReport,
} from "../setup/harness-registration.js";
import { checkNodeVersion } from "../setup/node-version.js";
import { pinnedNodePath } from "../setup/node-path.js";
import { setupPreflight, wslNote } from "../setup/preflight.js";
import { addLauncherToPath, describeLauncherPath, launcherPathMarker, recordLauncherPath, removeLauncherFromPath } from "../setup/shell-path.js";
import { TerminalInput } from "./setup-menu.js";
import { checkSetupAnswer, parseCodingAgents, runSetup, runSetupStep, setupAgents, setupPacks, startRerailStep, type DashboardPage, type SetupEnvironment, type SetupRequest } from "./repository-setup.js";
import { npmPackStore, type PackStore, type PackVersion } from "../setup/rule-pack-store.js";
import { workspaceStartState, type RepositorySetupIo } from "../runtime/repository-start.js";
import { renderBanner, renderInstalled, renderOnPath, SETUP_FLAGS, shellQuote, type DashboardLink, type SetupLane } from "../harness/setup-screens.js";
import { colourDepth } from "../presentation/rerail-mark.js";
import type { SetupStep } from "../harness/setup-step.js";
import { LAUNCHER_HEADER, type ManagedUpdater } from "../setup/updater.js";
import { readConsent, updateConsent } from "./consent.js";

export interface SetupContext {
  /** Rerail's own state directory (`~/.rerail` unless `RERAIL_HOME`); versions, launcher, daemon live here. */
  home: string;
  /** The package this launcher runs from: an npx cache, a global install, or a checkout. */
  packageRoot: string;
  updater: ManagedUpdater;
  /** Where rule packs come from and go to; npm, into `home`, unless given. */
  packs?: PackStore;
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

/** Put the launcher's directory on PATH (`shell-path.ts`); set-up's line for it, or undefined when nothing changed. */
async function launcherOnPath(context: SetupContext, lane: SetupLane): Promise<string | undefined> {
  const { launcherPath } = context.updater;
  // A run that wrote no launcher (an install that failed) has nothing to put on PATH yet.
  if (!existsSync(launcherPath)) return undefined;
  const binDir = dirname(launcherPath);
  const home = codingAgentHome();
  const change = await addLauncherToPath({ binDir, home });
  await recordLauncherPath(context.home, change);
  if (change.status === "failed") return renderOnPath({ binDir, locations: [], failure: change.reason, home }, lane).replace(/\n$/u, "");
  if (change.status !== "changed") return undefined;
  return renderOnPath({ binDir, locations: change.locations, home }, lane).replace(/\n$/u, "");
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

/**
 * What the uninstall deletes from the installation, each with what it is, in the order it goes. A folder with `ours`
 * goes whole only when everything in it is Rerail's; otherwise only Rerail's entries go and the folder stays, so a
 * Rerail home shared with other files (`RERAIL_HOME=$HOME`) keeps its own `bin`, `versions` or `packs`.
 */
function installedFiles(context: SetupContext): Array<{ path: string; what: string; ours?: (name: string, path: string) => boolean }> {
  const { updater } = context;
  // `rerail.cmd` and, on Windows, Git Bash's `rerail` beside it; each carries the header Rerail writes. A launcher
  // write cut short leaves `<launcher>.<pid>.<uuid>.tmp`, possibly before the header was written.
  const launchers = new Set([basename(updater.launcherPath), "rerail"]);
  const launcherWrite = (name: string) => /^(.+)\.\d+\.[0-9a-f-]+\.tmp$/u.exec(name)?.[1];
  return [
    { path: updater.currentLink, what: "the active version's link" },
    {
      path: dirname(updater.launcherPath),
      what: "the rerail command",
      ours: (name, path) => (launchers.has(name) && isRerailLauncher(path)) || launchers.has(launcherWrite(name) ?? ""),
    },
    {
      path: updater.versionsRoot,
      what: "the installed versions",
      ours: (name, path) => /^\..+\.staging$/u.test(name) || existsSync(join(path, "node_modules", "@neralabs", "rerail")),
    },
    { path: updater.statePath, what: "the record of them" },
    { path: installedPacksPrefix(context.home), what: "the installed rule packs", ours: name => RULE_PACK_PREFIX_ENTRIES.has(name) },
    { path: mainContentStoreDirectory(), what: "the cache" },
  ];
}

/** What npm and the rule pack store put in the packs prefix, with the lock a pack change holds. */
const RULE_PACK_PREFIX_ENTRIES: ReadonlySet<string> = new Set(["node_modules", "package.json", "package-lock.json", "staging", ".lock"]);

/**
 * What Rerail itself writes in its home: settings, records, logs, locks, its databases, `rerail activate`'s profiles
 * and device secrets, `rules export`'s `rules.json` and the installation, each with the `.<pid>[.<uuid>].tmp` or
 * `.old` a write cut short leaves beside it.
 */
const RERAIL_HOME_ENTRY =
  /^(?:(?:coding-agents|consent|daemon|daemon-port|launcher-path|update-state|rules)\.json|daemon\.log.*|(?:harness|rerail)\.sqlite.*|(?:start|update)\.lock|bin|current|versions|packs|profiles|active-profile|device-secrets)(?:\.\d+(?:\.[0-9a-f-]+)?\.(?:tmp|old))?$/u;

/**
 * The command that finishes removing `path` by hand. PowerShell: single quotes expand nothing, and `-LiteralPath` reads
 * `[` and `]` as themselves. A link (`current`, a junction on Windows) goes through .NET's non-recursive
 * `Directory.Delete`, which removes the link and never follows it as Windows PowerShell's `Remove-Item -Recurse` does,
 * and, unlike cmd's `rmdir`, expands no `%NAME%` in the path.
 */
export function cleanupCommand(path: string, link: boolean, platform: NodeJS.Platform = process.platform): string {
  if (platform !== "win32") return `rm -rf ${shellQuote(path, platform)}`;
  const powershell = `'${path.replaceAll("'", "''")}'`;
  return link ? `[System.IO.Directory]::Delete(${powershell})` : `Remove-Item -LiteralPath ${powershell} -Recurse -Force`;
}

function isRerailLauncher(path: string): boolean {
  try {
    return readFileSync(path, "utf8").includes(LAUNCHER_HEADER);
  } catch {
    return false;
  }
}

function pathExists(path: string): boolean {
  try {
    lstatSync(path);
    return true;
  } catch {
    return false;
  }
}

/** Another Rerail daemon (another `RERAIL_HOME`'s) that still holds the machine-wide cache, by pid. */
function cacheHolder(cache: string): number | undefined {
  try {
    const { pid } = JSON.parse(readFileSync(join(cache, MAIN_INSTANCE_HOLDER_FILE), "utf8")) as { pid?: unknown };
    return typeof pid === "number" && Number.isInteger(pid) && pid !== process.pid && processAlive(pid) ? pid : undefined;
  } catch {
    return undefined;
  }
}

/** What the uninstall removes and keeps, decided before anything goes so `--dry-run` says the same as the real run. */
function planRemoval(context: SetupContext): { remove: Array<{ path: string; what: string }>; keep: Array<{ path: string; what: string; why: string }> } {
  const remove: Array<{ path: string; what: string }> = [];
  const keep: Array<{ path: string; what: string; why: string }> = [];
  for (const { path, what, ours } of installedFiles(context)) {
    if (!pathExists(path)) continue;
    const holder = path === mainContentStoreDirectory() ? cacheHolder(path) : undefined;
    if (holder !== undefined) {
      keep.push({ path, what, why: `another Rerail daemon, pid ${holder}, still uses it` });
      continue;
    }
    if (ours === undefined) {
      remove.push({ path, what });
      continue;
    }
    let entries: string[];
    try {
      entries = readdirSync(path);
    } catch {
      keep.push({ path, what, why: "it is not a folder Rerail made" });
      continue;
    }
    const foreign = entries.filter(name => !ours(name, join(path, name)));
    if (foreign.length === 0) {
      remove.push({ path, what });
      continue;
    }
    for (const name of entries) if (!foreign.includes(name)) remove.push({ path: join(path, name), what });
    keep.push({ path, what, why: `it holds files that are not Rerail's: ${foreign.sort().join(", ")}` });
  }
  return { remove, keep };
}

/**
 * The uninstall's last step, once the coding agents are disconnected, the daemon has stopped and the database is in
 * the backup: delete the installation itself (the versions, the launcher, `current`, the rule packs and the cache), so
 * "removed from this computer" holds. `current` is unlinked, never followed: `install --root` points it at a checkout.
 * A file Windows keeps locked (a session still running Rerail, an antivirus scan) is named with the command that
 * finishes the job; the uninstall itself has happened, so it still exits 0. What stays in the Rerail home is named too.
 */
async function removeInstallation(context: SetupContext, dryRun: boolean): Promise<void> {
  const { remove, keep } = planRemoval(context);
  if (dryRun) {
    for (const { path, what } of remove) process.stdout.write(`Would remove ${path} (${what}).\n`);
    for (const { path, what, why } of keep) process.stdout.write(`Would keep ${path} (${what}): ${why}.\n`);
    return;
  }
  const failed: Array<{ path: string; reason: string }> = [];
  for (const { path, what } of remove) {
    try {
      if (path === context.updater.currentLink) await unlink(path);
      // rm retries Windows' transient EBUSY and EPERM itself (as the updater's staging cleanup does).
      else await rm(path, { recursive: true, force: true, maxRetries: 10 });
      process.stdout.write(`Removed ${path} (${what}).\n`);
    } catch (error) {
      failed.push({ path, reason: error instanceof Error ? error.message : String(error) });
    }
  }
  for (const { path, what, why } of keep) process.stdout.write(`Kept ${path} (${what}): ${why}.\n`);
  for (const { path, reason } of failed) {
    const command = cleanupCommand(path, path === context.updater.currentLink);
    process.stdout.write(`Could not remove ${path} (${reason}). Close every coding agent and terminal still running Rerail, then run: ${command}\n`);
  }
  const kept = (await readdir(context.home).catch(() => [])).sort();
  // A folder kept for the files in it that are not Rerail's (`bin`, `versions`, `packs`) is not Rerail's to delete.
  const keptForOthers = new Set(keep.map(({ path }) => path));
  const ours = kept.filter(name => RERAIL_HOME_ENTRY.test(name) && !keptForOthers.has(join(context.home, name)));
  // A Rerail home shared with other files (`RERAIL_HOME=$HOME`) is never offered for deletion as a whole.
  if (ours.length < kept.length) {
    process.stdout.write(`Kept ${context.home}, which also holds files that are not Rerail's.${ours.length === 0 ? "" : ` Rerail's settings, logs and records in it: ${ours.join(", ")}; delete those if you will not install Rerail again.`}\n`);
  } else if (kept.length > 0) {
    process.stdout.write(`Kept ${context.home}: ${kept.join(", ")} (Rerail's settings, logs and records; delete it if you will not install Rerail again).\n`);
  }
}

/**
 * `rerail install --refresh`: register this version again with every coding agent already connected to the managed
 * install, so commands it added arrive and ones it retired go, and write the launcher again; `rerail update` runs it
 * from the version it activated. Both then name the node it runs on. Coding agents never connected stay untouched, and
 * one whose configuration cannot be read to tell is named and left alone. Exits 1 only when a connected coding agent
 * could not be refreshed.
 */
async function runRefresh(context: SetupContext): Promise<number> {
  const root = registrationRoot(context);
  const result = await refreshCodingAgents(root, codingAgentHome());
  if (result.agents.length === 0 && result.failures.length === 0 && result.unchecked.length === 0) {
    process.stdout.write(`No coding agents are connected to ${root}; nothing to refresh.\n`);
  }
  if (result.agents.length > 0) {
    const changed = result.changes.filter(change => change.action === "created" || change.action === "updated");
    const names = result.agents.map(id => CODING_AGENT_NAMES[id]).join(", ");
    process.stdout.write(changed.length === 0 ? `Coding agents up to date: ${names}\n` : `Refreshed Rerail in ${names}:\n`);
    printReport({ changes: changed, notes: changed.length === 0 ? [] : result.notes }, false);
  }
  for (const { agent, message } of result.unchecked) {
    process.stdout.write(`  note: left ${CODING_AGENT_NAMES[agent]} alone: its configuration could not be read to tell whether Rerail is connected (${message})\n`);
  }
  // The hooks above now name the node this runs on; the launcher names it too, or it would keep the one an earlier
  // set-up recorded and, once that is upgraded or removed, fall back to whichever `node` PATH has.
  if (await context.updater.refreshLauncher()) {
    process.stdout.write(`Launcher: ${context.updater.launcherPath} now runs ${pinnedNodePath()}\n`);
  }
  // An install made by a version whose set-up never wrote PATH keeps the launcher off it until something puts it
  // on: the refresh every update ends with is that something, once. A set-up that handled PATH leaves a record, and
  // from then on PATH is the person's: a block they took out stays out.
  const pathLine = await repairLauncherPath(context);
  if (pathLine !== undefined) process.stdout.write(`${pathLine}\n`);
  for (const { agent, message } of result.failures) {
    process.stderr.write(`rerail: could not refresh ${CODING_AGENT_NAMES[agent]}: ${message}\n`);
  }
  if (result.failures.length === 0) return 0;
  process.stderr.write("rerail: fix that, then run rerail install --refresh\n");
  return 1;
}

/**
 * Put the launcher on PATH where no set-up ever handled it; the line to print when that changed or failed, else
 * undefined. A run that wrote no launcher (an install that failed) has nothing to put on PATH yet.
 */
async function repairLauncherPath(context: SetupContext): Promise<string | undefined> {
  const { launcherPath } = context.updater;
  if (existsSync(launcherPathMarker(context.home)) || !existsSync(launcherPath)) return undefined;
  const binDir = dirname(launcherPath);
  const change = await addLauncherToPath({ binDir, home: codingAgentHome() });
  await recordLauncherPath(context.home, change);
  return describeLauncherPath(change, binDir, "add");
}

export async function runInstallCommand(args: readonly string[], context: SetupContext): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--dry-run", "--remove", "--refresh"], options: ["--root", "--agents"] },
    argument => `Unknown install option: ${argument}`,
  );
  if (flags.has("--refresh")) {
    if (flags.size > 1 || options.size > 0) throw new Error("install --refresh takes no other options");
    return await runRefresh(context);
  }
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
  // What `rerail install` found or was named is seen, so set-up run again connects no agent behind this choice.
  if (!parsed.dryRun) await recordSeenCodingAgents(context.home, [...detectHarnesses(codingAgentHome()), ...(parsed.hosts ?? [])]);
  // Removing every coding agent from the managed install is the uninstall. Removing one coding agent, or a
  // checkout registered with `--root`, leaves the shared rules and history in place.
  const uninstall = remove && parsed.hosts === undefined && (explicitRoot === undefined || root === context.updater.currentLink);
  if (uninstall) {
    await backUpRulesAndHistory(context.home, parsed.dryRun);
    const binDir = dirname(context.updater.launcherPath);
    const removed = describeLauncherPath(await removeLauncherFromPath({ binDir, home: codingAgentHome(), dryRun: parsed.dryRun }), binDir, "remove", parsed.dryRun);
    if (removed !== undefined) process.stdout.write(`${removed}\n`);
    if (!parsed.dryRun) await rm(launcherPathMarker(context.home), { force: true });
    await removeInstallation(context, parsed.dryRun);
  }
  if (explicitRoot !== undefined && !remove) {
    const binDir = dirname(context.updater.launcherPath);
    const change = await addLauncherToPath({ binDir, home: codingAgentHome(), dryRun: parsed.dryRun });
    await recordLauncherPath(context.home, change, parsed.dryRun);
    const pathLine = describeLauncherPath(change, binDir, "add", parsed.dryRun);
    process.stdout.write(["", ...(pathLine === undefined ? [] : [pathLine]), "Next:", "  restart Cursor, Claude Code, Codex, OMP, or Pi so they load the new hooks", ""].join("\n"));
  }
  return 0;
}

/** Where set-up talks: a terminal it can ask, or a coding agent that relays each question. */
export interface SetupIo extends RepositorySetupIo {
  close(): void;
}

/**
 * The terminal lane needs a TTY on both stdin and stdout, and no `--relay`: set-up never waits on input that a coding
 * agent, or a pipe, will not type. `/rerail-start` and every answer printed for a coding agent pass `--relay`.
 */
export function setupIo(args: readonly string[] = []): SetupIo {
  const say = (text: string): void => {
    process.stdout.write(`${text}\n`);
  };
  if (args.includes(SETUP_FLAGS.relay) || !process.stdin.isTTY || !process.stdout.isTTY) return { lane: "agent", say, close: () => {} };
  // One reader for the run: lines for the folder and pack prompts, keys for each question's menu, and what is typed
  // ahead of a prompt kept for it.
  const input = new TerminalInput(process.stdin, text => process.stdout.write(text), undefined, () => process.stdout.columns);
  return {
    lane: "terminal",
    say,
    ask: async prompt => await input.line(prompt),
    choose: async menu => await input.choose(menu),
    close: () => input.close(),
  };
}

/** Why no browser can open here (E6), or undefined when one can. */
export function noBrowserReason(env: NodeJS.ProcessEnv, platform: NodeJS.Platform): string | undefined {
  if (env.BROWSER === "none") return "BROWSER is set to none";
  if (env.SSH_CONNECTION || env.SSH_CLIENT || env.SSH_TTY) return "this looks like a remote session";
  if (platform === "linux" && !env.DISPLAY && !env.WAYLAND_DISPLAY) return "there is no display here";
  return undefined;
}

/** The dashboard's hash route for each page set-up links to. */
const DASHBOARD_ROUTES: Record<DashboardPage, string> = { rules: "rules" };

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

/** What set-up needs from this machine: the coding agents, their registration, the dashboard, and npm for rule packs. */
export function setupEnvironment(context: SetupContext, port?: number): SetupEnvironment {
  return {
    packs: context.packs ?? npmPackStore(context.home),
    agentsFound: () => detectHarnesses(codingAgentHome()),
    agentsSeen: () => seenCodingAgents(context.home),
    async connect(agents, repoRoot) {
      const home = codingAgentHome();
      // Claude Code's project status line lives in the repository: only one the person said yes to.
      const result = await connectCodingAgents(registrationRoot(context), home, agents, repoRoot);
      // One that could not be connected stays unseen, even if `rerail install` recorded it before, so set-up run again
      // connects it once its config is fixed.
      const failed = result.failures.map(({ agent }) => agent);
      await recordSeenCodingAgents(context.home, [...detectHarnesses(home), ...agents].filter(id => !failed.includes(id)));
      await forgetSeenCodingAgents(context.home, failed);
      return result;
    },
    sessionsApproved() {
      return readConsent(context.home).sessions !== undefined;
    },
    async setSessionsApproved(approved) {
      updateConsent(context.home, ({ sessions: _sessions, ...others }) =>
        approved ? { ...others, sessions: { approvedAt: new Date().toISOString() } } : others);
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

/** What a set-up run carries into every answer it prints, so the next run acts on the same database, port, scope, coding agents and lane. */
export interface SetupCarry {
  readonly port?: number;
  readonly databasePath?: string;
  /** `rerail start --repository`: set-up starts every checkout of the repository. */
  readonly repository?: boolean;
  /** `--agents`: connect only these coding agents, not every one found. */
  readonly agents?: readonly Harness[];
  /** `--relay`: the agent lane's answers say so, so a coding agent that runs them in a terminal still gets the agent lane. */
  readonly relay?: boolean;
}

function carriedArgs(carry: SetupCarry): string[] {
  return [
    ...(carry.relay === true ? [SETUP_FLAGS.relay] : []),
    ...(carry.port === undefined ? [] : ["--port", String(carry.port)]),
    ...(carry.databasePath === undefined ? [] : ["--database", carry.databasePath]),
    ...(carry.repository === true ? ["--repository"] : []),
    // Ids, as `parseCodingAgents` reads them; an answer's own `--agents` comes later and wins.
    ...(carry.agents === undefined ? [] : ["--agents", carry.agents.join(",")]),
  ];
}

/**
 * What the coding agent runs to answer a set-up question: the launcher setup wrote, which stays put
 * between answers (an `npx` copy may not), with what this run carries, quoted for `platform`'s shell.
 * `rerail setup` until the launcher exists.
 */
export function setupAnswerCommand(context: SetupContext, carry: SetupCarry = {}, platform: NodeJS.Platform = process.platform): string | undefined {
  const carried = carriedArgs(carry).map(arg => shellQuote(arg, platform));
  if (!existsSync(context.updater.launcherPath)) return carried.length === 0 ? undefined : ["rerail", "setup", ...carried].join(" ");
  return [shellQuote(context.updater.launcherPath, platform), "setup", ...carried].join(" ");
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
 * in this repository only, and the summary. Without a TTY, or with `--relay`, each run stops at the next question for
 * the coding agent, which runs set-up again with `--answer`, `--repo` and `--agents`. With `--json` the
 * same run prints one `SetupStep` and nothing else, failures included.
 */
export async function runSetupCommand(
  args: readonly string[],
  context: SetupContext,
  io: SetupIo = setupIo(args),
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
  agentsError?: string;
  packs?: readonly PackVersion[];
  packsError?: string;
}

/** Checked before the install, so a typo changes nothing. */
function setupArguments(args: readonly string[]): SetupArguments {
  const { flags, options } = parseArguments(
    args,
    { flags: [JSON_FLAG, SETUP_FLAGS.relay, "--repository"], options: ["--answer", "--repo", "--agents", SETUP_FLAGS.packs, "--port", "--database"] },
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
    ...setupAgents(agents, answer),
    ...setupPacks(options.get(SETUP_FLAGS.packs), answer),
  };
}

/** The command a set-up run came from: `rerail setup` itself, or `rerail start --json` handing over to it. */
type SetupOrigin = "setup" | "start";

/**
 * Make this package the active install (T2) when it is not. Only a fresh `rerail setup` installs over another
 * version. An answer (`--answer`, `--repo`), or a step `rerail start --json` began, installs only while nothing is
 * installed yet: OMP's and Pi's `/rerail-start` runs those steps from the session's own package, fixed when the
 * session loaded it, and adopting that one after a `/rerail-update` would undo the update. The coding agent's answer
 * commands run the launcher, which is the active version already. T2's line belongs to a first run that installed
 * something, in a repo not on already.
 */
async function install(context: SetupContext, parsed: SetupArguments, cwd: string, lane: SetupLane, origin: SetupOrigin): Promise<string | undefined> {
  // `active` is undefined before the first install.
  const [active, running] = await Promise.all([
    realpath(context.updater.currentLink).catch(() => undefined),
    realpath(context.packageRoot).catch(() => context.packageRoot),
  ]);
  if (active === running) return undefined;
  const answering = parsed.answer !== undefined || parsed.repo !== undefined;
  if (active !== undefined && (answering || origin === "start")) return undefined;
  await context.updater.install();
  if (answering || workspaceStartState(cwd, parsed.databasePath) === "started") return undefined;
  return renderInstalled(lane).replace(/\n$/u, "");
}

/** The run's carry: its own arguments, with the place's configured database when it names none. */
function carryOf(parsed: SetupArguments, place: SetupPlace): SetupCarry {
  const databasePath = parsed.databasePath ?? place.databasePath;
  return {
    ...(parsed.port === undefined ? {} : { port: parsed.port }),
    ...(databasePath === undefined ? {} : { databasePath }),
    ...(parsed.repository === true ? { repository: true } : {}),
    ...(parsed.agents === undefined ? {} : { agents: parsed.agents }),
  };
}

/** The agent lane's request carries `--relay` into every answer it prints. */
function setupRequest(context: SetupContext, parsed: SetupArguments, place: SetupPlace, lane: SetupLane): SetupRequest {
  const carry = { ...carryOf(parsed, place), ...(lane === "agent" ? { relay: true } : {}) };
  const setupCommand = setupAnswerCommand(context, carry);
  return {
    cwd: place.cwd,
    ...(carry.databasePath === undefined ? {} : { databasePath: carry.databasePath }),
    ...(carry.repository === true ? { repository: true } : {}),
    ...(parsed.repo === undefined ? {} : { repo: parsed.repo }),
    ...(parsed.answer === undefined ? {} : { answer: parsed.answer }),
    ...(parsed.agents === undefined ? {} : { agents: parsed.agents }),
    ...(parsed.agentsError === undefined ? {} : { agentsError: parsed.agentsError }),
    ...(parsed.packs === undefined ? {} : { packs: parsed.packs }),
    ...(parsed.packsError === undefined ? {} : { packsError: parsed.packsError }),
    ...(setupCommand === undefined ? {} : { setupCommand }),
    environment: place.environment(parsed.port),
  };
}

/** T2w for the folder this run is for, once: on the first run and when a folder is named, not on every answer. */
function setupWslNote(place: SetupPlace, parsed: SetupArguments, lane: SetupLane): string | undefined {
  if (parsed.answer !== undefined) return undefined;
  return wslNote(resolve(place.cwd, parsed.repo ?? "."), lane)?.replace(/\n$/u, "");
}

async function setupText(args: readonly string[], context: SetupContext, io: SetupIo, nodeVersion: string): Promise<number> {
  // E2 before anything else: an older Node changes nothing on this machine.
  const node = checkNodeVersion(nodeVersion, io.lane);
  if (!node.ok) {
    io.say(node.message.replace(/\n$/u, ""));
    return 1;
  }
  // E7 and E8: an engine that cannot load here, or no Git, stop set-up before it installs anything.
  const stop = setupPreflight(io.lane);
  if (stop !== undefined) {
    io.say(stop.replace(/\n$/u, ""));
    return 1;
  }
  const parsed = setupArguments(args);
  const banner = renderBanner({ version: context.updater.packageVersionLabel, depth: colourDepth() }, io.lane);
  if (banner !== "") io.say(banner.replace(/\n$/u, ""));
  const place = machinePlace(context);
  const wsl = setupWslNote(place, parsed, io.lane);
  if (wsl !== undefined) io.say(wsl);
  const installedLine = await install(context, parsed, place.cwd, io.lane, "setup");
  if (installedLine !== undefined) io.say(installedLine);
  const pathLine = await launcherOnPath(context, io.lane);
  if (pathLine !== undefined) io.say(pathLine);
  const outcome = await runSetup(io, setupRequest(context, parsed, place, io.lane));
  return outcome.status === "cancelled" ? 1 : 0;
}

/** `rerail setup --json` as one step: what the CLI prints, and what an extension running Node calls in-process. */
export function setupStep(args: readonly string[], context: SetupContext, place: SetupPlace, nodeVersion: string = process.versions.node): Promise<SetupStep> {
  return jsonSetupStep(args, context, place, nodeVersion, "setup");
}

async function jsonSetupStep(args: readonly string[], context: SetupContext, place: SetupPlace, nodeVersion: string, origin: SetupOrigin): Promise<SetupStep> {
  try {
    const node = checkNodeVersion(nodeVersion, "json");
    if (!node.ok) return errorStep(node.message.replace(/\n$/u, ""));
    const stop = setupPreflight("json");
    if (stop !== undefined) return errorStep(stop.replace(/\n$/u, ""));
    const parsed = setupArguments(args);
    const wsl = setupWslNote(place, parsed, "json");
    const installedLine = await install(context, parsed, place.cwd, "json", origin);
    const pathLine = await launcherOnPath(context, "json");
    const step = await runSetupStep({ ...setupRequest(context, parsed, place, "json"), nextArgs: jsonNextArgs(carryOf(parsed, place)) });
    const opening = [wsl, installedLine, pathLine].filter(line => line !== undefined);
    return opening.length === 0 ? step : { ...step, screen: [...opening, step.screen].join("\n") };
  } catch (error) {
    return errorStep(messageOf(error));
  }
}

/**
 * `rerail start --json` as one step: the set-up's first question where the repository is not set up, `done` otherwise.
 * `--relay`, which `/rerail-start` passes, is accepted and `--json` wins: the step is JSON either way.
 */
export async function startStep(args: readonly string[], context: SetupContext, place: SetupPlace): Promise<SetupStep> {
  try {
    const { flags, options } = parseArguments(
      args,
      { flags: [JSON_FLAG, SETUP_FLAGS.relay, "--repository"], options: ["--repo", "--agents", SETUP_FLAGS.packs, "--answer", "--database"] },
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
        ...(options.has(SETUP_FLAGS.packs) ? [SETUP_FLAGS.packs, options.get(SETUP_FLAGS.packs)!] : []),
        ...(databasePath === undefined ? [] : ["--database", databasePath]),
        ...(repository ? ["--repository"] : []),
      ];
      return await jsonSetupStep([JSON_FLAG, ...passed], context, { ...place, cwd: workspace }, process.versions.node, "start");
    }
    const { agentsError, ...chosen } = setupAgents(agents, answer);
    const carry: SetupCarry = {
      ...(databasePath === undefined ? {} : { databasePath }),
      ...(repository ? { repository } : {}),
      ...chosen,
    };
    const setupCommand = setupAnswerCommand(context, carry);
    return await startRerailStep(workspace, {
      ...carry,
      ...(agentsError === undefined ? {} : { agentsError }),
      ...setupPacks(options.get(SETUP_FLAGS.packs), answer),
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
