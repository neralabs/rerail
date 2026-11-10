import { existsSync, lstatSync, readdirSync, readFileSync } from "node:fs";
import { mkdir, readdir, realpath, rm, rmdir, unlink, writeFile } from "node:fs/promises";
import { FOLDER_REMOVAL_RETRIES } from "../engine/folder-removal.js";
import { renameWithRetry } from "../engine/file-replace.js";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { createRequire } from "node:module";
import type * as Sqlite from "node:sqlite";
import { rulesDashboardUrl, type RulesDashboardScope } from "../api/session-contracts.js";
import {
  CODING_AGENT_NAMES,
  codingAgentHome,
  connectCodingAgents,
  forgetSeenCodingAgents,
  recordSeenCodingAgents,
  refreshCodingAgents,
  registrationRoot as packageRegistrationRoot,
  SEEN_AGENTS_FILE,
  seenCodingAgents,
} from "../setup/coding-agents.js";
import { RERAIL_DATABASE_FILE } from "../engine/storage/database.js";
import { commandLine, powershellLiteral, type Shell } from "../engine/command-line.js";
import { ENTRY_POINTS } from "../engine/package-root.js";
import { RULES_DUMP_FILE } from "../engine/catalog/rule-bootstrap.js";
import { INSTALLED_PACKS_DIR, installedPacksPrefix } from "../engine/catalog/rule-catalog.js";
import { MAIN_INSTANCE_HOLDER_FILE } from "../engine/native/main-instance.js";
import { contentStoreOverride, legacyWindowsContentStoreDirectory, mainContentStoreDirectory } from "../engine/native/rerail-build.js";
import {
  DAEMON_ACCESS_TOKEN_FILE,
  DAEMON_LOG_FILE,
  DAEMON_STATE_FILE,
  daemonMayHold,
  daemonUsesPackageRoot,
  lookupDaemon,
  openDashboard,
  START_LOCK_DIR,
  startDaemon,
  stopDaemon,
  type DaemonMachine,
  type DaemonState,
} from "../runtime/daemon-control.js";
import { DAEMON_PORT_FILE, DEFAULT_DAEMON_PORT, parseArguments, parsePort } from "../daemon/daemon.js";
import { ACTIVE_PROFILE_FILE, PROFILES_DIR } from "../activation/customer-profile.js";
import { DEVICE_SECRETS_DIR } from "../activation/device-keystore.js";
import { CLAUDE_STATUS_DATABASE_FILE } from "../setup/claude-status-store.js";
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
import { addLauncherToPath, describeLauncherPath, LAUNCHER_PATH_FILE, launcherPathMarker, recordLauncherPath, removeLauncherFromPath } from "../setup/shell-path.js";
import { TerminalInput } from "./setup-menu.js";
import { checkSetupAnswer, parseCodingAgents, runSetup, runSetupStep, setupAgents, setupPacks, startRerailStep, type DashboardPage, type SetupEnvironment, type SetupRequest } from "./repository-setup.js";
import { npmPackStore, type PackStore, type PackVersion } from "../setup/rule-pack-store.js";
import { workspaceStartState, type RepositorySetupIo } from "../runtime/repository-start.js";
import { renderBanner, renderInstalled, renderOnPath, SETUP_FLAGS, type DashboardLink, type SetupCommand, type SetupLane } from "../harness/setup-screens.js";
import { colourDepth } from "../presentation/rerail-mark.js";
import type { SetupStep } from "../harness/setup-step.js";
import { CURRENT_LINK, LAUNCHER_DIR, LAUNCHER_HEADER, UPDATE_LOCK_DIR, UPDATE_STATE_FILE, VERSIONS_DIR, type ManagedUpdater } from "../setup/updater.js";
import { requireLfsContent } from "../setup/lfs-pointers.js";
import { SCRUB_HINT_FILE, scrubHintOnce } from "./scrub-cli.js";
import { HOOK_FAILURES_FILE } from "../harness/hook-failures.js";
import { runRegisteredClaudeHook } from "../setup/registered-hooks.js";

