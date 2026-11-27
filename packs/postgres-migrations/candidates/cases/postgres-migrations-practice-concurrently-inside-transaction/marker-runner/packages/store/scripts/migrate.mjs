// Applies drizzle/'s journal in order. A migration whose first line is the no-transaction marker runs statement by
// statement with no transaction; every other one runs in BEGIN ... COMMIT.
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import postgres from "postgres";

const DIR = join(import.meta.dirname, "..", "drizzle");
const journal = JSON.parse(await readFile(join(DIR, "meta", "_journal.json"), "utf8"));
const sql = postgres(process.env.DATABASE_URL, { max: 1 });
const cx = await sql.reserve();

for (const entry of journal.entries) {
  const body = await readFile(join(DIR, `${entry.tag}.sql`), "utf8");
  const firstLine = body.split("\n", 1)[0] ?? "";
  const noTransaction = /^--\s*acme:no-transaction\b/.test(firstLine);
  const statements = body.split("--> statement-breakpoint").map((s) => s.trim()).filter(Boolean);
  if (!noTransaction) await cx.unsafe("BEGIN");
  for (const statement of statements) await cx.unsafe(statement);
  if (!noTransaction) await cx.unsafe("COMMIT");
}
cx.release();
await sql.end();
