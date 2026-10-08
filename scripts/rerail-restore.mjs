// Shared by `just rerail --restore` and `just rerail-dev --restore`: pick a `just nuke-rerail` backup and put its
// databases back behind a stopped daemon, never pairing a restored main file with a stale -wal/-shm.
import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, statSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export const BACKUP_PREFIX = ".rerail-clean-";
export const BACKUP_RERAIL_HOME = "rerail-home";
/** The one Rerail database: the rule catalog and every repository's history. harness.sqlite belongs to the fresh install. */
export const RERAIL_DATABASE = "rerail.sqlite";
/**
 * A backup taken before the single database holds the machine rule catalog here instead. It is restored as it was,
 * with no rerail.sqlite beside it, so the next daemon start copies it into a new rerail.sqlite. The backup's
 * transcripts.sqlite is not restored: the single database never imports checkout history.
 */
export const LEGACY_RULE_CATALOG = "rules.sqlite";
/** The non-Git home workspace identity that scopes its rules, and that workspace's session ledgers. */
export const RESTORED_HOME_FILES = ["repository-id"];
export const RESTORED_HOME_DIRECTORIES = ["sessions"];
/** Advisory lock files of live coding-agent sessions; a restored copy would describe processes that no longer exist. */
const TRANSIENT_SESSION_ENTRIES = new Set(["claude-code-locks"]);
const SQLITE_SIDECARS = ["-wal", "-shm", "-journal"];

export class RestoreError extends Error {}

export function rerailHomeFrom(env = process.env, home = env.HOME ?? homedir()) {
  return env.RERAIL_HOME === undefined || env.RERAIL_HOME === "" ? join(home, ".rerail") : resolve(env.RERAIL_HOME);
}

/**
 * Pull `--restore [dir]` / `--restore=<dir>` out of an argument list. A following argument is the backup directory
 * only when it does not look like a flag; everything else is returned untouched for the wrapped command.
 */
export function parseRestoreArguments(argv) {
  const args = [];
  let restore;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!isRestoreFlag(argument)) {
      args.push(argument);
      continue;
    }
    if (restore !== undefined) throw new RestoreError("--restore may be given once");
    const option = readRestoreFlag(argument, argv[index + 1]);
    restore = option.restore;
    index += option.consumed;
  }
  return { restore, args };
}

function isRestoreFlag(argument) {
  return argument === "--restore" || argument.startsWith("--restore=");
}

/** One `--restore` flag, and whether it consumed `next` as its directory. */
function readRestoreFlag(argument, next) {
  if (argument !== "--restore") {
    const backupDir = argument.slice("--restore=".length);
    if (backupDir === "") throw new RestoreError("--restore= needs a backup directory");
    return { restore: { backupDir }, consumed: 0 };
  }
  if (next !== undefined && !next.startsWith("-")) return { restore: { backupDir: next }, consumed: 1 };
  return { restore: {}, consumed: 0 };
}