export interface SetupContext {
  /** Rerail's own state directory (`~/.rerail` unless `RERAIL_HOME`); versions, launcher, daemon live here. */
  home: string;
  /** The package this launcher runs from: an npx cache, a global install, or a checkout. */
  packageRoot: string;
  updater: ManagedUpdater;
  /** Where rule packs come from and go to; npm, into `home`, unless given. */
  packs?: PackStore;
  /**
   * The machine whose processes and named pipes say whether another Rerail daemon still holds the cache the uninstall
   * would remove (`cacheHolder`); this one unless a test names another.
   */
  machine?: DaemonMachine;
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
  // A checkout cloned without Git LFS builds, then fails every session on the model's checksum.
  requireLfsContent(root);
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
/** How long a database move waits for Windows to release a lock. */
const LOCKED_MOVE_MS = 10_000;

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
    try {
      await renameWithRetry(join(home, name), join(target, name), { retryMs: LOCKED_MOVE_MS });
    } catch (error) {
      const code = error instanceof Error && "code" in error ? error.code : undefined;
      // A sidecar SQLite removed between the listing and the move holds nothing the main file lacks.
      if (code !== "ENOENT" || !/-(wal|shm)$/u.test(name)) throw error;
    }
  }
  process.stdout.write(`Backup: ${target} holds your rules and history. Install Rerail again and restore this backup to get them back.\n`);
}

/** Rerail's databases in its home, with every file named after them: WAL, shared memory, journal, `.restoring`, copies. */
const HOME_DATABASE = /^(?:rerail|harness)\.sqlite/u;

/** A directory itself, never a link to one. */
function isRealDirectory(path: string): boolean {
  try {
    return lstatSync(path).isDirectory();
  } catch {
    return false;
  }
}

/**
 * Every checkout the database at `path` records, read before the database is deleted, or why they could not be read.
 * The connection is a writer's so that, as the last one, its close removes the `-wal` and `-shm` its open needed: a dry
 * run leaves the files as it found them. Nothing else in this process has the file open. It reads through Node's own
 * `node:sqlite`, which loads no file: better-sqlite3 would load its addon from `versions/<v>/node_modules` into this
 * process, Node never unloads an addon, and Windows refuses to delete a loaded DLL, so the uninstall could never remove
 * the versions folder and told the person to close their coding agents.
 */
export function recordedCheckouts(path: string): { checkouts: string[]; problem?: string } {
  if (!existsSync(path)) return { checkouts: [] };
  let client: Sqlite.DatabaseSync | undefined;
  try {
    const sqlite: typeof Sqlite = createRequire(import.meta.url)("node:sqlite");
    client = new sqlite.DatabaseSync(path, { timeout: 5000 });
    const rows = client.prepare("SELECT checkout_path AS path FROM checkouts ORDER BY checkout_path").all();
    return { checkouts: rows.flatMap(row => (typeof row.path === "string" ? [row.path] : [])) };
  } catch (error) {
    return { checkouts: [], problem: error instanceof Error ? error.message : String(error) };
  } finally {
    client?.close();
  }
}

/**
 * `<checkout>/.rerail/sessions` when the checkout and its `.rerail` are real folders: a link at either is never
 * followed, so a link elsewhere cannot point the deletion at anything outside the checkout. The ledger itself may be a
 * link; deleting it removes the link, never what it points at.
 */
function sessionLedger(checkout: string): string | undefined {
  const folder = join(checkout, ".rerail");
  if (!isAbsolute(checkout) || !isRealDirectory(checkout) || !isRealDirectory(folder)) return undefined;
  const ledger = join(folder, "sessions");
  return pathExists(ledger) ? ledger : undefined;
}

/**
 * What `--delete-data` deletes for good where the uninstall would back up, decided before anything goes so `--dry-run`
 * says the same as the real run: the session ledgers of every checkout the database records, the Rerail home's
 * databases, every `~/.rerail-clean-*` backup, and the content store `RERAIL_CONTENT_STORE_PATH` names. The machine-wide
 * cache folder goes with the installation (`removeInstallation`), unless another daemon holds it.
 */
