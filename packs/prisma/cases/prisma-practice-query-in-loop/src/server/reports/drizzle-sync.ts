import { eq } from "drizzle-orm";
import { db } from "@/db/drizzle";

// Drizzle's `db` and the `tx` of its `transaction` take the same call shapes as Prisma's, but are not Prisma.
export async function ownersOf(rows: { userId: string }[]) {
  for (const r of rows) {
    await db.query.users.findFirst({ where: (u: any) => eq(u.id, r.userId) });
  }
  await db.transaction(async (tx) => {
    for (const r of rows) {
      await tx.query.users.findMany({ where: (u: any) => eq(u.id, r.userId) });
    }
  });
}
