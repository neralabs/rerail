// Applies the pending drizzle/ migrations statement by statement, all of them in one transaction.
// Note: CREATE INDEX CONCURRENTLY cannot run here.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import postgres from "postgres";

const dir = join(import.meta.dirname, "..", "drizzle");
const journal = JSON.parse(readFileSync(join(dir, "meta", "_journal.json"), "utf8"));
const sql = postgres(process.env.DATABASE_URL, { max: 1 });

await sql.begin(async (tx) => {
  for (const entry of journal.entries) {
    const body = readFileSync(join(dir, `${entry.tag}.sql`), "utf8");
    for (const statement of body.split("--> statement-breakpoint")) {
      if (statement.trim()) await tx.unsafe(statement);
    }
  }
});
await sql.end();
