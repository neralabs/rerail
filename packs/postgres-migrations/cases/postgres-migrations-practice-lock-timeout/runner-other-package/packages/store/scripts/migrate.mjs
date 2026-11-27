// Applies a migration set (--set=engine|product) in journal order, each migration in its own transaction.
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import postgres from "postgres";

const HERE = dirname(fileURLToPath(import.meta.url));
const SETS = {
  engine: join(HERE, "..", "drizzle"),
  product: join(HERE, "..", "..", "..", "apps", "web", "drizzle"),
};
const LOCK_TIMEOUT = process.env.MIGRATE_LOCK_TIMEOUT ?? "10s";
const set = process.argv.find((a) => a.startsWith("--set="))?.split("=")[1] ?? "engine";
const dir = SETS[set];
const journal = JSON.parse(await readFile(join(dir, "meta", "_journal.json"), "utf8"));
const sql = postgres(process.env.DATABASE_URL, { max: 1 });

for (const entry of journal.entries) {
  const body = await readFile(join(dir, `${entry.tag}.sql`), "utf8");
  await sql.begin(async (tx) => {
    await tx.unsafe(`SET LOCAL lock_timeout = '${LOCK_TIMEOUT}'`);
    for (const statement of body.split("--> statement-breakpoint")) {
      if (statement.trim()) await tx.unsafe(statement);
    }
  });
}
await sql.end();
