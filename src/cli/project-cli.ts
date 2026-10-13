import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import Database from "better-sqlite3";
import { loadBundledCatalog } from "../engine/catalog/bundled-rules.js";
import type { UpdateResult, UpdateStatus } from "../setup/updater.js";
import { parseArguments } from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase, resetRepositoryHistory } from "../engine/storage/database.js";
import { RERAIL_SCHEMA_VERSION } from "../engine/storage/migrations.js";
import { parseJsonWithoutDuplicateKeys } from "../engine/json-duplicate-keys.js";
import { listRepositories, lookupCheckout } from "../engine/catalog/repository-registry.js";
import { defaultRulesDumpPath, exportRulesDump, importRulesDump } from "../engine/catalog/rule-bootstrap.js";

/** Where `rerail update` leaves a consistent copy of the Rerail database before it switches versions. */
export function preUpdateSnapshotPath(databasePath: string): string {
  return `${databasePath}.pre-update`;
}

/** The version change a pre-update copy was taken for, recorded beside it as `rerail.sqlite.pre-update.json`. */
interface SnapshotTransition {
  from: string;
  to: string;
  /** The highest schema floor `from` opens: the floor of the version that ran the update. Absent in older records. */
  fromReads?: number;
}

/** What `rerail update` and `rerail update rollback` ask of the managed updater. */
export interface SnapshotUpdater {
  status(): Promise<UpdateStatus>;
  update(channel?: string): Promise<UpdateResult>;
  rollback(): Promise<UpdateResult>;
}

/**
 * A consistent copy of the whole database (every project's rules and every repository's history) beside the
 * current copy, or undefined when there is no database or the copy failed. A copy that fails is reported, never
 * fatal; the update must not be blocked by a database the new version may be the fix for.
 */
function stageSnapshot(databasePath: string): string | undefined {
  if (!existsSync(databasePath)) return undefined;
  const staging = `${preUpdateSnapshotPath(databasePath)}.${process.pid}.tmp`;
  try {
    rmSync(staging, { force: true });
    const source = new Database(databasePath, { readonly: true, fileMustExist: true });
    try {
      source.pragma("busy_timeout = 5000");
      source.prepare("VACUUM INTO ?").run(staging);
    } finally {
      source.close();
    }
    return staging;
  } catch (error) {
    rmSync(staging, { force: true });
    process.stderr.write(`rerail: could not copy ${databasePath} before the update: ${(error as Error).message}\n`);
    return undefined;
  }
}

/**
 * `rerail update`: copy the database, then update. Only an update that changes the version replaces
 * `rerail.sqlite.pre-update`, and the copy records that change and the schema floor this version opens (`fromReads`,
 * this build's own); an update that finds nothing newer, or fails, leaves the copy of the last real change in place, so
 * rolling that change back still finds it.
 */
export async function updateWithSnapshot(
  updater: SnapshotUpdater,
  databasePath: string,
  channel: string | undefined,
  fromReads: number = RERAIL_SCHEMA_VERSION,
): Promise<UpdateResult> {
  const from = (await updater.status()).current.version;
  const staged = stageSnapshot(databasePath);
  let result: UpdateResult;
  try {
    result = await updater.update(channel);
  } catch (error) {
    if (staged !== undefined) rmSync(staged, { force: true });
    throw error;
  }
  if (staged === undefined) return result;
  if (!result.changed || result.current.version === from) {
    rmSync(staged, { force: true });
    return result;
  }
  const target = preUpdateSnapshotPath(databasePath);
  const transition: SnapshotTransition = { from, to: result.current.version, fromReads };
  writeFileSync(`${target}.json.tmp`, `${JSON.stringify(transition)}\n`);
  renameSync(staged, target);
  renameSync(`${target}.json.tmp`, `${target}.json`);
  process.stdout.write(`Copied ${databasePath} to ${target}\n`);
  return result;
}

/** `PRAGMA user_version` of the SQLite file at `path`, or undefined when it cannot be read. */
function schemaFloor(path: string): number | undefined {
  try {
    const database = new Database(path, { readonly: true, fileMustExist: true });
    try {
      return database.pragma("user_version", { simple: true }) as number;
    } finally {
      database.close();
    }
  } catch {
    return undefined;
  }
}

