import { randomUUID } from "node:crypto";
import { chmodSync, existsSync, readFileSync, statSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { LazyReadonlyRerailDatabase, openRerailDatabase } from "../../../src/engine/storage/database.js";
import {
  RERAIL_BASELINE_SCHEMA_VERSION,
  RERAIL_MIGRATIONS,
  RERAIL_SCHEMA_VERSION,
  addColumn,
  type Migration,
} from "../../../src/engine/storage/migrations.js";
import { ruleSettings } from "../../../src/engine/storage/schema.js";
import { listRepositoryRules, loadRuleCatalog } from "../../../src/engine/index.js";
import { RERAIL_SCHEMA_STATEMENTS } from "../../../src/engine/storage/schema-sql.generated.js";
import { RERAIL_BASELINE_SCHEMA_STATEMENTS } from "../../../src/engine/storage/baseline-schema.js";
import { fixturesPath } from "../../support/oracles.js";
import { installPack, uninstallPack } from "../../support/installed-packs.js";
import { codeRule } from "../../support/rule-configuration-builders.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDatabase(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-baseline-"));
  temporaryDirectories.push(root);
  return join(root, "home", "rerail.sqlite");
}

/** Every table the generated DDL creates. */
const BASELINE_TABLES = RERAIL_SCHEMA_STATEMENTS
  .map(statement => /^CREATE TABLE IF NOT EXISTS `([^`]+)`/u.exec(statement)?.[1])
  .filter(name => name !== undefined)
  .sort();

interface Stamp {
  tables: string[];
  version: number;
}

function stampOf(path: string): Stamp {
  const database = new Database(path, { readonly: true });
  try {
    const tables = database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'").pluck().all() as string[];
    return { tables: tables.sort(), version: database.pragma("user_version", { simple: true }) as number };
  } finally {
    database.close();
  }
}

/** Create a current file at `path`, then change it behind the schema. */
function stampedFile(path: string, statements: string): void {
  openRerailDatabase(path).$client.close();
  const database = new Database(path);
  try {
    database.exec(statements);
  } finally {
    database.close();
  }
}

/** The file's bytes with its WAL checkpointed in, so a refused open is seen to leave it untouched. */
function settledBytes(path: string): Buffer {
  const database = new Database(path);
  database.pragma("wal_checkpoint(TRUNCATE)");
  database.close();
  return readFileSync(path);
}

/** The refusal for a file that holds something other than a Rerail database. */
function unreadable(path: string, recorded: string): string {
  return `${path} is not a database this build can open (${recorded}; this build reads schema v${RERAIL_SCHEMA_VERSION}). `
    + "Move the file aside to start fresh.";
}

const newerRefusal = (path: string, version: number): string =>
  `${path} was written by a newer Rerail (schema v${version}); this Rerail supports schema v${RERAIL_SCHEMA_VERSION}. `
  + "Update Rerail to open it, or move the file aside to start fresh.";

/** Expect every open of `path` (writable, read-only, lazy) to be refused with `message`. */
function expectRefused(path: string, message: string): void {
  expect(() => openRerailDatabase(path)).toThrow(new Error(message));
  expect(() => openRerailDatabase(path, { readonly: true })).toThrow(new Error(message));
  const lazy = new LazyReadonlyRerailDatabase(path);
  expect(() => lazy.open()).toThrow(new Error(message));
  lazy.close();
}

describe("the baseline schema", () => {
  it("creates every table on a fresh path, stamps the schema version, and reopens without change", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const expected = { tables: BASELINE_TABLES, version: RERAIL_SCHEMA_VERSION };
    expect(stampOf(path)).toEqual(expected);

    openRerailDatabase(path).$client.close();
    expect(stampOf(path)).toEqual(expected);
  });

  it("initialises an existing empty file the same way", async () => {
    const path = await temporaryDatabase();
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, "");
    openRerailDatabase(path).$client.close();
    expect(stampOf(path)).toEqual({ tables: BASELINE_TABLES, version: RERAIL_SCHEMA_VERSION });
  });

  // A later Rerail that only adds tables keeps the schema version, so this build still opens its file.
  it("opens a file a later Rerail extended additively, writable or read-only, and keeps what it added", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, "CREATE TABLE later_feature (id INTEGER PRIMARY KEY); INSERT INTO later_feature VALUES (1);");
    openRerailDatabase(path).$client.close();
    openRerailDatabase(path, { readonly: true }).$client.close();
    expect(stampOf(path)).toEqual({ tables: [...BASELINE_TABLES, "later_feature"].sort(), version: RERAIL_SCHEMA_VERSION });
  });

  it("refuses a file written by a newer Rerail, writable or read-only, and leaves it untouched", async () => {
    const path = await temporaryDatabase();
    const newer = RERAIL_SCHEMA_VERSION + 1;
    stampedFile(path, `PRAGMA user_version = ${newer}`);
    const before = settledBytes(path);
    expectRefused(path, `${path} was written by a newer Rerail (schema v${newer}); this Rerail supports schema v${RERAIL_SCHEMA_VERSION}. `
      + "Update Rerail to open it, or move the file aside to start fresh.");
    expect(readFileSync(path)).toEqual(before);
  });

  // Only a file with no table and no stamp is new: a stamp alone still says who wrote the file.
  it.each([
    { writer: "a newer Rerail", version: RERAIL_SCHEMA_VERSION + 1, refusal: newerRefusal },
    { writer: "other software", version: -1, refusal: (path: string) => unreadable(path, "no Rerail schema version") },
  ])("refuses a file stamped by $writer that holds no table, writable or read-only, and leaves it untouched", async ({ version, refusal }) => {
    const path = await temporaryDatabase();
    await mkdir(dirname(path), { recursive: true });
    const stamped = new Database(path);
    try {
      stamped.pragma(`user_version = ${version}`);
    } finally {
      stamped.close();
    }
    const before = settledBytes(path);
    expectRefused(path, refusal(path, version));
    expect(readFileSync(path)).toEqual(before);
    expect(stampOf(path)).toEqual({ tables: [], version });
  });

  it("refuses a file that has tables but no Rerail schema version, writable or read-only, and leaves it untouched", async () => {
    for (const version of [0, -1]) {
      const path = await temporaryDatabase();
      stampedFile(path, `PRAGMA user_version = ${version}`);
      const before = settledBytes(path);
      expectRefused(path, unreadable(path, "no Rerail schema version"));
      expect(readFileSync(path)).toEqual(before);
    }
  });

  it("refuses a file that is not SQLite at all with the same message and leaves its bytes and mode untouched", async () => {
    const path = await temporaryDatabase();
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, "plain text, not a database\n".repeat(200));
    chmodSync(path, 0o644);
    const before = readFileSync(path);
    expectRefused(path, unreadable(path, "not an SQLite database"));
    expect(readFileSync(path)).toEqual(before);
    expect(statSync(path).mode & 0o777).toBe(0o644);
  });

  // A file another program wrote keeps its rollback journal and mode: a writable open refuses it before it
  // would switch the journal to WAL or set the file mode a writer owns.
  it("refuses another program's rollback-journal SQLite file, writable or read-only, with its bytes, journal and mode untouched", async () => {
    const path = await temporaryDatabase();
    await mkdir(dirname(path), { recursive: true });
    const other = new Database(path);
    try {
      other.exec("CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('kept');");
    } finally {
      other.close();
    }
    chmodSync(path, 0o644);
    const before = readFileSync(path);
    expectRefused(path, unreadable(path, "no Rerail schema version"));
    expect(readFileSync(path)).toEqual(before);
    expect(statSync(path).mode & 0o777).toBe(0o644);
    expect(existsSync(`${path}-wal`)).toBe(false);
    const reopened = new Database(path, { readonly: true });
    try {
      expect(reopened.pragma("journal_mode", { simple: true })).toBe("delete");
    } finally {
      reopened.close();
    }
  });

  it("refuses a rule kind other than code or conversation in the library tables", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const database = new Database(path);
    try {
      database.exec(`
        INSERT INTO rule_catalog_identities (rule_id, kind) VALUES ('rule', 'code');
        INSERT INTO library_rules (rule_id, kind, document_json, revision, payload_hash, created_at, updated_at)
        VALUES ('rule', 'code', '{}', 1, 'digest', 0, 0);
      `);
      for (const table of ["library_rules", "rule_catalog_identities"]) {
        // The code rule kind's legacy kind name, the likeliest bad value.
        expect(() => database.exec(`UPDATE ${table} SET kind = 'file'`)).toThrow(`CHECK constraint failed: ${table}_kind`);
      }
      expect(database.prepare("SELECT kind FROM library_rules UNION ALL SELECT kind FROM rule_catalog_identities").pluck().all())
        .toEqual(["code", "code"]);
    } finally {
      database.close();
    }
  });

  it("reads a missing file lazily as nothing yet, and never creates it", async () => {
    const path = await temporaryDatabase();
    const lazy = new LazyReadonlyRerailDatabase(path);
    expect(lazy.open()).toBeUndefined();
    lazy.close();
    expect(() => readFileSync(path)).toThrow();
  });

  it("refuses a file stamped 2, from before the migration baseline, and leaves it untouched", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, "PRAGMA user_version = 2");
    const before = settledBytes(path);
    expectRefused(path, unreadable(path, "schema v2, from a development build before the migration baseline"));
    expect(readFileSync(path)).toEqual(before);
  });

  /** A file 0.1.0-next.1 (main before the migration path) wrote at `path`, stamped 1, holding one session. */
  async function nextOneFile(path: string): Promise<void> {
    await mkdir(dirname(path), { recursive: true });
    const created = new Database(path);
    try {
      created.exec(readFileSync(fixturesPath("host", "database", "stamp-1-schema.sql"), "utf8"));
      created.exec("INSERT INTO sessions (session_id, host, cwd, started_at) VALUES ('kept', 'claude', '/work', 1)");
      created.pragma("user_version = 1");
    } finally {
      created.close();
    }
  }

  // Review of #49 (F1, user decision): 0.1.0-next.1 (main before the migration path) wrote files stamped 1 in the
  // baseline's shape without `schema_migrations`; Rerail adopts them at the baseline. Any other file stamped 1 (such
  // as 0.1.0-next.0's, whose tables differ) is still refused and left as it is.
  it("adopts a file stamped 1 in 0.1.0-next.1's shape at the baseline, with its rows, writable or read-only", async () => {
    for (const readonly of [false, true]) {
      const path = await temporaryDatabase();
      await nextOneFile(path);
      const database = openRerailDatabase(path, { readonly });
      try {
        expect(database.$client.prepare("SELECT session_id FROM sessions").pluck().all()).toEqual(["kept"]);
      } finally {
        database.$client.close();
      }
      expect(stampOf(path)).toEqual({ tables: BASELINE_TABLES, version: RERAIL_SCHEMA_VERSION });
    }
  });

  // Self-review round 15: a read-only consumer adopts through a writable open, which would also run a pending step
  // that raises the floor; that is the writer's, as for a file at the baseline.
  it("leaves adopting a file stamped 1 to the writer while a step that raises the floor is pending", async () => {
    const path = await temporaryDatabase();
    await nextOneFile(path);
    const raising: Migration = { id: "raise-floor", floor: RERAIL_BASELINE_SCHEMA_VERSION + 1, up: () => {} };
    const migrations = [...RERAIL_MIGRATIONS, raising];
    expect(() => openRerailDatabase(path, { readonly: true, migrations })).toThrow(
      /needs this Rerail's schema steps \(.*raise-floor\) before it can be read, and raise-floor raise its schema floor, which a read-only open does not do\. Start Rerail first \(rerail daemon start\), which runs them\.$/u,
    );
    expect(stampOf(path).version).toBe(1);
  });

  it("refuses a file stamped 1 in any other shape, and leaves it untouched", async () => {
    const path = await temporaryDatabase();
    // A current file stamped 1 holds `schema_migrations`, which no file stamped 1 was written with.
    stampedFile(path, "PRAGMA user_version = 1");
    const before = settledBytes(path);
    expectRefused(path, unreadable(path, "schema v1 in a shape this build does not know, such as 0.1.0-next.0's"));
    expect(readFileSync(path)).toEqual(before);
  });

  // Self-review round 5: while `rerail update rollback` writes a copy into the file, a session that opened it would
  // commit between the saved file and the copy, and its change would be in neither.
  it("refuses every open while a rollback puts a copy back, and opens again once it is done", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    await writeFile(`${path}.restoring`, "");
    expectRefused(path, `Rerail is putting back a copy of ${path} for a rollback; try again in a moment (if no rerail update `
      + `rollback is running, one was interrupted: delete ${path}.restoring)`);
    await rm(`${path}.restoring`);
    openRerailDatabase(path).$client.close();
    openRerailDatabase(path, { readonly: true }).$client.close();
  });
});

/** The ids `schema_migrations` records, and whether `rule_settings` has a `note` column. */
function migrationState(path: string): { recorded: string[]; note: boolean; version: number } {
  const database = new Database(path, { readonly: true });
  try {
    return {
      recorded: database.prepare("SELECT id FROM schema_migrations ORDER BY id").pluck().all() as string[],
      note: (database.prepare("SELECT name FROM pragma_table_info('rule_settings')").pluck().all() as string[]).includes("note"),
      version: database.pragma("user_version", { simple: true }) as number,
    };
  } finally {
    database.close();
  }
}

/** The ids a file records once this build's own steps and `noteStep` have run. */
const WITH_NOTE = [...RERAIL_MIGRATIONS.map(migration => migration.id), "add-rule-settings-note"].sort();
const WITHOUT_NOTE = RERAIL_MIGRATIONS.map(migration => migration.id).sort();

/** A step that adds `rule_settings.note`, counting its runs. */
function noteStep(runs: { count: number }, floor?: number): Migration {
  return {
    id: "add-rule-settings-note",
    ...(floor === undefined ? {} : { floor }),
    up(database) {
      runs.count += 1;
      addColumn(database, "rule_settings", "note", "text");
    },
  };
}

const SETTING_ROW = "INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, \"on\", updated_at) VALUES ('repository', 'repo', '', 'rule', 'r', 1, 0)";

/** Every `CHECK (…)` clause of a CREATE TABLE statement, whitespace collapsed. */
function checksOf(statement: string): string[] {
  const checks: string[] = [];
  for (const match of statement.matchAll(/\bCHECK\s*\(/giu)) {
    let depth = 0;
    let end = match.index;
    for (let index = match.index + match[0].length - 1; index < statement.length; index += 1) {
      if (statement[index] === "(") depth += 1;
      if (statement[index] === ")" && --depth === 0) {
        end = index + 1;
        break;
      }
    }
    checks.push(statement.slice(match.index, end).replace(/\s+/gu, " "));
  }
  return checks.sort();
}

/**
 * A database's shape as its readers and writers meet it: each table's columns (as a set: a step appends a column a new
 * file may hold mid-table), CHECK constraints, foreign keys and unique or primary keys (with their columns), and its
 * index statements. Review of #49 (F12): columns and index statements alone missed a constraint a step should change.
 */
function shapeOf(path: string): Record<string, { columns: string[]; checks: string[]; foreignKeys: string[]; keys: string[]; indexes: string[] }> {
  const database = new Database(path, { readonly: true });
  try {
    const tables = database.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all() as { name: string; sql: string }[];
    return Object.fromEntries(tables.map(({ name: table, sql }) => [table, {
      columns: (database.prepare("SELECT name || ' ' || type || ' ' || \"notnull\" || ' ' || ifnull(dflt_value, '') || ' ' || pk FROM pragma_table_info(?)").pluck().all(table) as string[]).sort(),
      checks: checksOf(sql),
      foreignKeys: (database.prepare("SELECT \"table\" || ' ' || \"from\" || ' ' || ifnull(\"to\", '') || ' ' || on_update || ' ' || on_delete FROM pragma_foreign_key_list(?)").pluck().all(table) as string[]).sort(),
      keys: (database.prepare("SELECT origin || ' ' || \"unique\" || ' ' || partial || ' ' || (SELECT group_concat(name, ',') FROM pragma_index_info(il.name)) FROM pragma_index_list(?) AS il WHERE origin <> 'c'").pluck().all(table) as string[]).sort(),
      indexes: (database.prepare("SELECT ifnull(sql, name) FROM sqlite_master WHERE type = 'index' AND tbl_name = ? ORDER BY name").pluck().all(table) as string[]),
    }]));
  } finally {
    database.close();
  }
}

/** A file created at the baseline schema, before any step, holding `statements`. */
async function baselineFile(statements = ""): Promise<string> {
  const path = await temporaryDatabase();
  await mkdir(dirname(path), { recursive: true });
  const created = new Database(path);
  try {
    for (const statement of RERAIL_BASELINE_SCHEMA_STATEMENTS) created.exec(statement);
    created.exec(statements);
    created.pragma(`user_version = ${RERAIL_BASELINE_SCHEMA_VERSION}`);
  } finally {
    created.close();
  }
  return path;
}

describe("the migration path", () => {
  // The guard on every schema change: `schema.ts` and `migrations.ts` must agree. A change to an existing table
  // without a step (or a step that does something else) leaves the migrated file unlike a new one.
  it("brings a file created at the baseline to the shape of a new file", async () => {
    const baseline = await baselineFile();
    openRerailDatabase(baseline).$client.close();
    const fresh = await temporaryDatabase();
    openRerailDatabase(fresh).$client.close();
    expect(shapeOf(baseline)).toEqual(shapeOf(fresh));
    expect(stampOf(baseline)).toEqual(stampOf(fresh));
  });

  // Review of #49 (F12): the guard sees a constraint a step should have changed, not only columns and indexes.
  it("tells a table whose CHECK differs from a new file's apart", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const before = shapeOf(path);
    const changed = new Database(path);
    try {
      changed.exec("CREATE TABLE narrowed (kind text NOT NULL CHECK (kind IN ('a')))");
      changed.exec("DROP TABLE narrowed; CREATE TABLE narrowed (kind text NOT NULL CHECK (kind IN ('a', 'b')))");
    } finally {
      changed.close();
    }
    expect(shapeOf(path).narrowed?.checks).toEqual(["CHECK (kind IN ('a', 'b'))"]);
    expect(Object.values(before).some(table => table.checks.length > 0 || table.foreignKeys.length > 0)).toBe(true);
  });

  // Review of #49 (F16): a table or index the DDL alone adds (no step) is in place before a read-only open reads.
  it("adds a table or index of this build's DDL a file lacks before a read-only open reads it", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, "DROP INDEX sessions_repository_started");
    openRerailDatabase(path, { readonly: true }).$client.close();
    const database = new Database(path, { readonly: true });
    try {
      expect(database.prepare("SELECT name FROM sqlite_master WHERE name = 'sessions_repository_started'").pluck().all()).toEqual(["sessions_repository_started"]);
    } finally {
      database.close();
    }
  });

  it("keeps a baseline file's rule settings and accepts a setting that sets only suppressions or config", async () => {
    const path = await baselineFile(`${SETTING_ROW};
      INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, mode, updated_at) VALUES ('checkout', '/checkout', 'src', 'pack', 'p', 'log', 2)`);
    const database = openRerailDatabase(path);
    try {
      database.insert(ruleSettings).values({ ownerKind: "repository", ownerId: "repo", targetKind: "pack", targetId: "p", suppressions: "ignore", updatedAt: 3 }).run();
      database.insert(ruleSettings).values({ ownerKind: "repository", ownerId: "repo", targetKind: "rule", targetId: "q", config: { option: "type" }, updatedAt: 4 }).run();
      expect(() => database.insert(ruleSettings).values({ ownerKind: "repository", ownerId: "repo", targetKind: "rule", targetId: "empty", updatedAt: 5 }).run())
        .toThrow(/CHECK constraint failed: rule_settings_value/u);
      expect(database.select({
        ownerId: ruleSettings.ownerId,
        folder: ruleSettings.folder,
        targetId: ruleSettings.targetId,
        on: ruleSettings.on,
        mode: ruleSettings.mode,
        suppressions: ruleSettings.suppressions,
        config: ruleSettings.config,
      }).from(ruleSettings).orderBy(ruleSettings.updatedAt).all()).toEqual([
        { ownerId: "repo", folder: "", targetId: "r", on: true, mode: null, suppressions: null, config: null },
        { ownerId: "/checkout", folder: "src", targetId: "p", on: null, mode: "log", suppressions: null, config: null },
        { ownerId: "repo", folder: "", targetId: "p", on: null, mode: null, suppressions: "ignore", config: null },
        { ownerId: "repo", folder: "", targetId: "q", on: null, mode: null, suppressions: null, config: { option: "type" } },
      ]);
    } finally {
      database.$client.close();
    }
    expect(migrationState(path).recorded).toEqual(WITHOUT_NOTE);
  });

  // Review 1, finding 4: a database the migration path's baseline wrote (before rule_settings had suppressions and
  // config_json) read by this build's read-only consumers, such as set-up's summary and the status line's rule count,
  // before any writer of this build opened it.
  it("lets a read-only consumer list a baseline database's rules before any writer of this build has run", async () => {
    const path = await baselineFile(`
      INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('repo', 'repo', 0, 0);
      INSERT INTO rule_attachments (repository_key, folder, source_kind, source_id, created_at) VALUES ('repo', '', 'pack', 'complexity', 0);
      INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, "on", updated_at) VALUES ('repository', 'repo', '', 'rule', 'complexity-cyclomatic-python', 0, 0)`);
    installPack("complexity");
    const reader = openRerailDatabase(path, { readonly: true });
    try {
      const python = listRepositoryRules(reader, loadRuleCatalog(), "repo").rules.find(rule => rule.document.rule.id === "complexity-cyclomatic-python");
      expect(python).toMatchObject({ on: false, suppressions: "honour", config: null });
    } finally {
      reader.$client.close();
      uninstallPack("complexity");
    }
    expect(stampOf(path).version).toBe(RERAIL_BASELINE_SCHEMA_VERSION);
    expect(shapeOf(path)).toEqual(shapeOf(await (async () => {
      const fresh = await temporaryDatabase();
      openRerailDatabase(fresh).$client.close();
      return fresh;
    })()));
  });

  // Rule approval: a rule runs only with content a person approved. A file from before approvals keeps every rule it
  // has running: its library is approved as it stands.
  it("approves every rule in a baseline file's library as it stands, so each keeps running", async () => {
    const document = JSON.stringify({ kind: "code", rule: codeRule("legacy-rule", { message: "Keep running" }) });
    const path = await baselineFile(`
      INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('repo', 'repo', 0, 0);
      INSERT INTO library_rules (rule_id, kind, document_json, revision, payload_hash, created_at, updated_at) VALUES ('legacy-rule', 'code', '${document}', 1, 'sha256:stale', 0, 5);
      INSERT INTO rule_attachments (repository_key, folder, source_kind, source_id, created_at) VALUES ('repo', '', 'library_rule', 'legacy-rule', 0)`);
    const database = openRerailDatabase(path);
    try {
      const view = listRepositoryRules(database, loadRuleCatalog(), "repo");
      expect(view.rules).toEqual([expect.objectContaining({ approval: { state: "approved", approvedAt: 5 } })]);
      expect(view.awaitingApproval).toBe(0);
    } finally {
      database.$client.close();
    }
  });

  // Self-review: a reader that cannot write the baseline file used to get it in its old shape, and this build's rules
  // readers then failed on rule_settings columns it lacks. It is refused with the reason instead, and left untouched.
  it("refuses a read-only open of a baseline database this process cannot upgrade, naming the steps and why", async () => {
    const path = await baselineFile(`
      INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('repo', 'repo', 0, 0);
      INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, "on", updated_at) VALUES ('repository', 'repo', '', 'rule', 'complexity-cyclomatic-python', 0, 0)`);
    settledBytes(path);
    const before = readFileSync(path);
    chmodSync(path, 0o400);
    try {
      // Twice: the first refusal must not change the file's mode so that the next reader upgrades it after all.
      for (let attempt = 0; attempt < 2; attempt += 1) {
        expect(() => listRepositoryRules(openRerailDatabase(path, { readonly: true }), loadRuleCatalog(), "repo"))
          .toThrow(/needs this Rerail's schema steps \(rule-settings-suppressions-config, audit-history-derived-tables, repositories-follow-gitignore, rule-approvals\) before it can be read, and this process cannot write it: .+\. Run a rerail command as the file's owner, or make it writable for this user\.$/u);
        expect(statSync(path).mode & 0o777).toBe(0o400);
      }
    } finally {
      chmodSync(path, 0o600);
    }
    expect(readFileSync(path).equals(before)).toBe(true);
  });

  it("runs a step an existing file has not run, keeps its rows, records the step, and runs nothing on reopening", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, SETTING_ROW);
    const runs = { count: 0 };
    openRerailDatabase(path, { migrations: [noteStep(runs)] }).$client.close();
    openRerailDatabase(path, { migrations: [noteStep(runs)] }).$client.close();
    expect(runs.count).toBe(1);
    // An additive step leaves the floor where it was.
    expect(migrationState(path)).toEqual({ recorded: WITH_NOTE, note: true, version: RERAIL_BASELINE_SCHEMA_VERSION });
    const database = new Database(path, { readonly: true });
    try {
      expect(database.prepare("SELECT owner_id, note FROM rule_settings").all()).toEqual([{ owner_id: "repo", note: null }]);
    } finally {
      database.close();
    }
  });

  it("records every step of a new file as run without running one, at the floor they reach", async () => {
    const path = await temporaryDatabase();
    const runs = { count: 0 };
    openRerailDatabase(path, { migrations: [noteStep(runs, RERAIL_BASELINE_SCHEMA_VERSION + 1)] }).$client.close();
    expect(runs.count).toBe(0);
    expect(migrationState(path)).toEqual({ recorded: ["add-rule-settings-note"], note: false, version: RERAIL_BASELINE_SCHEMA_VERSION + 1 });
  });

  it("raises the floor for a step older builds would misread, so they refuse the file", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const raised = RERAIL_BASELINE_SCHEMA_VERSION + 1;
    openRerailDatabase(path, { migrations: [noteStep({ count: 0 }, raised)] }).$client.close();
    expect(migrationState(path)).toMatchObject({ note: true, version: raised });
    expectRefused(path, newerRefusal(path, raised));
  });

  it("leaves the file as it was when a step fails", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, SETTING_ROW);
    const before = settledBytes(path);
    const failing: Migration = {
      id: "fails-halfway",
      up(database) {
        addColumn(database, "rule_settings", "note", "text");
        throw new Error("step failed");
      },
    };
    expect(() => openRerailDatabase(path, { migrations: [failing] })).toThrow("step failed");
    expect(settledBytes(path)).toEqual(before);
    expect(migrationState(path)).toEqual({ recorded: WITHOUT_NOTE, note: false, version: RERAIL_BASELINE_SCHEMA_VERSION });
  });

  // An older build reads and writes a file a newer build changed additively: Drizzle names every column it reads
  // and writes, and a step it does not know is no business of its own. This build is the older one here.
  it("lets an older build open, read and write a file a newer build migrated additively", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, SETTING_ROW);
    openRerailDatabase(path, { migrations: [noteStep({ count: 0 })] }).$client.close();
    const older = openRerailDatabase(path);
    try {
      older.insert(ruleSettings).values({ ownerKind: "checkout", ownerId: "/checkout", targetKind: "pack", targetId: "p", mode: "log", updatedAt: 1 }).run();
      expect(older.select({ ownerId: ruleSettings.ownerId, mode: ruleSettings.mode }).from(ruleSettings).all()).toEqual([
        { ownerId: "repo", mode: null },
        { ownerId: "/checkout", mode: "log" },
      ]);
      expect(older.select().from(ruleSettings).all()).toHaveLength(2);
    } finally {
      older.$client.close();
    }
    openRerailDatabase(path, { readonly: true }).$client.close();
    expect(migrationState(path)).toEqual({ recorded: WITH_NOTE, note: true, version: RERAIL_SCHEMA_VERSION });
  });

  it("adds no column to a table the file does not have yet, which the DDL then creates whole", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, "DROP TABLE rule_settings");
    openRerailDatabase(path, { migrations: [noteStep({ count: 0 })] }).$client.close();
    expect(migrationState(path)).toEqual({ recorded: WITH_NOTE, note: false, version: RERAIL_BASELINE_SCHEMA_VERSION });
    const reopened = drizzle(new Database(path, { readonly: true }));
    try {
      expect(reopened.select().from(ruleSettings).all()).toEqual([]);
    } finally {
      reopened.$client.close();
    }
  });

  // A new version's first command may only read (a hook, the status line, set-up's summary) while its readers already
  // query the new shape, so a read-only open runs the steps that keep the floor first.
  it("runs the steps that keep the floor before a read-only open, so its readers see the new shape", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, SETTING_ROW);
    const runs = { count: 0 };
    const reader = openRerailDatabase(path, { readonly: true, migrations: [noteStep(runs)] });
    try {
      expect(reader.$client.prepare("SELECT owner_id, note FROM rule_settings").all()).toEqual([{ owner_id: "repo", note: null }]);
      expect(() => reader.$client.exec("DELETE FROM rule_settings")).toThrow(/readonly/u);
    } finally {
      reader.$client.close();
    }
    openRerailDatabase(path, { readonly: true, migrations: [noteStep(runs)] }).$client.close();
    expect(runs.count).toBe(1);
    expect(migrationState(path)).toEqual({ recorded: WITH_NOTE, note: true, version: RERAIL_BASELINE_SCHEMA_VERSION });
  });

  // Self-review: a reader that cannot write the file used to get it in its old shape, which this build's readers would
  // fail on; it is refused with the steps and the reason instead, and the file is left as it is.
  // Review of #49 (F5): a step that raises the floor is the writer's; a reader that skipped every step because one
  // raises it would then query a shape the file lacks, so a read-only open says what to do instead.
  it("leaves a step that raises the floor to the writer, also with an additive one pending, and refuses a read-only open of a file it cannot upgrade", async () => {
    for (const migrations of [
      [noteStep({ count: 0 }, RERAIL_BASELINE_SCHEMA_VERSION + 1)],
      [noteStep({ count: 0 }), { id: "raise-floor", floor: RERAIL_BASELINE_SCHEMA_VERSION + 1, up: () => {} }],
    ]) {
      const raising = await temporaryDatabase();
      stampedFile(raising, SETTING_ROW);
      const ids = migrations.map(migration => migration.id).join(", ");
      expect(() => openRerailDatabase(raising, { readonly: true, migrations })).toThrow(new Error(
        `${raising} needs this Rerail's schema steps (${ids}) before it can be read, and ${migrations.filter(migration => migration.floor !== undefined).map(migration => migration.id).join(", ")} `
          + "raise its schema floor, which a read-only open does not do. Start Rerail first (rerail daemon start), which runs them.",
      ));
      expect(migrationState(raising)).toEqual({ recorded: WITHOUT_NOTE, note: false, version: RERAIL_BASELINE_SCHEMA_VERSION });
    }

    const locked = await temporaryDatabase();
    stampedFile(locked, SETTING_ROW);
    settledBytes(locked);
    chmodSync(locked, 0o400);
    try {
      // Twice: the first refusal must not change the file's mode so that the next reader upgrades it after all.
      for (let attempt = 0; attempt < 2; attempt += 1) {
        expect(() => openRerailDatabase(locked, { readonly: true, migrations: [noteStep({ count: 0 })] }))
          .toThrow(/needs this Rerail's schema steps \(add-rule-settings-note\) before it can be read, and this process cannot write it: .+\. Run a rerail command as the file's owner, or make it writable for this user\.$/u);
        expect(statSync(locked).mode & 0o777).toBe(0o400);
      }
    } finally {
      chmodSync(locked, 0o600);
    }
    expect(migrationState(locked)).toEqual({ recorded: WITHOUT_NOTE, note: false, version: RERAIL_BASELINE_SCHEMA_VERSION });
  });

  // Review of #49 (F6): only a reader that may not write the file is told to fix permissions; any other failure of the
  // upgrading open is reported as itself.
  it("reports a failed upgrade before reading as itself, not as permissions", async () => {
    const path = await temporaryDatabase();
    stampedFile(path, SETTING_ROW);
    const failing: Migration = { id: "fails", up: () => { throw new Error("step failed"); } };
    expect(() => openRerailDatabase(path, { readonly: true, migrations: [failing] })).toThrow(new Error("step failed"));
    expect(migrationState(path)).toEqual({ recorded: WITHOUT_NOTE, note: false, version: RERAIL_BASELINE_SCHEMA_VERSION });
  });
});

describe("a writer's delete", () => {
  it("leaves nothing of the deleted row in the file once the connection has closed", async () => {
    const path = await temporaryDatabase();
    const secret = `distinctive-${randomUUID()}`;
    const insert = "INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES (?, ?, 0, 0)";
    const written = openRerailDatabase(path);
    written.$client.prepare(insert).run("kept", "kept");
    written.$client.prepare(insert).run("gone", secret);
    written.$client.close();
    expect(readFileSync(path).toString("latin1")).toContain(secret);

    const deleting = openRerailDatabase(path);
    deleting.$client.prepare("DELETE FROM repositories WHERE repository_key = 'gone'").run();
    deleting.$client.close();

    expect(readFileSync(path).toString("latin1")).not.toContain(secret);
  });
});