function planDataDeletion(
  home: string,
  machine: DaemonMachine | undefined,
): { remove: Array<{ path: string; what: string; ledger?: true }>; notes: string[] } {
  const remove: Array<{ path: string; what: string; ledger?: true }> = [];
  const notes: string[] = [];
  const database = join(home, RERAIL_DATABASE_FILE);
  const { checkouts, problem } = recordedCheckouts(database);
  if (problem !== undefined) notes.push(`Could not read the checkouts recorded in ${database} (${problem}); their .rerail/sessions folders stay.`);
  for (const checkout of checkouts) {
    const ledger = sessionLedger(checkout);
    if (ledger !== undefined) remove.push({ path: ledger, what: `the session ledgers of ${checkout}`, ledger: true });
  }
  const store = contentStoreOverride();
  const fromMainCache = store === undefined ? "" : relative(mainContentStoreDirectory(), store);
  // A store inside the machine-wide cache folder goes with that folder in `removeInstallation`, under its guard.
  if (store !== undefined && (fromMainCache === ".." || fromMainCache.startsWith(`..${sep}`) || isAbsolute(fromMainCache))) {
    const holder = cacheHolder(dirname(store), machine);
    const files = ["", "-wal", "-shm", "-journal"].map(suffix => `${store}${suffix}`).filter(pathExists);
    if (holder !== undefined && files.length > 0) notes.push(`Kept ${store} (the content cache): another Rerail daemon, pid ${holder}, still uses it.`);
    else for (const path of files) remove.push({ path, what: "the content cache" });
  }
  let userFiles: string[];
  try {
    userFiles = readdirSync(codingAgentHome());
  } catch {
    userFiles = [];
  }
  for (const name of userFiles.filter(entry => entry.startsWith(".rerail-clean-")).sort()) {
    remove.push({ path: join(codingAgentHome(), name), what: "a backup of rules and history" });
  }
  const databases = existsSync(home) ? readdirSync(home).filter(name => HOME_DATABASE.test(name)).sort() : [];
  for (const name of databases) {
    remove.push({ path: join(home, name), what: name.startsWith("harness.") ? "the coding agents' recovery records" : "rules and every repository's history" });
  }
  return { remove, notes };
}

/**
 * `install --remove --delete-data`: stop the daemon, then delete for good what the uninstall otherwise backs up, with
 * the session ledgers in recorded checkouts and every earlier backup (`planDataDeletion`). The checkouts are read
 * after the stop and before the database goes. A checkout's `.rerail` goes too once the ledgers were all it held.
 * Links are removed, never followed (`fs.rm` does not follow them inside a folder either).
 */
async function deleteRulesAndHistory(home: string, dryRun: boolean, machine: DaemonMachine | undefined): Promise<void> {
  if (dryRun) process.stdout.write("Would stop the Rerail daemon.\n");
  else process.stdout.write(await stopDaemon(home) === "stopped" ? "Stopped the Rerail daemon.\n" : "The Rerail daemon was not running.\n");
  const { remove, notes } = planDataDeletion(home, machine);
  if (dryRun) {
    for (const { path, what } of remove) process.stdout.write(`Would delete ${path} (${what}).\n`);
    for (const note of notes) process.stdout.write(`${note}\n`);
    return;
  }
  const failed: Array<{ path: string; reason: string }> = [];
  for (const { path, what, ledger } of remove) {
    try {
      await rm(path, { recursive: true, force: true, ...FOLDER_REMOVAL_RETRIES });
      process.stdout.write(`Deleted ${path} (${what}).\n`);
    } catch (error) {
      failed.push({ path, reason: error instanceof Error ? error.message : String(error) });
      continue;
    }
    // Only an empty `.rerail` goes: anything else in it is not the ledgers.
    if (ledger) await rmdir(dirname(path)).catch(() => undefined);
  }
  if (remove.length === 0) process.stdout.write("No rules, history or backups to delete.\n");
  for (const note of notes) process.stdout.write(`${note}\n`);
  for (const { path, reason } of failed) {
    process.stdout.write(`Could not delete ${path} (${reason}). Close every coding agent and terminal still running Rerail, then run: ${cleanupCommand(path, false)}\n`);
  }
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
    ...(process.platform === "win32" ? [{ path: legacyWindowsContentStoreDirectory(), what: "the cache an earlier release kept" }] : []),
  ];
}

/** Whether `path` is a content cache folder a Rerail daemon of another home may still hold. */
function isContentCache(path: string): boolean {
  return path === mainContentStoreDirectory() || (process.platform === "win32" && path === legacyWindowsContentStoreDirectory());
}

/** What npm and the rule pack store put in the packs prefix, with the lock a pack change holds. */
const RULE_PACK_PREFIX_ENTRIES: ReadonlySet<string> = new Set(["node_modules", "package.json", "package-lock.json", "staging", ".lock"]);

