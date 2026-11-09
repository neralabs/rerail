/**
 * What tables and columns an SQLite file holds, asked before reading it. A read-only open applies no schema, so a file an
 * earlier build wrote can lack a table, and one whose creator has not committed its schema yet has none. Takes the driver's
 * own connection: Rerail's database (`database.$client`) and the coding-agent stores on `node:sqlite` both answer it.
 */
interface SqliteConnection {
  prepare(statement: string): {
    get(...parameters: string[]): unknown;
    all(...parameters: string[]): unknown[];
  };
}

/** Whether the file holds a table named `table`. */
export function holdsTable(connection: SqliteConnection, table: string): boolean {
  return connection.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?").get(table) !== undefined;
}

/** The names of the columns of `table`, a name the caller owns rather than input; none when the file lacks the table. */
export function tableColumns(connection: SqliteConnection, table: string): string[] {
  return (connection.prepare(`PRAGMA table_info(${table})`).all() as Array<{ name: string }>).map(column => column.name);
}
