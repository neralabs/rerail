import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import Database from "better-sqlite3";
import { loadBundledCatalog } from "../engine/catalog/bundled-rules.js";
import { parseArguments } from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase, resetRepositoryHistory } from "../engine/storage/database.js";
import { parseJsonWithoutDuplicateKeys } from "../engine/json-duplicate-keys.js";
import { listRepositories, lookupCheckout } from "../engine/catalog/repository-registry.js";
import { defaultRulesDumpPath, exportRulesDump, importRulesDump } from "../engine/catalog/rule-bootstrap.js";

/** Where `rerail update` leaves a consistent copy of the Rerail database before it switches versions. */
export function preUpdateSnapshotPath(databasePath: string): string {
  return `${databasePath}.pre-update`;
}

/**
 * Before an update swaps the active version, copy the whole Rerail database (every project's
 * rules and every repository's history) to `rerail.sqlite.pre-update`, replacing the previous
 * copy. A copy that fails is reported, never fatal; the update must not be blocked by a
 * database the new version may be the fix for.
 */
export function snapshotDatabaseBeforeUpdate(databasePath = defaultRerailDatabasePath()): void {
  if (!existsSync(databasePath)) return;
  const target = preUpdateSnapshotPath(databasePath);
  const staging = `${target}.${process.pid}.tmp`;
  try {
    rmSync(staging, { force: true });
    const source = new Database(databasePath, { readonly: true, fileMustExist: true });
    try {
      source.pragma("busy_timeout = 5000");
      source.prepare("VACUUM INTO ?").run(staging);
    } finally {
      source.close();
    }
    renameSync(staging, target);
    process.stdout.write(`Copied ${databasePath} to ${target}\n`);
  } catch (error) {
    rmSync(staging, { force: true });
    process.stderr.write(`rerail: could not copy ${databasePath} before the update: ${(error as Error).message}\n`);
  }
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

/**
 * After `rerail update rollback`: when the update's migrations raised the database's schema floor above the copy
 * taken before it, the version rolled back to refuses the database, so the pre-update copy goes back in its place.
 * The replaced database, with what changed since the update, is kept as `rerail.sqlite.before-rollback`. A floor
 * the update left alone (additive steps only) keeps the database as it is. `stopDaemon` runs first: the daemon
 * holds the database open. A restore that fails is reported, never fatal: the version has rolled back already.
 */
export async function restoreDatabaseForRollback(databasePath: string, stopDaemon: () => Promise<unknown>): Promise<void> {
  const snapshot = preUpdateSnapshotPath(databasePath);
  const current = schemaFloor(databasePath);
  const before = schemaFloor(snapshot);
  if (current === undefined || before === undefined || current <= before) return;
  const kept = `${databasePath}.before-rollback`;
  try {
    await stopDaemon();
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
