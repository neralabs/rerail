import { closeSync, existsSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import Database from "better-sqlite3";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import type { UpdateResult } from "../setup/updater.js";
import { parseArguments } from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase, resetRepositoryHistoryIn } from "../engine/storage/database.js";
import { parseJsonWithoutDuplicateKeys } from "../engine/json-duplicate-keys.js";
import { listRepositories, lookupCheckout, repositoryHistoryIds } from "../engine/catalog/repository-registry.js";
import { defaultRulesDumpPath, exportRulesDump, importRulesDump } from "../engine/catalog/rule-bootstrap.js";

/** Where `rerail update` leaves a consistent copy of the Rerail database before it switches versions. */
export function preUpdateSnapshotPath(databasePath: string): string {
  return `${databasePath}.pre-update`;
}

/** What `rerail update` asks of the managed updater. */
export interface SnapshotUpdater {
  update(channel?: string): Promise<UpdateResult>;
}

/**
 * A consistent copy of the whole database (every project's rules and every repository's history) beside the
 * current copy, readable only by its owner, or undefined when there is no database or the copy failed. A copy that
 * fails is reported, never fatal; the update must not be blocked by a database the new version may be the fix for.
 */
function stageSnapshot(databasePath: string): string | undefined {
  if (!existsSync(databasePath)) return undefined;
  const staging = `${preUpdateSnapshotPath(databasePath)}.${process.pid}.tmp`;
  try {
    rmSync(staging, { force: true });
    // VACUUM INTO fills an empty file, keeping the mode it was created with.
    closeSync(openSync(staging, "wx", 0o600));
    const source = new Database(databasePath, { readonly: true, fileMustExist: true });
    try {
      source.pragma("busy_timeout = 5000");
      source.prepare("VACUUM INTO ?").run(staging);
    } finally {
      source.close();
    }
    return staging;
  } catch (error) {
    removeQuietly(staging);
    process.stderr.write(`rerail: could not copy ${databasePath} before the update: ${(error as Error).message}\n`);
    return undefined;
  }
}

/** Best-effort cleanup on a path already failing: a file left behind must not hide the failure being reported. */
function removeQuietly(path: string): void {
  try {
    rmSync(path, { force: true });
  } catch {
    // The failure being reported is the one that matters.
  }
}

/**
 * Removes the pre-update copy of an earlier change. Runs after a version change that kept no copy of its own, which
 * has succeeded: a removal that fails is reported with what to remove by hand, never fatal.
 */
function discardSnapshot(databasePath: string): void {
  const target = preUpdateSnapshotPath(databasePath);
  try {
    rmSync(target, { force: true });
  } catch (error) {
    process.stderr.write(`rerail: ${target} is from an earlier update; remove it yourself: ${(error as Error).message}\n`);
  }
}

/**
 * `rerail update`: copy the database, then update. Only an update that changes the version replaces
 * `rerail.sqlite.pre-update`; an update that finds nothing newer, or fails, leaves the copy of the last real change in
 * place. A version change without a copy (the copy, or keeping it, failed) removes the last one: it is not the
 * database as it was before the change that is now active.
 */
export async function updateWithSnapshot(updater: SnapshotUpdater, databasePath: string, channel: string | undefined): Promise<UpdateResult> {
  const staged = stageSnapshot(databasePath);
  let result: UpdateResult;
  try {
    result = await updater.update(channel);
  } catch (error) {
    if (staged !== undefined) removeQuietly(staged);
    throw error;
  }
  if (!result.changed || result.current.version === result.previous?.version) {
    if (staged !== undefined) removeQuietly(staged);
    return result;
  }
  if (staged === undefined) {
    discardSnapshot(databasePath);
    return result;
  }
  const target = preUpdateSnapshotPath(databasePath);
  try {
    renameSync(staged, target);
  } catch (error) {
    removeQuietly(staged);
    process.stderr.write(`rerail: could not keep the copy of ${databasePath} taken before the update: ${(error as Error).message}\n`);
    discardSnapshot(databasePath);
    return result;
  }
  process.stdout.write(`Copied ${databasePath} to ${target}\n`);
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
      const report = importRulesDump(db, loadRuleCatalog(), value);
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
  let repositoryKey: string;
  let sessions: number;
  let openSessions: number;
  try {
    const checkout = lookupCheckout(db, process.cwd());
    if (checkout === undefined) {
      process.stdout.write("Rerail has not seen this repository; nothing to reset.\n");
      return 0;
    }
    repositoryKey = checkout.repositoryKey;
    const repository = listRepositories(db).find(candidate => candidate.repositoryKey === repositoryKey);
    ({ sessions, openSessions } = resetRepositoryHistoryIn(db, repository === undefined ? [repositoryKey] : repositoryHistoryIds(repository)));
  } finally {
    db.$client.close();
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
