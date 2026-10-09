// Shared by `just rerail --restore` and `just rerail-dev --restore`: pick a `just nuke-rerail` backup and put its
// database back behind a stopped daemon, never pairing a restored main file with a stale -wal/-shm.
import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, statSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export const BACKUP_PREFIX = ".rerail-clean-";
/** Where a backup keeps the Rerail home it took; `rerail install --remove` uses the same name. */
export const BACKUP_RERAIL_HOME = "rerail-home";
/** The one Rerail database: the rule catalog and every repository's history. harness.sqlite belongs to the fresh install. */
export const RERAIL_DATABASE = "rerail.sqlite";
/**
 * The home directory's own workspace state: the non-Git workspace identity that scopes its rules, and its session
 * ledgers. Both live in `~/.rerail`, so they are restored only into that home.
 */
export const RESTORED_HOME_FILES = ["repository-id"];
export const RESTORED_HOME_DIRECTORIES = ["sessions"];
/** Advisory lock files of live coding-agent sessions; a restored copy would describe processes that no longer exist. */
const TRANSIENT_SESSION_ENTRIES = new Set(["claude-code-locks"]);
const SQLITE_SIDECARS = ["-wal", "-shm", "-journal"];

export class RestoreError extends Error {}

/** `$RERAIL_HOME`, or `~/.rerail` under `home`. */
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
    if (argument !== "--restore" && !argument.startsWith("--restore=")) {
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
export function selectBackup({ userHome, backupDir }) {
  if (backupDir !== undefined) {
    const chosen = resolve(backupDir);
    if (!isDirectory(join(chosen, BACKUP_RERAIL_HOME))) {
      throw new RestoreError(`${chosen} is not a just nuke-rerail backup: it has no ${BACKUP_RERAIL_HOME}/ directory`);
    }
    return chosen;
  }
  let names;
  try {
    names = readdirSync(userHome);
  } catch {
    names = [];
  }
  const candidates = names
    .filter(name => name.startsWith(BACKUP_PREFIX) && isDirectory(join(userHome, name, BACKUP_RERAIL_HOME)))
    .sort();
  if (candidates.length === 0) {
    throw new RestoreError(`No Rerail backup found: no ${join(userHome, `${BACKUP_PREFIX}*`)} directory has a `
      + `${BACKUP_RERAIL_HOME}/. Pass the backup directory: --restore <dir>`);
  }
  return join(userHome, candidates.at(-1));
}

/** node:sqlite prints an ExperimentalWarning on load under Node 24; these scripts' output has no use for it. */
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

/** Rows in the rule library, or undefined when the database has no `library_rules` table. */
export async function countRules(databasePath) {
  const { DatabaseSync } = await loadSqlite();
  const database = new DatabaseSync(databasePath, { readOnly: true });
  try {
    const found = database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'library_rules'").get();
    if (found === undefined) return undefined;
    return { table: "library_rules", count: Number(database.prepare("SELECT count(*) AS n FROM library_rules").get().n) };
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

/**
 * Replace `<destDir>/<name>` with the consolidated `staged` file. The destination's main file and every sidecar move
 * aside together first, so no -wal or -shm from the fresh install can be replayed onto the restored database.
 */
export function installDatabase(staged, destDir, name, stamp) {
  mkdirSync(destDir, { recursive: true, mode: 0o700 });
  const target = join(destDir, name);
  const movedAside = [target, ...SQLITE_SIDECARS.map(sidecar => `${target}${sidecar}`)]
    .map(path => moveAside(path, stamp))
    .filter(path => path !== undefined);
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

/** Stage the backup's rerail.sqlite as one consolidated file; the caller removes `stagingDir`. */
export async function stageDatabase(backupDir, stagingParent = tmpdir()) {
  const source = join(backupDir, BACKUP_RERAIL_HOME);
  if (!existsSync(join(source, RERAIL_DATABASE))) throw new RestoreError(`${source} holds no ${RERAIL_DATABASE}; nothing to restore`);
  mkdirSync(stagingParent, { recursive: true });
  const stagingDir = mkdtempSync(join(stagingParent, "rerail-restore-"));
  try {
    return { stagingDir, staged: await consolidateDatabase(source, RERAIL_DATABASE, stagingDir) };
  } catch (error) {
    rmSync(stagingDir, { recursive: true, force: true });
    throw error;
  }
}

/**
 * Copy the staged database into a stopped Rerail home, plus the home directory's workspace identity and session
 * ledgers when `home` is the user's `~/.rerail`, where that workspace keeps them.
 */
export function installIntoHome({ backupDir, staged, home, stamp, includeHomeWorkspace }) {
  const results = [installDatabase(staged, home, RERAIL_DATABASE, stamp)];
  if (!includeHomeWorkspace) return results;
  const sourceHome = join(backupDir, BACKUP_RERAIL_HOME);
  for (const name of RESTORED_HOME_FILES) {
    const source = join(sourceHome, name);
    if (existsSync(source)) results.push(installFile(source, join(home, name), stamp));
  }
  for (const name of RESTORED_HOME_DIRECTORIES) {
    const source = join(sourceHome, name);
    if (isDirectory(source)) results.push(installDirectory(source, join(home, name), stamp));
  }
  return results;
}

/** The daemon state `rerail daemon start` records in `<home>/daemon.json`, or undefined. */
export function readDaemonState(home) {
  try {
    const state = JSON.parse(readFileSync(join(home, "daemon.json"), "utf8"));
    return typeof state === "object" && state !== null ? state : undefined;
  } catch {
    return undefined;
  }
}

export function processAlive(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

export function launcherPath(home) {
  return join(home, "bin", process.platform === "win32" ? "rerail.cmd" : "rerail");
}

/** Run the home's installed launcher with `RERAIL_HOME` pinned to it and autostart off. */
export function runLauncher(home, args, { env = process.env, timeout = 120_000 } = {}) {
  return spawnSync(launcherPath(home), args, {
    env: { ...env, RERAIL_HOME: home, RERAIL_AUTOSTART: "0" },
    encoding: "utf8",
    timeout,
    shell: process.platform === "win32",
  });
}

/**
 * Stop the daemon that owns `home`: its own `rerail daemon stop` when a launcher is installed, else SIGTERM the pid in
 * daemon.json, wait up to `timeoutMs`, then SIGKILL. Nothing running is not an error.
 * Returns "launcher", "pid", or "not-running".
 */
export async function stopDaemon(home, { env = process.env, timeoutMs = 10_000, log = () => {} } = {}) {
  if (existsSync(launcherPath(home))) {
    const result = runLauncher(home, ["daemon", "stop"], { env, timeout: 30_000 });
    const command = `${launcherPath(home)} daemon stop`;
    if (result.status === 0) {
      log(`${command}: ${(result.stdout ?? "").trim() || "done"}`);
      if (!processAlive(readDaemonState(home)?.pid)) return "launcher";
    } else {
      log(`${command} failed (${(result.stderr ?? "").trim() || `exit ${String(result.status)}`}); stopping by pid`);
    }
  }
  const pid = readDaemonState(home)?.pid;
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

/**
 * Restore a `just nuke-rerail` backup's database into the Rerail home `home`: the newest `~/.rerail-clean-*` under
 * `userHome` unless `backupDir` names one. Stops the daemon that owns `home` first and leaves it stopped; the
 * caller restarts it. Everything replaced is kept beside it as `*.pre-restore-<stamp>`; the backup is never modified.
 * `dryRun` only selects and validates the backup.
 */
export async function restoreBackup({ backupDir, home, dryRun = false, userHome = process.env.HOME ?? homedir(), env = process.env, log = () => {} }) {
  const chosen = selectBackup({ userHome, backupDir });
  const source = join(chosen, BACKUP_RERAIL_HOME, RERAIL_DATABASE);
  if (!existsSync(source)) throw new RestoreError(`${dirname(source)} holds no ${RERAIL_DATABASE}; nothing to restore`);
  const target = resolve(home);
  if (dryRun) {
    log(`Would stop the daemon of ${target} and restore ${source} into it`);
    return { backupDir: chosen, home: target, restored: [], rules: undefined, dryRun: true };
  }
  const stamp = timestamp();
  const { stagingDir, staged } = await stageDatabase(chosen);
  try {
    const rules = await countRules(staged);
    log(`Stopping the Rerail daemon of ${target}`);
    await stopDaemon(target, { env, log });
    const restored = installIntoHome({
      backupDir: chosen,
      staged,
      home: target,
      stamp,
      includeHomeWorkspace: target === resolve(userHome, ".rerail"),
    });
    for (const result of restored) {
      log(`Restored ${result.target}`);
      for (const aside of result.movedAside) log(`  kept the previous copy as ${aside}`);
    }
    return { backupDir: chosen, home: target, restored, rules };
  } finally {
    rmSync(stagingDir, { recursive: true, force: true });
  }
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
 * `just rerail --restore`: after `rerail setup` succeeded, restore the backup's database into the installed Rerail
 * home, restart the daemon on the port it served, and verify /healthz and the rules in rerail.sqlite.
 */
export async function restoreInstalled({ backupDir, env = process.env, log = console.log }) {
  const userHome = env.HOME ?? homedir();
  const home = rerailHomeFrom(env, userHome);
  const launcher = launcherPath(home);
  if (!existsSync(launcher)) throw new RestoreError(`${launcher} is missing; rerail setup did not complete, so nothing was restored`);
  const before = readDaemonState(home);
  const { rules } = await restoreBackup({ backupDir, home, userHome, env, log });
  const args = restartArguments(before);
  log(`Restarting: rerail ${args.join(" ")}`);
  const started = runLauncher(home, args, { env });
  if (started.status !== 0) {
    throw new RestoreError(`rerail ${args.join(" ")} failed: ${(started.stderr || started.stdout || "").trim()}`);
  }
  const after = readDaemonState(home);
  const health = typeof after?.url === "string" ? await daemonHealth(after.url) : undefined;
  if (health === undefined) throw new RestoreError(`The restarted daemon did not answer /healthz at ${String(after?.url)}`);
  log(`Daemon healthy at ${after.url}`);
  const database = join(home, RERAIL_DATABASE);
  const now = existsSync(database) ? await countRules(database) : undefined;
  if (now !== undefined) log(`${RERAIL_DATABASE}: ${now.count} rows in ${now.table} (the backup held ${rules?.count ?? 0})`);
  return { home, url: after.url, rules: now };
}
