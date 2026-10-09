import { chmodSync, existsSync, readFileSync, statSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { LazyReadonlyRerailDatabase, openRerailDatabase } from "../src/database.js";
import { RERAIL_SCHEMA_VERSION } from "../src/schema.js";
import { RERAIL_SCHEMA_STATEMENTS } from "../src/schema-sql.js";

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
});