/** In the Rerail home: the person's one approval that Rerail may read their coding agent sessions (T2b). */
export const CONSENT_FILE = "consent.json";

/**
 * What Rerail itself writes in its home, by the name its writer gives it: settings, records, locks, `rerail activate`'s
 * profiles and device secrets, `rules export`'s dump and the installation.
 */
const RERAIL_HOME_ENTRIES: readonly string[] = [
  CONSENT_FILE,
  SEEN_AGENTS_FILE,
  LAUNCHER_PATH_FILE,
  DAEMON_STATE_FILE,
  DAEMON_PORT_FILE,
  DAEMON_ACCESS_TOKEN_FILE,
  START_LOCK_DIR,
  RULES_DUMP_FILE,
  PROFILES_DIR,
  ACTIVE_PROFILE_FILE,
  DEVICE_SECRETS_DIR,
  INSTALLED_PACKS_DIR,
  UPDATE_STATE_FILE,
  UPDATE_LOCK_DIR,
  VERSIONS_DIR,
  CURRENT_LINK,
  LAUNCHER_DIR,
  SCRUB_HINT_FILE,
];

/** What Rerail writes in its home under these names and longer ones: SQLite's `-wal` and `-shm`, `rerail update`'s `.pre-update` copy. */
const RERAIL_HOME_ENTRY_PREFIXES: readonly string[] = [RERAIL_DATABASE_FILE, CLAUDE_STATUS_DATABASE_FILE, DAEMON_LOG_FILE, HOOK_FAILURES_FILE];

/** The `.<pid>[.<uuid>].tmp` or `.old` a write cut short leaves beside what it writes. */
const CUT_SHORT_WRITE = /\.\d+(?:\.[0-9a-f-]+)?\.(?:tmp|old)$/u;

/** Whether `name`, at the top of a Rerail home, is something Rerail wrote there. */
function isRerailHomeEntry(name: string): boolean {
  const written = name.replace(CUT_SHORT_WRITE, "");
  return RERAIL_HOME_ENTRIES.includes(written) || RERAIL_HOME_ENTRY_PREFIXES.some(prefix => written.startsWith(prefix));
}

/**
 * The command that finishes removing `path` by hand. PowerShell: single quotes expand nothing, and `-LiteralPath` reads
 * `[` and `]` as themselves. A link (`current`, a junction on Windows) goes through .NET's non-recursive
 * `Directory.Delete`, which removes the link and never follows it as Windows PowerShell's `Remove-Item -Recurse` does,
 * and, unlike cmd's `rmdir`, expands no `%NAME%` in the path.
 */
