import { drizzle } from "drizzle-orm/node-postgres";
import { migrate } from "drizzle-orm/node-postgres/migrator";
import { pool } from "../db";
import { resolveWorkspaceFolder } from "../paths";

export async function runDatabaseMigrations() {
  const client = await pool.connect();
  try {
    // A waiting ALTER queues every later query on its table, so give up quickly instead.
    await client.query("SET lock_timeout = '5s'");
    await migrate(drizzle({ client }), { migrationsFolder: resolveWorkspaceFolder("migrations") });
    await client.query("RESET lock_timeout");
  } finally {
    client.release();
  }
}