/** The transition recorded beside the pre-update copy, or undefined when there is none or it is unreadable. */
function snapshotTransition(databasePath: string): SnapshotTransition | undefined {
  try {
    const value = JSON.parse(readFileSync(`${preUpdateSnapshotPath(databasePath)}.json`, "utf8")) as Partial<SnapshotTransition>;
    if (typeof value.from !== "string" || typeof value.to !== "string") return undefined;
    return { from: value.from, to: value.to, ...(Number.isInteger(value.fromReads) ? { fromReads: value.fromReads } : {}) };
  } catch {
    return undefined;
  }
}

/**
 * Whether this process is alone in having the database open. SQLite leaves WAL mode only for its last connection, so
 * the switch succeeds only then; it also writes the WAL's pages into the file, so the file holds the whole database.
 */
function aloneWithDatabase(path: string): boolean {
  try {
    const database = new Database(path, { fileMustExist: true });
    try {
      database.pragma("busy_timeout = 0");
      return database.pragma("journal_mode = DELETE", { simple: true }) === "delete";
    } finally {
      database.close();
    }
  } catch {
    return false;
  }
}

/**
 * `rerail update rollback`: roll the version back, then, when the copy taken before updating from the version rolled
 * back to (`from`) to the version left (`to`) exists and the database's schema floor is now above what `from` opens,
 * put the copy back: the version rolled back to refuses the database otherwise. The replaced database, with what
 * changed since the update, is kept as `rerail.sqlite.before-rollback`. A floor `from` still opens (additive steps, or
 * a floor it already supported) keeps the database as it is, and a copy taken for another change is never used. A
 * record from before `fromReads` was kept is judged by the copy's own floor. `stopDaemon` runs first: the daemon holds
 * the database open. A coding agent session can hold it open too, and would keep writing to the replaced file, so the
 * copy is put back only when nothing else has it open; otherwise the person is told how to finish. A restore that
 * fails is reported, never fatal: the version has rolled back already.
 */
export async function rollbackWithSnapshot(updater: SnapshotUpdater, databasePath: string, stopDaemon: () => Promise<unknown>): Promise<UpdateResult> {
  const leaving = (await updater.status()).current.version;
  const result = await updater.rollback();
  const transition = snapshotTransition(databasePath);
  if (transition === undefined || transition.from !== result.current.version || transition.to !== leaving) return result;
  const snapshot = preUpdateSnapshotPath(databasePath);
  const current = schemaFloor(databasePath);
  const before = schemaFloor(snapshot);
  if (current === undefined || before === undefined || current <= (transition.fromReads ?? before)) return result;
  const kept = `${databasePath}.before-rollback`;
  try {
    await stopDaemon();
    if (!aloneWithDatabase(databasePath)) {
      process.stderr.write(
        `rerail: ${databasePath} is at schema v${current}, which the version rolled back to cannot read, and coding agent `
        + `sessions still have it open. Close them, then copy ${snapshot} to ${databasePath} (keep the current file if you need what `
        + "changed since the update).\n",
      );
      return result;
    }
    for (const suffix of ["", "-wal", "-shm"]) {
      rmSync(`${kept}${suffix}`, { force: true });
      if (existsSync(`${databasePath}${suffix}`)) renameSync(`${databasePath}${suffix}`, `${kept}${suffix}`);
    }
    copyFileSync(snapshot, databasePath);
    chmodSync(databasePath, 0o600);
    process.stdout.write(
      `The update moved ${databasePath} to schema v${current}, which the version rolled back to cannot read, so it now holds `
      + `the copy taken before the update (schema v${before}). What changed since the update is in ${kept}.\n`,
    );
  } catch (error) {
    process.stderr.write(
      `rerail: could not put back ${snapshot} for the rolled-back version: ${(error as Error).message}. `
      + `Stop Rerail, then copy ${snapshot} to ${databasePath}.\n`,
    );
  }
  return result;
}

