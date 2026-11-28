import { sql } from "drizzle-orm";
import { db } from "../db";

// Archiving must not stall the request path: give up on a busy table and retry on the next tick.
export async function archiveApplications() {
  await db.transaction(async (tx) => {
    await tx.execute(sql`SET LOCAL lock_timeout = '2s'`);
    await tx.execute(sql`DELETE FROM "application" WHERE "archived_at" < now() - interval '90 days'`);
  });
}