export function cleanupCommand(path: string, link: boolean, platform: NodeJS.Platform = process.platform): string {
  if (platform !== "win32") return commandLine("posix", ["rm", "-rf", path]);
  const powershell = powershellLiteral(path);
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

/**
 * Another Rerail daemon (another `RERAIL_HOME`'s) that still holds the machine-wide cache, by pid. On Windows the
 * record outlives a daemon ended at logoff, so its pid counts only while a Rerail engine pipe exists (`daemonMayHold`).
 * `machine` is this one unless a test names another.
 */
export function cacheHolder(cache: string, machine?: DaemonMachine): number | undefined {
  try {
    const { pid } = JSON.parse(readFileSync(join(cache, MAIN_INSTANCE_HOLDER_FILE), "utf8")) as { pid?: unknown };
    return typeof pid === "number" && Number.isInteger(pid) && pid !== process.pid && daemonMayHold(pid, machine) ? pid : undefined;
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
    const holder = isContentCache(path) ? cacheHolder(path, context.machine) : undefined;
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
      else await rm(path, { recursive: true, force: true, ...FOLDER_REMOVAL_RETRIES });
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
  const ours = kept.filter(name => isRerailHomeEntry(name) && !keptForOthers.has(join(context.home, name)));
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
    // Notes about a change come only with one; a note about the agent's state (OMP's plugin install loading Rerail
    // too) holds whether or not this refresh changed anything, so every note is printed.
    printReport({ changes: changed, notes: result.notes }, false);
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
  // After an update to a version that scrubs what it stores: once, the hint to scrub what earlier versions stored.
  const scrubHint = scrubHintOnce(context.home);
  if (scrubHint !== undefined) process.stdout.write(scrubHint);
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

/** What one `rerail install` is to do, checked before anything changes. */
interface InstallPlan {
  readonly parsed: ReturnType<typeof registrationArguments>;
  /** The root `--root` names. */
  readonly explicitRoot: string | undefined;
  readonly root: string;
  readonly remove: boolean;
  readonly deleteData: boolean;
  /** The managed install: `current`, or before any set-up the running package, which is what Pi's gallery entry stands in for. A checkout registered with `--root` never wrote it. */
  readonly managed: boolean;
  /** Removing every coding agent from the managed install is the uninstall. Removing one coding agent, or a checkout registered with `--root`, leaves the shared rules and history in place. */
  readonly uninstall: boolean;
}

async function installRoot(options: ReadonlyMap<string, string>, context: SetupContext, remove: boolean, dryRun: boolean): Promise<Pick<InstallPlan, "explicitRoot" | "root" | "managed">> {
  const rootArgument = options.get("--root");
  const explicitRoot = rootArgument === undefined ? undefined : resolve(rootArgument);
  const root = explicitRoot === undefined
    ? registrationRoot(context)
    : await activateExplicitRoot(context, explicitRoot, remove, dryRun);
  return { explicitRoot, root, managed: explicitRoot === undefined || root === context.updater.currentLink };
}

async function planInstall(flags: ReadonlySet<string>, options: ReadonlyMap<string, string>, context: SetupContext): Promise<InstallPlan> {
  const parsed = registrationArguments(options, flags);
  const remove = flags.has("--remove");
  const deleteData = flags.has("--delete-data");
  if (deleteData && !remove) throw new Error("install --delete-data goes with --remove");
  const located = await installRoot(options, context, remove, parsed.dryRun);
  const uninstall = isUninstall(remove, parsed.hosts, located.managed);
  // Checked before anything changes: the data is everyone's, so only the whole uninstall deletes it.
  if (deleteData && !uninstall) throw new Error("install --delete-data deletes every repository's history, so it only goes with removing Rerail from every coding agent: drop --agents, and --root unless it names the active install");
  return { parsed, remove, deleteData, uninstall, ...located };
}

/** The whole uninstall: the managed install removed from every coding agent, with no `--agents` list narrowing it. */
const isUninstall = (remove: boolean, hosts: readonly Harness[] | undefined, managed: boolean): boolean => remove && hosts === undefined && managed;

/** Registers with or unregisters from the coding agents, and records the agents it saw. */
async function applyInstallPlan({ parsed, root, remove, managed }: InstallPlan, context: SetupContext): Promise<void> {
  process.stdout.write(`${remove ? "Removing Rerail from" : "Connecting Rerail to"} coding agents for ${root}${parsed.dryRun ? " (dry run)" : ""}\n`);
  // Pi's gallery entry goes with the managed install, so its removal takes that entry out too.
  const report = remove
    ? await unregisterHarnesses({ ...registrationOptions(root, codingAgentHome(), parsed), managed })
    : await registerHarnesses(registrationOptions(root, codingAgentHome(), parsed));
  printReport(report, parsed.dryRun);
  // What `rerail install` found or was named is seen, so set-up run again connects no agent behind this choice.
  if (!parsed.dryRun) await recordSeenCodingAgents(context.home, [...detectHarnesses(codingAgentHome()), ...(parsed.hosts ?? [])]);
}

/** What only the uninstall takes out: the data (or a backup of it), the launcher on PATH and the installation. */
async function finishUninstall({ parsed, deleteData }: InstallPlan, context: SetupContext): Promise<void> {
  if (deleteData) await deleteRulesAndHistory(context.home, parsed.dryRun, context.machine);
  else await backUpRulesAndHistory(context.home, parsed.dryRun);
  const binDir = dirname(context.updater.launcherPath);
  const removed = describeLauncherPath(await removeLauncherFromPath({ binDir, home: codingAgentHome(), dryRun: parsed.dryRun }), binDir, "remove", parsed.dryRun);
  if (removed !== undefined) process.stdout.write(`${removed}\n`);
  if (!parsed.dryRun) await rm(launcherPathMarker(context.home), { force: true });
  await removeInstallation(context, parsed.dryRun);
}

/** A checkout registered with `--root` gets the launcher on PATH too, and says what to do next. */
async function finishExplicitRoot({ parsed }: InstallPlan, context: SetupContext): Promise<void> {
  const binDir = dirname(context.updater.launcherPath);
  const change = await addLauncherToPath({ binDir, home: codingAgentHome(), dryRun: parsed.dryRun });
  await recordLauncherPath(context.home, change, parsed.dryRun);
  const pathLine = describeLauncherPath(change, binDir, "add", parsed.dryRun);
  process.stdout.write(["", ...(pathLine === undefined ? [] : [pathLine]), "Next:", "  restart Cursor, Claude Code, Codex, OMP, or Pi so they load the new hooks", ""].join("\n"));
}

async function runRefreshAlone(flags: ReadonlySet<string>, options: ReadonlyMap<string, string>, context: SetupContext): Promise<number> {
  if (flags.size > 1 || options.size > 0) throw new Error("install --refresh takes no other options");
  return await runRefresh(context);
}

export async function runInstallCommand(args: readonly string[], context: SetupContext): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--dry-run", "--remove", "--refresh", "--delete-data"], options: ["--root", "--agents"] },
    argument => `Unknown install option: ${argument}`,
  );
  if (flags.has("--refresh")) return await runRefreshAlone(flags, options, context);
  const plan = await planInstall(flags, options, context);
  await applyInstallPlan(plan, context);
  if (plan.uninstall) await finishUninstall(plan, context);
  if (plan.explicitRoot !== undefined && !plan.remove) await finishExplicitRoot(plan, context);
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
      // A registration Claude Code cannot run records nothing and says nothing: set-up runs the hook as Claude Code
      // will, and one that fails is a failed connection.
      if (agents.includes("claude-code") && !result.failures.some(({ agent }) => agent === "claude-code")) {
        const problem = await runRegisteredClaudeHook(home);
        if (problem !== undefined) result.failures.push({ agent: "claude-code", message: problem });
      }
      // One that could not be connected stays unseen, even if `rerail install` recorded it before, so set-up run again
      // connects it once its config is fixed.
      const failed = result.failures.map(({ agent }) => agent);
      await recordSeenCodingAgents(context.home, [...detectHarnesses(home), ...agents].filter(id => !failed.includes(id)));
      await forgetSeenCodingAgents(context.home, failed);
      return result;
    },
    sessionsApproved() {
      try {
        const consent = JSON.parse(readFileSync(join(context.home, CONSENT_FILE), "utf8")) as { sessions?: { approvedAt?: unknown } };
        return typeof consent.sessions?.approvedAt === "string";
      } catch {
        return false;
      }
    },
    async setSessionsApproved(approved) {
      const path = join(context.home, CONSENT_FILE);
      if (!approved) {
        await rm(path, { force: true });
        return;
      }
      await mkdir(context.home, { recursive: true, mode: 0o700 });
      const consent = { sessions: { approvedAt: new Date().toISOString() } };
      await writeFile(path, `${JSON.stringify(consent, null, 2)}\n`, { mode: 0o600 });
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
  /** `--powershell`: the coding agent runs the answers in PowerShell, so they are written for it. */
  readonly powershell?: boolean;
}

/**
 * The on-off flags a set-up run carries into every answer it prints, each with its `SetupCarry` field: one row per flag.
 * Every parser of `rerail setup` and `rerail start` takes them (`CARRIED_FLAG_NAMES`), `carriedFlags` reads them, and
 * `carriedArgs` writes them back. `--relay` is not a row: it picks the agent lane (`setupIo`), whose answers carry it.
 */
const CARRIED_FLAGS = [
  { field: "powershell", flag: SETUP_FLAGS.powershell },
  { field: "repository", flag: "--repository" },
] as const satisfies readonly { field: keyof SetupCarry; flag: string }[];

type CarriedFlag = (typeof CARRIED_FLAGS)[number];

/** The carried on-off flags that are on, as their `SetupCarry` fields. */
export type CarriedFlags = { [Field in CarriedFlag["field"]]?: true };

/** The flags `CARRIED_FLAGS` names, for a parser's list of the flags it takes. */
export const CARRIED_FLAG_NAMES: readonly string[] = CARRIED_FLAGS.map(({ flag }) => flag);

/** The carried flags `isOn` says are on, read from a parsed command line (by `flag`) or from a carry (by `field`). */
export function carriedFlags(isOn: (row: CarriedFlag) => boolean): CarriedFlags {
  const on: CarriedFlags = {};
  for (const row of CARRIED_FLAGS) if (isOn(row)) on[row.field] = true;
  return on;
}

/** The carried on-off flags `carry` has on, as arguments. */
function carriedFlagArgs(carry: SetupCarry): string[] {
  return CARRIED_FLAGS.filter(({ field }) => carry[field] === true).map(({ flag }) => flag);
}

function carriedArgs(carry: SetupCarry): string[] {
  return [
    ...(carry.relay === true ? [SETUP_FLAGS.relay] : []),
    ...(carry.port === undefined ? [] : ["--port", String(carry.port)]),
    ...(carry.databasePath === undefined ? [] : ["--database", carry.databasePath]),
    ...carriedFlagArgs(carry),
    // Ids, as `parseCodingAgents` reads them; an answer's own `--agents` comes later and wins.
    ...(carry.agents === undefined ? [] : ["--agents", carry.agents.join(",")]),
  ];
}

/**
 * What starts an answer in `shell`, from what set-up wrote, which stays put between answers (an `npx` copy may not);
 * undefined until it is there. Never `rerail.cmd`: cmd.exe reads a batch file's arguments again, so the `&` in a folder
 * named `C:\code\R&D` would end the command and run the rest, and a `%NAME%` would expand. A POSIX shell gets the sh
 * launcher, which Git Bash runs on Windows; PowerShell, there for Codex and Cursor on Windows, gets node and the CLI.
 */
function answerLauncher(updater: ManagedUpdater, shell: Shell): string[] | undefined {
  const script = shell === "powershell" ? join(updater.currentLink, ENTRY_POINTS.cli) : join(dirname(updater.launcherPath), "rerail");
  if (!existsSync(script)) return undefined;
  return shell === "powershell" ? [pinnedNodePath(), script] : [script];
}

/**
 * What the coding agent runs to answer a set-up question: the launcher (`answerLauncher`) with what this run carries,
 * for the shell the agent runs it in: PowerShell when the run carries `--powershell`, else a POSIX shell, Git Bash on
 * Windows (`windowsCommandShell`). `rerail setup` until the launcher exists.
 */
export function setupAnswerCommand(context: SetupContext, carry: SetupCarry = {}): SetupCommand | undefined {
  const args = carriedArgs(carry);
  const shell = carry.powershell === true ? "powershell" : "posix";
  const launcher = answerLauncher(context.updater, shell);
  if (launcher === undefined && args.length === 0) return undefined;
  return { words: [...(launcher ?? ["rerail"]), "setup", ...args], shell };
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
    { flags: [JSON_FLAG, SETUP_FLAGS.relay, ...CARRIED_FLAG_NAMES], options: ["--answer", "--repo", "--agents", SETUP_FLAGS.packs, "--port", "--database"] },
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
    ...carriedFlags(({ flag }) => flags.has(flag)),
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
    ...carriedFlags(({ field }) => parsed[field] === true),
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
      { flags: [JSON_FLAG, SETUP_FLAGS.relay, ...CARRIED_FLAG_NAMES], options: ["--repo", "--agents", SETUP_FLAGS.packs, "--answer", "--database"] },
      argument => `Unknown start option: ${argument}`,
    );
    const agents = options.get("--agents");
    const database = options.get("--database");
    const databasePath = database === undefined ? place.databasePath : resolve(database);
    const carried = carriedFlags(({ flag }) => flags.has(flag));
    const answer = checkSetupAnswer(options.get("--answer"));
    const workspace = resolve(place.cwd, options.get("--repo") ?? ".");
    // A session opened in the home folder has no repository to start: the set-up asks which project (E3).
    if (workspace === resolve(homedir())) {
      const passed = [
        ...(answer === undefined ? [] : ["--answer", answer]),
        ...(agents === undefined ? [] : ["--agents", agents]),
        ...(options.has(SETUP_FLAGS.packs) ? [SETUP_FLAGS.packs, options.get(SETUP_FLAGS.packs)!] : []),
        ...(databasePath === undefined ? [] : ["--database", databasePath]),
        ...carriedFlagArgs(carried),
      ];
      return await jsonSetupStep([JSON_FLAG, ...passed], context, { ...place, cwd: workspace }, process.versions.node, "start");
    }
    const { agentsError, ...chosen } = setupAgents(agents, answer);
    const carry: SetupCarry = {
      ...(databasePath === undefined ? {} : { databasePath }),
      ...carried,
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
