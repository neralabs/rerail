import { chmodSync, closeSync, existsSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import Database from "better-sqlite3";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import type { UpdateResult } from "../setup/updater.js";
import { parseArguments } from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase, resetRepositoryHistory, rollbackRestoring, rollbackRestoringPath } from "../engine/storage/database.js";
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
  /** The highest schema floor `from` opens: the floor of the version that ran the update. */
  fromReads: number;
}

/** What `rerail update` and `rerail update rollback` ask of the managed updater. */
export interface SnapshotUpdater {
  update(channel?: string): Promise<UpdateResult>;
  rollback(): Promise<UpdateResult>;
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
 * Removes the pre-update copy and its record, so no later rollback finds them. Runs after a version change that kept
 * no copy of its own, which has succeeded: a removal that fails is reported with what to remove by hand, never fatal.
 */
function discardSnapshot(databasePath: string): void {
  const target = preUpdateSnapshotPath(databasePath);
  try {
    rmSync(`${target}.json`, { force: true });
    rmSync(target, { force: true });
  } catch (error) {
    process.stderr.write(`rerail: remove ${target} and ${target}.json yourself before rolling back: ${(error as Error).message}\n`);
  }
}

/**
 * `rerail update`: copy the database, then update. Only an update that changes the version replaces
 * `rerail.sqlite.pre-update`, and the copy records that change, read from the updater's own result (`previous`, under
 * its lock), and the schema floor this version opens (`fromReads`, this build's own); an update that finds nothing
 * newer, or fails, leaves the copy of the last real change in place, so rolling that change back still finds it. A
 * version change without a copy (the copy, or recording it, failed) removes the last one: it no longer belongs to the
 * change a rollback would undo.
 */
export async function updateWithSnapshot(
  updater: SnapshotUpdater,
  databasePath: string,
  channel: string | undefined,
  fromReads: number = RERAIL_SCHEMA_VERSION,
): Promise<UpdateResult> {
  const staged = stageSnapshot(databasePath);
  let result: UpdateResult;
  try {
    result = await updater.update(channel);
  } catch (error) {
    if (staged !== undefined) removeQuietly(staged);
    throw error;
  }
  const from = result.previous?.version;
  if (!result.changed || result.current.version === from) {
    if (staged !== undefined) removeQuietly(staged);
    return result;
  }
  if (staged === undefined || from === undefined) {
    if (staged !== undefined) removeQuietly(staged);
    discardSnapshot(databasePath);
    return result;
  }
  const target = preUpdateSnapshotPath(databasePath);
  const transition: SnapshotTransition = { from, to: result.current.version, fromReads };
  try {
    writeFileSync(`${target}.json.tmp`, `${JSON.stringify(transition)}\n`, { mode: 0o600 });
    // The old record goes first: a copy is never paired with a record of another change.
    rmSync(`${target}.json`, { force: true });
    renameSync(staged, target);
    renameSync(`${target}.json.tmp`, `${target}.json`);
  } catch (error) {
    removeQuietly(staged);
    removeQuietly(`${target}.json.tmp`);
    process.stderr.write(`rerail: could not keep the copy of ${databasePath} taken before the update: ${(error as Error).message}\n`);
    discardSnapshot(databasePath);
    return result;
  }
  process.stdout.write(`Copied ${databasePath} to ${target}\n`);
  return result;
}

/** `PRAGMA user_version` of the SQLite file at `path`, or why it cannot be read. */
function readFloor(path: string): { floor: number } | { error: string } {
  try {
    const database = new Database(path, { readonly: true, fileMustExist: true });
    try {
      return { floor: database.pragma("user_version", { simple: true }) as number };
    } finally {
      database.close();
    }
  } catch (error) {
    return { error: (error as Error).message };
  }
}

/** `PRAGMA user_version` of the SQLite file at `path`, or undefined when it cannot be read. */
function schemaFloor(path: string): number | undefined {
  const read = readFloor(path);
  return "floor" in read ? read.floor : undefined;
}

/** The transition recorded beside the pre-update copy, or undefined when there is none or it is unreadable. */
function snapshotTransition(databasePath: string): SnapshotTransition | undefined {
  try {
    const value = JSON.parse(readFileSync(`${preUpdateSnapshotPath(databasePath)}.json`, "utf8")) as Partial<SnapshotTransition>;
    if (typeof value.from !== "string" || typeof value.to !== "string" || !Number.isInteger(value.fromReads)) return undefined;
    return { from: value.from, to: value.to, fromReads: value.fromReads! };
  } catch {
    return undefined;
  }
}

/**
 * Removes the record of the change a copy was taken for, keeping the copy: once a rollback has acted on a record
 * (put the copy back, or told the person how to), no later rollback may match it again.
 */
function forgetTransition(databasePath: string): void {
  removeQuietly(`${preUpdateSnapshotPath(databasePath)}.json`);
}

/** Why `restoreInPlace` left the database as it was. */
type RestoreRefusal = "open" | "restoring";

/** How often, and how long apart, a backup that copied nothing is tried again while `.restoring` keeps openers out. */
const BACKUP_ATTEMPTS = 20;
const BACKUP_RETRY_MS = 50;

function sleep(milliseconds: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, milliseconds);
  return promise;
}