function isDirectory(path) {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

/** The named backup, or the newest `~/.rerail-clean-*` holding a rerail-home/ (names sort by date, then time). */
export function selectBackup({ home, backupDir }) {
  if (backupDir !== undefined) {
    const chosen = resolve(backupDir);
    if (!isDirectory(join(chosen, BACKUP_RERAIL_HOME))) {
      throw new RestoreError(`${chosen} is not a just nuke-rerail backup: it has no ${BACKUP_RERAIL_HOME}/ directory`);
    }
    return chosen;
  }
  let names;
  try {
    names = readdirSync(home);
  } catch {
    names = [];
  }
  const candidates = names
    .filter(name => name.startsWith(BACKUP_PREFIX) && isDirectory(join(home, name, BACKUP_RERAIL_HOME)))
    .sort();
  if (candidates.length === 0) {
    const pattern = join(home, `${BACKUP_PREFIX}*`);
    throw new RestoreError(`No Rerail backup found: no ${pattern} directory has a ${BACKUP_RERAIL_HOME}/. `
      + "Pass the backup directory: --restore <dir>");
  }
  return join(home, candidates.at(-1));
}

/** node:sqlite prints an ExperimentalWarning on load under Node 24; this script's output has no use for it. */
export async function loadSqlite() {
  const emitWarning = process.emitWarning;
  process.emitWarning = (warning, ...rest) => {
    if (String(warning).includes("SQLite")) return;
    emitWarning.call(process, warning, ...rest);
  };
  try {
    return await import("node:sqlite");
  } finally {
    process.emitWarning = emitWarning;
  }
}

export function timestamp(now = new Date()) {
  const pad = value => String(value).padStart(2, "0");
  return `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
}

/**
 * Copy a database and its WAL into `stagingDir`, fold the WAL into the main file, and return that one self-contained
 * file. The backup itself is never opened for writing. The -shm is left behind: SQLite rebuilds it from the WAL.
 */
export async function consolidateDatabase(sourceDir, name, stagingDir) {
  const source = join(sourceDir, name);
  const staged = join(stagingDir, name);
  copyFileSync(source, staged);
  if (existsSync(`${source}-wal`)) copyFileSync(`${source}-wal`, `${staged}-wal`);
  const { DatabaseSync } = await loadSqlite();
  const database = new DatabaseSync(staged);
  try {
    database.exec("PRAGMA wal_checkpoint(TRUNCATE)");
  } finally {
    database.close();
  }
  for (const sidecar of SQLITE_SIDECARS) rmSync(`${staged}${sidecar}`, { force: true });
  return staged;
}

/** The database a backup restores: its rerail.sqlite, or the legacy rule catalog of a backup that predates it. */
export function restoredDatabase(backupDir) {
  const source = join(backupDir, BACKUP_RERAIL_HOME);
  if (existsSync(join(source, RERAIL_DATABASE))) return RERAIL_DATABASE;
  if (existsSync(join(source, LEGACY_RULE_CATALOG))) return LEGACY_RULE_CATALOG;
  throw new RestoreError(`${source} holds neither ${RERAIL_DATABASE} nor ${LEGACY_RULE_CATALOG}; nothing to restore`);
}

/** Rows in the rule catalog (`catalog_rules`, or the legacy `rules` table), or undefined when neither exists. */
export async function countRules(databasePath) {
  const { DatabaseSync } = await loadSqlite();
  const database = new DatabaseSync(databasePath, { readOnly: true });
  try {
    for (const table of ["catalog_rules", "rules"]) {
      const found = database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?").get(table);
      if (found !== undefined) return { table, count: Number(database.prepare(`SELECT count(*) AS n FROM "${table}"`).get().n) };
    }
    return undefined;
  } finally {
    database.close();
  }
}

function exists(path) {
  try {
    lstatSync(path);
    return true;
  } catch {
    return false;
  }
}

/** Move `path` to `<path>.pre-restore-<stamp>` if anything is there; returns the new name. */
export function moveAside(path, stamp) {
  if (!exists(path)) return undefined;
  const aside = `${path}.pre-restore-${stamp}`;
  renameSync(path, aside);
  return aside;
}

/** Move `<destDir>/<name>` and every sidecar aside together; returns the new names. */
function moveDatabaseAside(destDir, name, stamp) {
  const target = join(destDir, name);
  return [target, ...SQLITE_SIDECARS.map(sidecar => `${target}${sidecar}`)]
    .map(path => moveAside(path, stamp))
    .filter(path => path !== undefined);
}

/**
 * Replace `<destDir>/<name>` with the consolidated `staged` file. The destination's main file and every sidecar move
 * aside together first, so no -wal or -shm from the fresh install can be replayed onto the restored database.
 */
export function installDatabase(staged, destDir, name, stamp) {
  mkdirSync(destDir, { recursive: true, mode: 0o700 });
  const target = join(destDir, name);
  const movedAside = moveDatabaseAside(destDir, name, stamp);
  const pending = join(destDir, `.${name}.restore-${stamp}`);
  copyFileSync(staged, pending);
  chmodSync(pending, 0o600);
  renameSync(pending, target);
  return { target, movedAside };
}

function installFile(source, target, stamp) {
  const aside = moveAside(target, stamp);
  const pending = join(dirname(target), `.${basename(target)}.restore-${stamp}`);
  copyFileSync(source, pending);
  chmodSync(pending, 0o600);
  renameSync(pending, target);
  return { target, movedAside: aside === undefined ? [] : [aside] };
}

function installDirectory(source, target, stamp) {
  const aside = moveAside(target, stamp);
  cpSync(source, target, {
    recursive: true,
    verbatimSymlinks: true,
    preserveTimestamps: true,
    filter: path => !TRANSIENT_SESSION_ENTRIES.has(basename(path)) || path === source,
  });
  return { target, movedAside: aside === undefined ? [] : [aside] };
}

/**
 * Stage the backup's database from `<backup>/rerail-home` as one consolidated file. Returns the staging directory
 * (the caller removes it) and the staged `{ name, path }`.
 */
export async function stageDatabase(backupDir, stagingParent = tmpdir()) {
  const name = restoredDatabase(backupDir);
  mkdirSync(stagingParent, { recursive: true });
  const stagingDir = mkdtempSync(join(stagingParent, "rerail-restore-"));
  try {
    const path = await consolidateDatabase(join(backupDir, BACKUP_RERAIL_HOME), name, stagingDir);
    return { stagingDir, staged: { name, path } };
  } catch (error) {
    rmSync(stagingDir, { recursive: true, force: true });
    throw error;
  }
}

/**
 * Copy the staged database plus the home workspace's identity and session ledgers into a stopped Rerail home. The
 * fresh install's rerail.sqlite always moves aside: either the backup's copy replaces it, or a restored legacy
 * catalog must find none so the next start adopts it.
 */
export function installIntoRerailHome({ backupDir, staged, rerailHome, stamp }) {
  const results = [];
  if (staged.name !== RERAIL_DATABASE) {
    const displaced = moveDatabaseAside(rerailHome, RERAIL_DATABASE, stamp);
    if (displaced.length > 0) results.push({ target: join(rerailHome, RERAIL_DATABASE), movedAside: displaced, displaced: true });
  }
  results.push(installDatabase(staged.path, rerailHome, staged.name, stamp));
  const sourceHome = join(backupDir, BACKUP_RERAIL_HOME);
  for (const name of RESTORED_HOME_FILES) {
    const source = join(sourceHome, name);
    if (existsSync(source)) results.push(installFile(source, join(rerailHome, name), stamp));
  }
  for (const name of RESTORED_HOME_DIRECTORIES) {
    const source = join(sourceHome, name);
    if (isDirectory(source)) results.push(installDirectory(source, join(rerailHome, name), stamp));
  }
  return results;
}

export function readDaemonState(rerailHome) {
  try {
    const state = JSON.parse(readFileSync(join(rerailHome, "daemon.json"), "utf8"));
    return typeof state === "object" && state !== null ? state : undefined;
  } catch {
    return undefined;
  }
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

export function launcherPath(rerailHome) {
  return join(rerailHome, "bin", process.platform === "win32" ? "rerail.cmd" : "rerail");
}

/**
 * The installed launcher and the arguments that stop its daemon: `rerail daemon stop`. An old `rerail` launcher
 * is never asked: once `current` points at this version its `stop` would stop a repository, not the daemon, so
 * such installs are stopped by pid. Undefined when no `rerail` launcher is installed.
 */
export function installedStopCommand(rerailHome) {
  const launcher = launcherPath(rerailHome);
  return existsSync(launcher) ? { launcher, args: ["daemon", "stop"] } : undefined;
}

/**
 * Stop the daemon that owns `rerailHome`: its own `rerail daemon stop` when installed, else SIGTERM the pid in
 * daemon.json, wait up to `timeoutMs`, then SIGKILL. Nothing
 * running is not an error.
 */
export async function stopDaemon(rerailHome, { env = process.env, timeoutMs = 10_000, log = () => {} } = {}) {
  const installed = installedStopCommand(rerailHome);
  if (installed !== undefined) {
    const { launcher, args } = installed;
    const command = `${launcher} ${args.join(" ")}`;
    const result = spawnSync(launcher, args, { env, encoding: "utf8", timeout: 30_000, shell: process.platform === "win32" });
    if (result.status === 0) {
      log(`${command}: ${(result.stdout ?? "").trim() || "done"}`);
      const pid = readDaemonState(rerailHome)?.pid;
      if (!processAlive(pid)) return "launcher";
    } else {
      const reason = (result.stderr ?? "").trim() || `exit ${String(result.status)}`;
      log(`${command} failed (${reason}); stopping by pid`);
    }
  }
  const pid = readDaemonState(rerailHome)?.pid;
  if (!processAlive(pid)) return "not-running";
  process.kill(pid, "SIGTERM");
  const deadline = Date.now() + timeoutMs;
  while (processAlive(pid) && Date.now() < deadline) await sleep(100); // NOSONAR: polling until the pid exits
  if (processAlive(pid)) {
    process.kill(pid, "SIGKILL");
    log(`Rerail pid ${pid} ignored SIGTERM for ${timeoutMs / 1000}s; sent SIGKILL`);
  }
  return "pid";
}

/** `rerail daemon start` arguments that bring the daemon back on the port it served before the restore. */
export function restartArguments(state) {
  const args = ["daemon", "start"];
  if (Number.isSafeInteger(state?.port) && state.port > 0) args.push("--port", String(state.port));
  return args;
}

export async function daemonHealth(url, fetchImpl = globalThis.fetch) {
  try {
    const response = await fetchImpl(new URL("/healthz", url), { signal: AbortSignal.timeout(5_000) });
    if (!response.ok) return undefined;
    const body = await response.json();
    return body?.status === "ok" ? body : undefined;
  } catch {
    return undefined;
  }
}

/**
 * `just rerail --restore`: after `rerail setup` succeeded, stop its daemon, restore the backup's database into the
 * Rerail home, restart the daemon on the same port, and verify /healthz and the rules in rerail.sqlite.
 */
export async function restoreInstalled({ backupDir, env = process.env, log = console.log }) {
  const home = env.HOME ?? homedir();
  const rerailHome = rerailHomeFrom(env, home);
  const launcher = launcherPath(rerailHome);
  if (!existsSync(launcher)) throw new RestoreError(`${launcher} is missing; rerail setup did not complete, so nothing was restored`);
  const stamp = timestamp();
  const { stagingDir, staged } = await stageDatabase(backupDir);
  try {
    const before = readDaemonState(rerailHome);
    log(`Stopping the Rerail daemon (${rerailHome})`);
    await stopDaemon(rerailHome, { env, log });
    for (const result of installIntoRerailHome({ backupDir, staged, rerailHome, stamp })) {
      log(result.displaced === true
        ? `Moved ${result.target} aside; the next start copies the restored ${LEGACY_RULE_CATALOG} into it`
        : `Restored ${result.target}`);
      for (const aside of result.movedAside) log(`  kept the fresh install's copy as ${aside}`);
    }
    const args = restartArguments(before);
    log(`Restarting: rerail ${args.join(" ")}`);
    const started = spawnSync(launcher, args, { env, encoding: "utf8", timeout: 120_000, shell: process.platform === "win32" });
    if (started.status !== 0) {
      throw new RestoreError(`rerail ${args.join(" ")} failed: ${(started.stderr || started.stdout || "").trim()}`);
    }
    const after = readDaemonState(rerailHome);
    const health = typeof after?.url === "string" ? await daemonHealth(after.url) : undefined;
    if (health === undefined) throw new RestoreError(`The restarted daemon did not answer /healthz at ${String(after?.url)}`);
    log(`Daemon healthy at ${after.url}`);
    const database = join(rerailHome, RERAIL_DATABASE);
    if (!existsSync(database)) throw new RestoreError(`The restarted daemon created no ${database}`);
    const rules = await countRules(database);
    if (rules !== undefined) log(`${RERAIL_DATABASE}: ${rules.count} rows in ${rules.table}`);
    return { rerailHome, url: after.url, rules };
  } finally {
    rmSync(stagingDir, { recursive: true, force: true });
  }
}

