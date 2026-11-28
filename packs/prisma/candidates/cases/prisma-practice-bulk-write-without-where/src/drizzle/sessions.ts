import { db } from "../db/drizzle";

// The same call shape on a Drizzle `db`: not Prisma's deleteMany.
export async function clearOnDrizzle() {
  await db.session.deleteMany();
}