/** `rerail rules export|import`: the library and every repository's configuration, as a rules dump. */
export function runRulesCommand(args: readonly string[], usage: () => string): number {
  const action = args[0];
  const database = defaultRerailDatabasePath();
  if (action === "export") {
    const { options } = parseArguments(
      args.slice(1),
      { flags: [], options: ["--output"] },
      argument => `Unknown rules export option: ${argument}\n\n${usage()}`,
    );
    const rawOutput = options.get("--output");
    const output = rawOutput === undefined ? defaultRulesDumpPath(database) : resolve(rawOutput);
    const db = openRerailDatabase(database);
    let dump;
    try {
      dump = exportRulesDump(db);
    } finally {
      db.$client.close();
    }
    const staging = `${output}.${process.pid}.tmp`;
    mkdirSync(dirname(output), { recursive: true, mode: 0o700 });
    try {
      writeFileSync(staging, `${JSON.stringify(dump, null, 2)}\n`, { mode: 0o600 });
      renameSync(staging, output);
    } finally {
      rmSync(staging, { force: true });
    }
    process.stdout.write(`Exported ${dump.library.length} rules created by you and ${dump.repositories.length} repositories from ${database} to ${output}\n`);
    return 0;
  }
  if (action === "import") {
    const file = args[1];
    if (file === undefined || file.startsWith("--")) throw new Error(`rules import needs a dump file\n\n${usage()}`);
    parseArguments(args.slice(2), { flags: [], options: [] }, argument => `Unknown rules import option: ${argument}\n\n${usage()}`);
    const source = resolve(file);
    const value = parseJsonWithoutDuplicateKeys(readFileSync(source, "utf8"), source);
    const db = openRerailDatabase(database);
    try {
      const report = importRulesDump(db, loadBundledCatalog(), value);
      process.stdout.write(`Imported ${report.library} rules created by you and ${report.repositories} repositories from ${source} into ${database}\n`);
    } finally {
      db.$client.close();
    }
    return 0;
  }
  throw new Error(`Unknown rules action: ${action ?? "(none)"}\n\n${usage()}`);
}

function plural(count: number, noun: string): string {
  return count === 1 ? noun : `${noun}s`;
}

/**
 * Delete this repository's execution history, across all its checkouts, from the Rerail database: sessions recorded
 * under its key or aliases, and older ones under its checkouts' clone ids. Rules, other repositories, and settings
 * stay; live sessions never observe a rule reset.
 */
export function runResetCommand(args: readonly string[], usage: () => string): number {
  parseArguments(args, { flags: [], options: [] }, argument => `Unknown reset option: ${argument}\n\n${usage()}`);
  const database = defaultRerailDatabasePath();
  if (!existsSync(database)) {
    process.stdout.write(`No Rerail database at ${database}; nothing to reset.\n`);
    return 0;
  }
  const db = openRerailDatabase(database);
  let ids: string[];
  let repositoryKey: string;
  try {
    const checkout = lookupCheckout(db, process.cwd());
    if (checkout === undefined) {
      process.stdout.write("Rerail has not seen this repository; nothing to reset.\n");
      return 0;
    }
    repositoryKey = checkout.repositoryKey;
    const repository = listRepositories(db).find(candidate => candidate.repositoryKey === repositoryKey);
    ids = [...new Set([repositoryKey, ...(repository?.aliases ?? []), ...(repository?.checkouts ?? []).map(row => row.cloneId)])];
  } finally {
    db.$client.close();
  }
  let sessions = 0;
  let openSessions = 0;
  for (const id of ids) {
    const report = resetRepositoryHistory(database, id);
    sessions += report.sessions;
    openSessions += report.openSessions;
  }
  const kept = openSessions > 0 ? `, ${openSessions} running ${plural(openSessions, "session")} kept (history dropped)` : "";
  process.stdout.write(
    `Removed the history of this repository (${repositoryKey}, across all its checkouts) from ${database}: `
    + `${sessions} ended ${plural(sessions, "session")} deleted`
    + kept
    + ". Rules and other repositories are unchanged.\n",
  );
  return 0;
}
