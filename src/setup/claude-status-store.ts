import { chmodSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import type * as Sqlite from "node:sqlite";
import { enableWal } from "../engine/storage/sqlite-wal.js";

/** Installation recovery has its own schema in `harness.sqlite`, separate from the Rerail database. */
const STATUS_COLUMNS = `settings_path AS settingsPath, installed_command AS installedCommand,
  installed_root AS installedRoot, previous_json AS previousJson`;
type StoredStatus = Omit<ClaudeStatusRegistration, "previous" | "previousInherited"> & { previousJson: string };

let sqlite: typeof Sqlite | undefined;
function openStatusDatabase(path: string, options: ConstructorParameters<typeof Sqlite.DatabaseSync>[1]): Sqlite.DatabaseSync {
  const module: typeof Sqlite = sqlite ??= createRequire(import.meta.url)("node:sqlite");
  return new module.DatabaseSync(path, options);
}

export interface ClaudeStatusLine extends Record<string, unknown> {
  type: "command";
  command: string;
}

export interface ClaudeStatusRegistration {
  settingsPath: string;
  installedCommand: string;
  installedRoot: string;
  previous: ClaudeStatusLine | null;
  /** The predecessor was inherited from shared settings, not present in this local file. */
  previousInherited?: boolean;
}

/** Registration ownership lives outside project databases and presentation preferences. */
export function claudeStatusDatabasePath(home: string = homedir()): string {
  return join(resolve(home), ".rerail", "harness.sqlite");
}

export function isClaudeStatusLine(value: unknown): value is ClaudeStatusLine {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    && "type" in value && value.type === "command"
    && "command" in value && typeof value.command === "string" && value.command.trim() !== "";
}

function readClaudeStatusData<T>(home: string, read: (database: Sqlite.DatabaseSync) => T): T | undefined {
  const path = claudeStatusDatabasePath(home);
  if (!existsSync(path)) return undefined;
  using database = openStatusDatabase(path, { readOnly: true, timeout: 100 });
  if (database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'harness_status_lines'").get() === undefined) return undefined;
  return read(database);
}

/** Read-only even on first run: never creates or repairs an installation. */
export function readClaudeStatusRegistration(
  home: string = homedir(),
  settingsPath: string = join(resolve(home), ".claude", "settings.json"),
): ClaudeStatusRegistration | undefined {
  return readClaudeStatusData(home, database => {
    const row = database.prepare(`SELECT ${STATUS_COLUMNS} FROM harness_status_lines WHERE settings_path = ?`)
      .get(resolve(settingsPath)) as StoredStatus | undefined;
    if (row === undefined) return undefined;
    let preserved: unknown;
    try {
      preserved = JSON.parse(row.previousJson);
    } catch {
      throw new Error("Cannot read preserved Claude status line: invalid registration data");
    }
    const inherited = !isClaudeStatusLine(preserved) && typeof preserved === "object" && preserved !== null
      && "previousInherited" in preserved && preserved.previousInherited === true && "previous" in preserved ? preserved : undefined;
    const previous = inherited === undefined ? preserved : inherited.previous;
    if (previous !== null && !isClaudeStatusLine(previous)) {
      throw new Error("Cannot read preserved Claude status line: invalid command specification");
    }
    return {
      settingsPath: row.settingsPath,
      installedCommand: row.installedCommand,
      installedRoot: row.installedRoot,
      previous,
      ...(inherited === undefined ? {} : { previousInherited: true }),
    };
  });
}

export function readClaudeStatusCommand(
  home: string = homedir(), settingsPath?: string, cwd: string = process.cwd(),
): ClaudeStatusLine | undefined {
  const registration = readClaudeStatusRegistration(home, settingsPath);
  if (!registration?.previousInherited) return registration?.previous ?? undefined;
  // A generated root-local override is wiring, not a user-local predecessor.
  for (const path of [
    join(resolve(cwd), ".claude", "settings.local.json"),
    join(resolve(cwd), ".claude", "settings.json"),
    join(resolve(home), ".claude", "settings.json"),
  ]) {
    if (path === registration.settingsPath || !existsSync(path)) continue;
    const value: unknown = JSON.parse(readFileSync(path, "utf8"));
    if (typeof value !== "object" || value === null || !("statusLine" in value) || !isClaudeStatusLine(value.statusLine)) continue;
    const owner = readClaudeStatusRegistration(home, path);
    if (value.statusLine.command !== owner?.installedCommand) return value.statusLine;
    if (!owner.previousInherited && owner.previous !== null) return owner.previous;
  }
  return undefined;
}

/** Installation inventory, including projects other than the current working directory. */
export function readClaudeStatusSettingsPaths(home: string = homedir()): string[] {
  return readClaudeStatusData(home, database =>
    database.prepare("SELECT settings_path AS path FROM harness_status_lines").all().map(row => row.path as string)) ?? [];
}

/** Commit recovery data before installation; remove it only after settings restoration succeeds. */
export async function writeClaudeStatusRegistration(
  home: string,
  registration: ClaudeStatusRegistration | undefined,
  writeSettings: () => Promise<void>,
  path: string = join(resolve(home), ".claude", "settings.json"),
): Promise<void> {
  const databasePath = claudeStatusDatabasePath(home);
  mkdirSync(dirname(databasePath), { recursive: true, mode: 0o700 });
  using database = openStatusDatabase(databasePath, { timeout: 5000 });
  const settingsPath = resolve(registration?.settingsPath ?? path);
  chmodSync(databasePath, 0o600);
  enableWal(() => database.exec("PRAGMA journal_mode = WAL"));
  database.exec("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL");
  database.exec(`CREATE TABLE IF NOT EXISTS harness_status_lines (
    settings_path text PRIMARY KEY NOT NULL, installed_command text NOT NULL,
    installed_root text NOT NULL, previous_json text NOT NULL
  )`);
  const remove = database.prepare("DELETE FROM harness_status_lines WHERE settings_path = ?");
  if (registration === undefined) {
    await writeSettings();
    remove.run(settingsPath);
    return;
  }
  const upsert = database.prepare(`INSERT INTO harness_status_lines
    (settings_path, installed_command, installed_root, previous_json) VALUES (?, ?, ?, ?)
    ON CONFLICT(settings_path) DO UPDATE SET installed_command = excluded.installed_command,
    installed_root = excluded.installed_root, previous_json = excluded.previous_json`);
  let previous: StoredStatus | undefined;
  database.exec("BEGIN");
  try {
    previous = database.prepare(`SELECT ${STATUS_COLUMNS} FROM harness_status_lines WHERE settings_path = ?`).get(settingsPath) as StoredStatus | undefined;
    upsert.run(settingsPath, registration.installedCommand, registration.installedRoot, JSON.stringify(registration.previousInherited
      ? { previous: registration.previous, previousInherited: true } : registration.previous));
    database.exec("COMMIT");
  } catch (error) {
    database.exec("ROLLBACK");
    throw error;
  }
  try {
    await writeSettings();
  } catch (error) {
    try {
      if (previous === undefined) remove.run(settingsPath);
      else upsert.run(previous.settingsPath, previous.installedCommand, previous.installedRoot, previous.previousJson);
    } catch (restoreError) {
      throw new AggregateError([error, restoreError], "Cannot write Claude settings or restore registration ownership");
    }
    throw error;
  }
}