/**
 * `just rerail-dev --restore`: with this checkout's Rerail container healthy, stage the backup's database under
 * `.nera/` and let ops.sh stop the container, swap it into its volume (moving the container's rerail.sqlite aside), and
 * start it again.
 */
export async function restoreContainer({ backupDir, repoRoot = REPO_ROOT, log = console.log, runOps }) {
  const stamp = timestamp();
  const { stagingDir, staged } = await stageDatabase(backupDir, join(repoRoot, ".nera"));
  try {
    log(`==> Restoring ${staged.name} from ${backupDir} into this checkout's Rerail container`);
    const opsArgs = ["rerail-restore", stagingDir, stamp, staged.name];
    const status = runOps === undefined
      ? spawnSync("/bin/bash", [join(repoRoot, "scripts", "ops.sh"), ...opsArgs], { cwd: repoRoot, stdio: "inherit" }).status ?? 1
      : await runOps(opsArgs);
    if (status !== 0) throw new RestoreError(`ops.sh rerail-restore exited ${status}; the container's previous database was kept aside`);
    const rules = await countRules(staged.path);
    if (rules !== undefined) log(`==> ${staged.name}: restored ${rules.count} rows in ${rules.table}`);
    return rules === undefined ? undefined : { name: staged.name, ...rules };
  } finally {
    rmSync(stagingDir, { recursive: true, force: true });
  }
}

/** Process interface for Rerail's rerail-dev.mjs, which may not import Landing code: `select [dir]` | `container <dir>`. */
export async function cli(argv, { env = process.env } = {}) {
  const [command, ...rest] = argv;
  if (command === "select" && rest.length <= 1) {
    process.stdout.write(`${selectBackup({ home: env.HOME ?? homedir(), backupDir: rest[0] })}\n`);
    return 0;
  }
  if (command === "container" && rest.length === 1) {
    await restoreContainer({ backupDir: selectBackup({ home: env.HOME ?? homedir(), backupDir: rest[0] }) });
    return 0;
  }
  throw new RestoreError("usage: rerail-restore.mjs select [backup-dir] | container <backup-dir>");
}

if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) {
  try {
    process.exitCode = await cli(process.argv.slice(2));
  } catch (error) {
    if (!(error instanceof RestoreError)) throw error;
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
