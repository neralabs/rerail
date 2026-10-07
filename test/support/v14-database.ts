import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import Database from "better-sqlite3";
import { RERAIL_V14_SCHEMA_STATEMENTS } from "../../src/schema-v14-sql.js";

/** The `CREATE TABLE` statement schema v14 used for `table`. */
export function v14TableStatement(table: string): string {
  const statement = RERAIL_V14_SCHEMA_STATEMENTS.find(candidate => candidate.startsWith(`CREATE TABLE IF NOT EXISTS \`${table}\` (`));
  if (statement === undefined) throw new Error(`schema v14 has no ${table} table`);
  return statement;
}

/** A database exactly as a v14 Rerail leaves it: the v14 DDL, schema revision 14 and compatibility floor 14. */
export function createV14Database(path: string): Database.Database {
  mkdirSync(dirname(path), { recursive: true });
  const database = new Database(path);
  for (const statement of RERAIL_V14_SCHEMA_STATEMENTS) database.exec(statement);
  database.exec(
    "CREATE TABLE `rerail_schema_revision` (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), revision INTEGER NOT NULL);"
    + "INSERT INTO `rerail_schema_revision` (singleton, revision) VALUES (1, 14);"
    + "PRAGMA user_version = 14;",
  );
  return database;
}
