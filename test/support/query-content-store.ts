import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";

const sqlite = createRequire(import.meta.url).resolve("better-sqlite3");
const read = `
  const [sqlite, path, sql, parameters] = process.argv.slice(1);
  const Database = require(sqlite);
  const store = new Database(path, { readonly: true, fileMustExist: true });
  try {
    process.stdout.write(JSON.stringify(store.prepare(sql).all(...JSON.parse(parameters))));
  } finally {
    store.close();
  }
`;

/** Keep the test's SQLite binding outside the process that runs the native store and its collector. */
export function queryContentStore<Row = Record<string, unknown>>(
  path: string, sql: string, parameters: readonly (string | number | null)[] = [],
): Row[] {
  return JSON.parse(execFileSync(process.execPath, [
    "--input-type=commonjs", "-e", read, "--", sqlite, path, sql, JSON.stringify(parameters),
  ], { encoding: "utf8" })) as Row[];
}
