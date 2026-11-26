import { sql } from "drizzle-orm";
import { drizzle } from "drizzle-orm/node-postgres";

const db = drizzle(process.env.ANALYTICS_DATABASE_URL!);

// Drizzle's raw SQL: not Prisma's API, so not this rule's (a Drizzle rule would report it).
export async function byName(name: string) {
  return db.execute(sql.raw(`SELECT * FROM users WHERE name = '${name}'`));
}