/** SQLite's answer when another connection holds the database: the only refusal that means a session has it open. */
function heldByAnother(error: unknown): boolean {
  const code = (error as { code?: unknown } | undefined)?.code;
  return typeof code === "string" && (code.startsWith("SQLITE_BUSY") || code.startsWith("SQLITE_LOCKED"));
}

/**
 * The first of `<base>`, `<base>.2`, `<base>.3`, ... that does not exist yet, created empty with mode 0600: an earlier
 * rollback's saved database is never replaced.
 */
function freshKeptPath(base: string): string {
  for (let index = 1; ; index += 1) {
    const candidate = index === 1 ? base : `${base}.${index}`;
    try {
      closeSync(openSync(candidate, "wx", 0o600));
      return candidate;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
  }
}

/**
 * Puts copy `snapshot` back into the database at `path`, in place and through SQLite, after saving what the database
 * holds under the first free name from `keptBase` (mode 0600), which it returns. Writing the same file, rather than
 * renaming another over it, keeps every connection, open or opening, on the restored database: none goes on writing
 * to a file that has been moved aside. `rerail.sqlite.restoring` is held from before the check through the copy, and
 * every Rerail opener refuses the database while it exists (`connectSqlite`), so no session commits between the save
 * and the copy, where its change would be in neither file. Refused, with nothing changed, while that file exists
 * (another rollback runs, or one was interrupted) or another connection holds the database (a coding agent session,
 * opened before the file existed): SQLite leaves WAL mode only for its last connection, and that session would go on
 * with the database it already read. An opener's brief read can still make a backup step find the file busy, and
 * better-sqlite3 then resolves without copying anything, so the backup is tried again while `.restoring` keeps new
 * openers out. Throws on any other failure, and when the copy did not reach the file (`floor`, the copy's schema
 * floor, is not what the file records).
 */
async function restoreInPlace(path: string, snapshot: string, keptBase: string, floor: number): Promise<RestoreRefusal | { kept: string }> {
  const restoring = rollbackRestoringPath(path);
  const claim = (): void => writeFileSync(restoring, `${process.pid}\n`, { flag: "wx", mode: 0o600 });
  try {
    claim();
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    // A rollback that was killed left its file behind; the process it names is gone, so nothing holds the database.
    if (rollbackRestoring(path)) return "restoring";
    rmSync(restoring, { force: true });
    try {
      claim();
    } catch (again) {
      if ((again as NodeJS.ErrnoException).code === "EEXIST") return "restoring";
      throw again;
    }
  }
  try {
    let kept: string;
    const database = new Database(path, { fileMustExist: true });
    try {
      database.pragma("busy_timeout = 0");
      let mode: unknown;
      try {
        mode = database.pragma("journal_mode = DELETE", { simple: true });
      } catch (error) {
        if (heldByAnother(error)) return "open";
        throw error;
      }
      if (mode !== "delete") return "open";
      // VACUUM INTO fills an empty file, keeping the mode it was created with; one it could not fill is not kept.
      kept = freshKeptPath(keptBase);
      try {
        database.prepare("VACUUM INTO ?").run(kept);
      } catch (error) {
        removeQuietly(kept);
        throw error;
      }
    } finally {
      database.close();
    }
    try {
      const source = new Database(snapshot, { readonly: true, fileMustExist: true });
      try {
        for (let attempt = 1; ; attempt += 1) {
          await source.backup(path);
          if (schemaFloor(path) === floor || attempt === BACKUP_ATTEMPTS) break;
          await sleep(BACKUP_RETRY_MS);
        }
      } finally {
        source.close();
      }
      chmodSync(path, 0o600);
      const written = schemaFloor(path);
      if (written !== floor) throw new Error(`the copy did not reach the database, which is still at schema v${String(written)}`);
    } catch (error) {
      throw new Error(`${(error as Error).message} (the database as it was is saved in ${kept})`, { cause: error });
    }
    return { kept };
  } finally {
    // Whatever the restore came to stands; a file left behind is reported, and is stale once this process ends.
    try {
      rmSync(restoring, { force: true });
    } catch (error) {
      process.stderr.write(`rerail: delete ${restoring}: ${(error as Error).message}\n`);
    }
  }
}

/**
 * After a rollback, a copy with no record of the change it was taken for: an older release's `rerail update` (such as
 * 0.1.0-next.1's) wrote `rerail.sqlite.pre-update` without one. It is not put back without knowing which change it
 * undoes, but when its schema floor is below the database's, the version rolled back to may refuse the database
 * (0.1.0-next.1 reads only schema v1, and this release adopts its files at the baseline), so the person is told how to
 * put it back by hand.
 */
function hintUnrecordedCopy(databasePath: string, version: string): void {
  const snapshot = preUpdateSnapshotPath(databasePath);
  if (!existsSync(snapshot)) return;
  const current = schemaFloor(databasePath);
  const copy = schemaFloor(snapshot);
  if (current === undefined || copy === undefined || copy >= current) return;
  process.stderr.write(
    `rerail: ${databasePath} is at schema v${current}, and ${snapshot} (schema v${copy}) was taken by an update that `
    + `recorded no version change, so Rerail does not put it back itself. If ${version} refuses the database, stop Rerail, `
    + `keep ${databasePath} elsewhere if you need what changed since that update, then copy ${snapshot} to ${databasePath}.\n`,
  );
}

/**
 * `rerail update rollback`: roll the version back, then, when the copy taken before updating from the version rolled
 * back to (`from`) to the version left (`to`, the updater's own `previous`, read under its lock) exists and the
 * database's schema floor is now above what `from` opens (`fromReads`), put the copy back: the version rolled back to
 * refuses the database otherwise. The replaced database, with what changed since the update, is kept as
 * `rerail.sqlite.before-rollback` (or `.before-rollback.2`, ..., never over an earlier one). A record is acted on once:
 * after the copy is put back, or the person is told how to put it back (a coding agent session holds the database,
 * another rollback runs, the restore failed, or a floor cannot be read), it is removed, so no later rollback matches it
 * again; a copy put back is removed too. A floor `from` still opens (additive steps, or a floor it already supported)
 * keeps the database as it is, and a copy taken for another change is never used. `stopDaemon` runs first, unless
 * another rollback is already restoring: the daemon holds the database open, and the messages say when it was stopped.
 * The copy is written into the database file itself (`restoreInPlace`). A restore that fails, or whose copy did not
 * reach the file, is reported, never fatal: the version has rolled back already.
 */
export async function rollbackWithSnapshot(updater: SnapshotUpdater, databasePath: string, stopDaemon: () => Promise<unknown>): Promise<UpdateResult> {
  const result = await updater.rollback();
  const leaving = result.previous?.version;
  const transition = snapshotTransition(databasePath);
  if (transition === undefined) {
    hintUnrecordedCopy(databasePath, result.current.version);
    return result;
  }
  if (leaving === undefined || transition.from !== result.current.version || transition.to !== leaving) return result;
  const snapshot = preUpdateSnapshotPath(databasePath);
  const restoring = rollbackRestoringPath(databasePath);
  const byHand = `copy ${snapshot} to ${databasePath} (keep the current file if you need what changed since the update)`;
  const current = readFloor(databasePath);
  const before = readFloor(snapshot);
  if (!("floor" in current) || !("floor" in before)) {
    const [file, read] = "floor" in current ? [snapshot, before] : [databasePath, current];
    forgetTransition(databasePath);
    process.stderr.write(
      `rerail: could not read the schema of ${file} (${"error" in read ? read.error : ""}), so the copy taken before the `
      + `update is not put back. If ${result.current.version} refuses the database, stop Rerail, then ${byHand}.\n`,
    );
    return result;
  }
  if (current.floor <= transition.fromReads) return result;
  const cannotRead = `${databasePath} is at schema v${current.floor}, which the version rolled back to cannot read`;
  if (rollbackRestoring(databasePath)) {
    forgetTransition(databasePath);
    process.stderr.write(
      `rerail: ${cannotRead}, and ${restoring} exists: another rollback is running, or one was interrupted. Once no `
      + `rollback runs, delete ${restoring}, then ${byHand}.\n`,
    );
    return result;
  }
  let daemon = "";
  try {
    if (await stopDaemon() === "stopped") daemon = " Rerail's daemon was stopped for this; rerail daemon start starts it again.";
    const restored = await restoreInPlace(databasePath, snapshot, `${databasePath}.before-rollback`, before.floor);
    if (typeof restored === "string") {
      forgetTransition(databasePath);
      const why = restored === "open"
        ? "coding agent sessions still have it open. Close them"
        : `${restoring} exists: another rollback is running, or one was interrupted. Once no rollback runs, delete ${restoring}`;
      process.stderr.write(`rerail: ${cannotRead}, and ${why}, then ${byHand}.${daemon}\n`);
      return result;
    }
    discardSnapshot(databasePath);
    process.stdout.write(
      `The update moved ${databasePath} to schema v${current.floor}, which the version rolled back to cannot read, so it now `
      + `holds the copy taken before the update (schema v${before.floor}). What changed since the update is in ${restored.kept}.${daemon}\n`,
    );
  } catch (error) {
    forgetTransition(databasePath);
    process.stderr.write(
      `rerail: could not put back ${snapshot} for the rolled-back version: ${(error as Error).message}. `
      + `Stop Rerail, then copy ${snapshot} to ${databasePath}.${daemon}\n`,
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
