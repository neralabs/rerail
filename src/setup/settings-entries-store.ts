import { chmodSync, existsSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import type * as Sqlite from "node:sqlite";
import { enableWal } from "../engine/storage/sqlite-wal.js";
import { claudeStatusDatabasePath } from "./claude-status-store.js";

/**
 * What registration added to the lists of one coding agent's settings file, so removal takes out exactly that: the
 * entries it added to each list, by dotted key (`permissions.deny`), and the objects and lists it created to hold them.
 * An entry the person had already is never recorded, so it stays when Rerail goes. `root` is the package root the
 * registration was for.
 */
export interface AddedSettingsEntries {
  root: string;
  entries: Record<string, string[]>;
  created: string[];
}

/**
 * Kept beside the status-line records in `$HOME/.rerail/harness.sqlite`, not in the Rerail home: set-up run again with
 * another `RERAIL_HOME` still finds what the earlier one added.
 */
const TABLE = "harness_added_settings_entries";

let sqlite: typeof Sqlite | undefined;
function openDatabase(path: string, options: ConstructorParameters<typeof Sqlite.DatabaseSync>[1]): Sqlite.DatabaseSync {
  const module: typeof Sqlite = sqlite ??= createRequire(import.meta.url)("node:sqlite");
  return new module.DatabaseSync(path, options);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(entry => typeof entry === "string");
}

function parseAdded(text: string): AddedSettingsEntries {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error("Cannot read the settings entries Rerail added: invalid registration data");
  }
  if (typeof value !== "object" || value === null || !("root" in value) || typeof value.root !== "string"
    || !("entries" in value) || !("created" in value)
    || typeof value.entries !== "object" || value.entries === null || !isStringArray(value.created)
    || !Object.values(value.entries).every(isStringArray)) {
    throw new Error("Cannot read the settings entries Rerail added: invalid registration data");
  }
  return { root: value.root, entries: value.entries as Record<string, string[]>, created: value.created };
}

/** Read-only, and undefined when nothing was recorded for `settingsPath`. */
export function readAddedSettingsEntries(home: string, settingsPath: string): AddedSettingsEntries | undefined {
  const path = claudeStatusDatabasePath(home);
  if (!existsSync(path)) return undefined;
  using database = openDatabase(path, { readOnly: true, timeout: 100 });
  if (database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?").get(TABLE) === undefined) return undefined;
  const row = database.prepare(`SELECT added_json AS addedJson FROM ${TABLE} WHERE settings_path = ?`)
    .get(resolve(settingsPath)) as { addedJson: string } | undefined;
  return row === undefined ? undefined : parseAdded(row.addedJson);
}

/**
 * Records `added` (none: forgets the file) before `writeSettings` runs, and puts the earlier record back when the
 * write fails, so the record never misses an entry that is in the file.
 */
export async function writeAddedSettingsEntries(
  home: string, settingsPath: string, added: AddedSettingsEntries | undefined, writeSettings: () => Promise<void>,
): Promise<void> {
  const databasePath = claudeStatusDatabasePath(home);
  mkdirSync(dirname(databasePath), { recursive: true, mode: 0o700 });
  using database = openDatabase(databasePath, { timeout: 5000 });
  chmodSync(databasePath, 0o600);
  enableWal(() => database.exec("PRAGMA journal_mode = WAL"));
  database.exec("PRAGMA synchronous = FULL");
  database.exec(`CREATE TABLE IF NOT EXISTS ${TABLE} (settings_path text PRIMARY KEY NOT NULL, added_json text NOT NULL)`);
  const key = resolve(settingsPath);
  const previous = database.prepare(`SELECT added_json AS addedJson FROM ${TABLE} WHERE settings_path = ?`)
    .get(key) as { addedJson: string } | undefined;
  const remove = database.prepare(`DELETE FROM ${TABLE} WHERE settings_path = ?`);
  const upsert = database.prepare(`INSERT INTO ${TABLE} (settings_path, added_json) VALUES (?, ?)
    ON CONFLICT(settings_path) DO UPDATE SET added_json = excluded.added_json`);
  // Removal forgets the record only once the file no longer holds the entries.
  if (added === undefined) {
    await writeSettings();
    remove.run(key);
    return;
  }
  upsert.run(key, JSON.stringify(added));
  try {
    await writeSettings();
  } catch (error) {
    try {
      if (previous === undefined) remove.run(key);
      else upsert.run(key, previous.addedJson);
    } catch (restoreError) {
      throw new AggregateError([error, restoreError], "Cannot write settings or restore the record of Rerail's entries");
    }
    throw error;
  }
}
