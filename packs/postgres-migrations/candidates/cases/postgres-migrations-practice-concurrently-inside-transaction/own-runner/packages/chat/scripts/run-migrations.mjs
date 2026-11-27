// Applies the drizzle/ migrations one at a time. A migration that holds a CONCURRENTLY statement runs statement by
// statement with no transaction around it; any other runs in a transaction of its own.
import { join } from "node:path";
import { sql } from "drizzle-orm";
import { drizzle } from "drizzle-orm/node-postgres";
import { readMigrationFiles } from "drizzle-orm/migrator";
import { Pool } from "pg";

const migrationsFolder = join(import.meta.dirname, "..", "drizzle");
const isNonTransactional = (statement) => /\bCONCURRENTLY\b/i.test(statement);

const db = drizzle({ client: new Pool({ connectionString: process.env.DATABASE_URL, max: 1 }) });
const applied = new Set((await db.execute(sql`select hash from drizzle.__drizzle_migrations`)).rows.map((r) => r.hash));

for (const migration of readMigrationFiles({ migrationsFolder })) {
  if (applied.has(migration.hash)) continue;
  const statements = migration.sql.filter((s) => s.trim());
  if (statements.some(isNonTransactional)) {
    for (const s of statements) await db.execute(sql.raw(s));
  } else {
    await db.transaction(async (tx) => {
      for (const s of statements) await tx.execute(sql.raw(s));
    });
  }
}
